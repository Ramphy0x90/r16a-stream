//! Garage access through the S3 API.
//!
//! rusty-s3 only signs, so every call is a presigned URL sent with reqwest. Presigned query
//! authentication works the same on every Garage version we target, while header
//! authentication on 1.0.1 would need `x-amz-content-sha256` among the signed headers
//! (SPEC 10.5).
//!
//! Garage nodes are listed in `R16A_S3_INTERNAL_ENDPOINTS`. A node that refuses connections
//! is skipped for [`DOWN_FOR`], so a dead node costs one fast failure, not one per request.
//! Nothing here assumes exactly two nodes.

pub mod probe;

use std::{
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use reqwest::{Method, StatusCode};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use secrecy::ExposeSecret;
use url::Url;

use crate::config::S3Config;

/// How long an endpoint that refused a connection is skipped.
pub const DOWN_FOR: Duration = Duration::from_secs(30);
/// Lifetime of the presigned URLs used for the API's and the worker's own calls.
const INTERNAL_URL_LIFETIME: Duration = Duration::from_secs(60);

/// The two buckets. Which one a component may write is decided by its Garage key, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BucketName {
    Media,
    Uploads,
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// No node answered, or every node answered with a server error. Retryable. With
    /// replication 2 this is what a write sees while one node is down.
    #[error("storage unavailable, {0}")]
    Unavailable(&'static str),
    /// Garage refused the key for this bucket or action. A configuration error, never retried.
    #[error("storage denied the request, status {0}")]
    Denied(u16),
    #[error("object not found")]
    NotFound,
    #[error("unexpected storage response, status {0}")]
    Unexpected(u16),
    #[error("invalid object key")]
    InvalidKey,
}

impl StorageError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

struct Endpoint {
    media: Bucket,
    uploads: Bucket,
    down_until: Mutex<Option<Instant>>,
}

impl Endpoint {
    fn bucket(&self, name: BucketName) -> &Bucket {
        match name {
            BucketName::Media => &self.media,
            BucketName::Uploads => &self.uploads,
        }
    }

    fn is_up(&self, now: Instant) -> bool {
        // A poisoned lock only means another task panicked mid-update, so the endpoint is tried.
        self.down_until.lock().map_or(true, |g| g.is_none_or(|t| t <= now))
    }
}

pub struct GarageClient {
    endpoints: Vec<Endpoint>,
    credentials: Credentials,
    http: reqwest::Client,
    next: AtomicUsize,
}

impl std::fmt::Debug for GarageClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GarageClient")
            .field("endpoints", &self.endpoints.len())
            .field("credentials", &"[redacted]")
            .finish_non_exhaustive()
    }
}

/// Keys are built by our code from ids, so anything unusual is a bug. Refusing it keeps a
/// malformed key from ever being signed.
pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 512
        && !key.starts_with('/')
        && !key.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..")
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b"/_-.".contains(&b))
}

fn expect_success(status: StatusCode) -> Result<(), StorageError> {
    match status.as_u16() {
        200..=299 => Ok(()),
        401 | 403 => Err(StorageError::Denied(status.as_u16())),
        404 => Err(StorageError::NotFound),
        s => Err(StorageError::Unexpected(s)),
    }
}

