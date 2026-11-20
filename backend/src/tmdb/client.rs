//! HTTP client for the TMDB API, with the 24 hour cache in `tmdb_cache`.

use std::time::Duration;

use reqwest::{Client, StatusCode, header};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use sqlx::PgPool;
use url::Url;

use super::repository;
use crate::{config::TmdbConfig, error::AppError};

/// Largest TMDB response accepted (SPEC 8.1). A show with credits and images is well below.
const MAX_BODY: usize = 2 * 1024 * 1024;
/// How long a response is served from the cache.
const CACHE_SECS: f64 = 24.0 * 3600.0;

pub struct TmdbClient {
    http: Client,
    base: Url,
    token: SecretString,
    language: String,
}

impl std::fmt::Debug for TmdbClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TmdbClient")
            .field("base", &self.base.as_str())
            .finish_non_exhaustive()
    }
}

impl TmdbClient {
    pub fn new(cfg: &TmdbConfig, https_only: bool) -> Result<Self, reqwest::Error> {
        let http = Client::builder()
            .timeout(cfg.timeout)
            .connect_timeout(cfg.timeout.min(Duration::from_secs(5)))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(https_only)
            .user_agent("r16a-stream")
            .build()?;
        Ok(Self {
            http,
            base: cfg.base_url.clone(),
            token: cfg.token.clone(),
            language: cfg.language.clone(),
        })
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    /// GETs `segments` under the API root, from the cache when a fresh copy exists. Segments come
    /// from code and numeric ids only. They are still pushed one by one, so no value can add a
    /// path separator or a query.
    pub async fn get(&self, db: &PgPool, segments: &[&str], append: Option<&str>) -> Result<Value, AppError> {
        let cache_key = format!("{}:{}", segments.join("/"), self.language);
        if let Some(hit) = repository::cached(db, &cache_key).await? {
            return Ok(hit);
        }
        let mut url = self.base.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|()| AppError::internal("TMDB base URL cannot take a path"))?;
            path.pop_if_empty();
            for s in segments {
                path.push(s);
            }
        }
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("language", &self.language);
            if let Some(a) = append {
                q.append_pair("append_to_response", a);
            }
        }
        let res = self
            .http
            .get(url)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.token.expose_secret()))
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(timeout = e.is_timeout(), "TMDB request failed");
                AppError::Unavailable
            })?;
        match res.status() {
            s if s.is_success() => {}
            StatusCode::NOT_FOUND => return Err(AppError::NotFound),
            StatusCode::UNAUTHORIZED => {
                tracing::error!("TMDB refused the token, check R16A_TMDB_TOKEN");
                return Err(AppError::Unavailable);
            }
            s => {
                tracing::warn!(status = %s, "TMDB answered with an error");
                return Err(AppError::Unavailable);
            }
        }
        let body = read_capped(res).await?;
        let value: Value = serde_json::from_slice(&body).map_err(|_| {
            tracing::warn!("TMDB answered with invalid JSON");
            AppError::Unavailable
        })?;
        repository::store(db, &cache_key, &value, CACHE_SECS).await?;
        Ok(value)
    }
}

async fn read_capped(mut res: reqwest::Response) -> Result<Vec<u8>, AppError> {
    if res.content_length().is_some_and(|l| l > MAX_BODY as u64) {
        return Err(AppError::Unavailable);
    }
    let mut out = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|_| AppError::Unavailable)? {
        if out.len() + chunk.len() > MAX_BODY {
            tracing::warn!("TMDB response over the size limit");
            return Err(AppError::Unavailable);
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}