impl GarageClient {
    pub fn new(cfg: &S3Config) -> Result<Self, StorageError> {
        let mut endpoints = Vec::with_capacity(cfg.internal_endpoints.len());
        for base in &cfg.internal_endpoints {
            let bucket = |name: &str| {
                Bucket::new(base.clone(), UrlStyle::Path, name.to_owned(), cfg.region.clone())
                    .map_err(|_| StorageError::Unavailable("invalid endpoint URL"))
            };
            endpoints.push(Endpoint {
                media: bucket(&cfg.media_bucket)?,
                uploads: bucket(&cfg.uploads_bucket)?,
                down_until: Mutex::new(None),
            });
        }
        if endpoints.is_empty() {
            return Err(StorageError::Unavailable("no endpoint configured"));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(cfg.connect_timeout)
            .timeout(cfg.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| StorageError::Unavailable("could not build the HTTP client"))?;
        Ok(Self {
            endpoints,
            credentials: Credentials::new(
                cfg.access_key_id.clone(),
                cfg.secret_access_key.expose_secret().to_owned(),
            ),
            http,
            next: AtomicUsize::new(0),
        })
    }

    /// Endpoints in the order to try. Healthy ones first, rotated so load spreads, then the
    /// ones marked down, so a request still goes out when every node was marked down.
    fn order(&self) -> Vec<usize> {
        let n = self.endpoints.len();
        let start = self.next.fetch_add(1, Ordering::Relaxed) % n;
        let now = Instant::now();
        let (mut up, mut down): (Vec<usize>, Vec<usize>) = (0..n)
            .map(|i| (start + i) % n)
            .partition(|&i| self.endpoints[i].is_up(now));
        up.append(&mut down);
        up
    }

    fn mark_down(&self, i: usize, down: bool) {
        if let Ok(mut g) = self.endpoints[i].down_until.lock() {
            *g = down.then(|| Instant::now() + DOWN_FOR);
        }
    }

    /// Sends one action to the first endpoint that answers. Connection failures, timeouts and
    /// server errors move on to the next endpoint. A 4xx answer is final, because every node
    /// gives the same one.
    async fn send(
        &self,
        bucket: BucketName,
        method: &Method,
        sign: impl Fn(&Bucket) -> Url,
        body: Option<&[u8]>,
        headers: &[(&'static str, &str)],
    ) -> Result<reqwest::Response, StorageError> {
        let mut last = "no endpoint answered";
        for i in self.order() {
            let url = sign(self.endpoints[i].bucket(bucket));
            let mut req = self.http.request(method.clone(), url);
            for (name, value) in headers {
                req = req.header(*name, *value);
            }
            if let Some(b) = body {
                req = req.body(b.to_vec());
            }
            match req.send().await {
                Ok(res) if res.status().is_server_error() => {
                    last = "server error";
                }
                Ok(res) => {
                    self.mark_down(i, false);
                    return Ok(res);
                }
                Err(e) => {
                    // reqwest errors can include the URL, which carries a signature. Only the
                    // kind of failure is kept.
                    last = if e.is_connect() {
                        "connection failed"
                    } else if e.is_timeout() {
                        "timed out"
                    } else {
                        "request failed"
                    };
                    if e.is_connect() || e.is_timeout() {
                        self.mark_down(i, true);
                    }
                }
            }
        }
        Err(StorageError::Unavailable(last))
    }

    fn signed<'a>(action: &impl S3Action<'a>) -> Url {
        action.sign_with_time(INTERNAL_URL_LIFETIME, &jiff::Timestamp::now())
    }

    /// Stores a small in-memory object. Large media use streaming helpers added with the
    /// upload pipeline.
    pub async fn put_object(
        &self,
        bucket: BucketName,
        key: &str,
        content: &[u8],
        content_type: &str,
        cache_control: Option<&str>,
    ) -> Result<(), StorageError> {
        if !valid_key(key) {
            return Err(StorageError::InvalidKey);
        }
        let creds = &self.credentials;
        let mut headers = vec![("content-type", content_type)];
        if let Some(cc) = cache_control {
            headers.push(("cache-control", cc));
        }
        let res = self
            .send(
                bucket,
                &Method::PUT,
                |b| Self::signed(&b.put_object(Some(creds), key)),
                Some(content),
                &headers,
            )
            .await?;
        expect_success(res.status())
    }

    /// Reads a small object fully. Returns the body and the stored `Cache-Control`.
    pub async fn get_object(&self, bucket: BucketName, key: &str) -> Result<(Vec<u8>, Option<String>), StorageError> {
        if !valid_key(key) {
            return Err(StorageError::InvalidKey);
        }
        let creds = &self.credentials;
        let res = self
            .send(
                bucket,
                &Method::GET,
                |b| Self::signed(&b.get_object(Some(creds), key)),
                None,
                &[],
            )
            .await?;
        expect_success(res.status())?;
        let cache_control = res
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = res
            .bytes()
            .await
            .map_err(|_| StorageError::Unavailable("body read failed"))?;
        Ok((bytes.to_vec(), cache_control))
    }

    /// Deletes an object. A missing object counts as deleted.
    pub async fn delete_object(&self, bucket: BucketName, key: &str) -> Result<(), StorageError> {
        if !valid_key(key) {
            return Err(StorageError::InvalidKey);
        }
        let creds = &self.credentials;
        let res = self
            .send(
                bucket,
                &Method::DELETE,
                |b| Self::signed(&b.delete_object(Some(creds), key)),
                None,
                &[],
            )
            .await?;
        match res.status() {
            StatusCode::NOT_FOUND => Ok(()),
            s => expect_success(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::valid_key;

    #[test]
    fn object_keys_are_restricted() {
        assert!(valid_key("v/0b6e/attempt/v720/seg_1.m4s"));
        assert!(valid_key("_health/api-local"));
        for bad in [
            "",
            "/abs",
            "a//b",
            "a/../b",
            "a/./b",
            "trailing/",
            "spa ce",
            "quote\"",
            "pct%2F",
            "ü",
        ] {
            assert!(!valid_key(bad), "{bad}");
        }
    }
}
