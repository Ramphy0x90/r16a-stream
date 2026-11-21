//! Multipart uploads, listing, HEAD and streaming transfers (SPEC 7.1, 7.3, 7.6, 10.5).

use std::{path::Path, time::Duration};

use reqwest::Method;
use rusty_s3::actions::{CompleteMultipartUpload, CreateMultipartUpload, ListObjectsV2, ListParts};
use tokio::io::AsyncWriteExt;

use super::{BucketName, GarageClient, StorageError, expect_success, valid_key};

/// A part Garage already holds for an unfinished multipart upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPart {
    pub number: u16,
    pub etag: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedObject {
    pub key: String,
    pub size: u64,
}

/// Longest a streamed transfer may take in total. A 60 GiB source at 50 MB/s needs 20 minutes.
const LONG_TRANSFER: Duration = Duration::from_hours(6);
/// Longest pause between two chunks of a streamed transfer before it counts as stalled.
const IDLE: Duration = Duration::from_mins(1);
/// S3 caps `ListParts` and `ListObjectsV2` at 1000 entries per page.
const PAGE_LIMIT: usize = 1000;

impl GarageClient {
    fn check(key: &str) -> Result<(), StorageError> {
        if valid_key(key) {
            Ok(())
        } else {
            Err(StorageError::InvalidKey)
        }
    }

    async fn body_text(res: reqwest::Response) -> Result<String, StorageError> {
        res.text()
            .await
            .map_err(|_| StorageError::Unavailable("body read failed"))
    }

    /// Starts a multipart upload and returns its upload id.
    pub async fn create_multipart(
        &self,
        bucket: BucketName,
        key: &str,
        content_type: &str,
    ) -> Result<String, StorageError> {
        Self::check(key)?;
        let creds = &self.credentials;
        let res = self
            .send(
                bucket,
                &Method::POST,
                |b| Self::signed(&b.create_multipart_upload(Some(creds), key)),
                None,
                &[("content-type", content_type)],
            )
            .await?;
        expect_success(res.status())?;
        let text = Self::body_text(res).await?;
        let parsed = CreateMultipartUpload::parse_response(&text).map_err(|_| StorageError::Unexpected(200))?;
        Ok(parsed.upload_id().to_owned())
    }

    /// Every part Garage holds for an upload, following the part marker across pages.
    pub async fn list_parts(
        &self,
        bucket: BucketName,
        key: &str,
        upload_id: &str,
    ) -> Result<Vec<StoredPart>, StorageError> {
        Self::check(key)?;
        let creds = &self.credentials;
        let mut out = Vec::new();
        let mut marker: Option<u16> = None;
        loop {
            let res = self
                .send(
                    bucket,
                    &Method::GET,
                    |b| {
                        let mut a = b.list_parts(Some(creds), key, upload_id);
                        if let Some(m) = marker {
                            a.set_part_number_marker(m);
                        }
                        Self::signed(&a)
                    },
                    None,
                    &[],
                )
                .await?;
            expect_success(res.status())?;
            let text = Self::body_text(res).await?;
            let page = ListParts::parse_response(&text).map_err(|_| StorageError::Unexpected(200))?;
            out.extend(page.parts.iter().map(|p| StoredPart {
                number: p.number,
                etag: p.etag.clone(),
                size: p.size,
            }));
            // A full page with a marker means more parts follow. Anything else is the end.
            match page.next_part_number_marker {
                Some(m) if Some(m) != marker && page.parts.len() >= usize::from(page.max_parts.max(1)) => {
                    marker = Some(m);
                }
                _ => break,
            }
        }
        Ok(out)
    }

    /// Completes an upload from its parts in order. The part tags are those the browser saw.
    pub async fn complete_multipart(
        &self,
        bucket: BucketName,
        key: &str,
        upload_id: &str,
        etags: &[String],
    ) -> Result<(), StorageError> {
        Self::check(key)?;
        let creds = &self.credentials;
        let body = CompleteMultipartUpload::new(
            match bucket {
                BucketName::Media => &self.endpoints[0].media,
                BucketName::Uploads => &self.endpoints[0].uploads,
            },
            Some(creds),
            key,
            upload_id,
            etags.iter().map(String::as_str),
        )
        .body();
        let res = self
            .send_with(
                bucket,
                &Method::POST,
                |b| {
                    Self::signed(&b.complete_multipart_upload(
                        Some(creds),
                        key,
                        upload_id,
                        etags.iter().map(String::as_str),
                    ))
                },
                Some(body.as_bytes()),
                &[("content-type", "application/xml")],
                Some(Duration::from_mins(5)),
            )
            .await?;
        expect_success(res.status())?;
        // S3 can answer 200 with an error document while completing.
        let text = Self::body_text(res).await?;
        if text.contains("<Error>") {
            return Err(StorageError::Unexpected(200));
        }
        Ok(())
    }

    pub async fn abort_multipart(&self, bucket: BucketName, key: &str, upload_id: &str) -> Result<(), StorageError> {
        Self::check(key)?;
        let creds = &self.credentials;
        let res = self
            .send(
                bucket,
                &Method::DELETE,
                |b| Self::signed(&b.abort_multipart_upload(Some(creds), key, upload_id)),
                None,
                &[],
            )
            .await?;
        match res.status().as_u16() {
            // Already gone counts as aborted.
            404 => Ok(()),
            _ => expect_success(res.status()),
        }
    }

    /// The object's size, or `None` when it does not exist.
    pub async fn head_object(&self, bucket: BucketName, key: &str) -> Result<Option<u64>, StorageError> {
        Self::check(key)?;
        let creds = &self.credentials;
        let res = self
            .send(
                bucket,
                &Method::HEAD,
                |b| Self::signed(&b.head_object(Some(creds), key)),
                None,
                &[],
            )
            .await?;
        if res.status().as_u16() == 404 {
            return Ok(None);
        }
        expect_success(res.status())?;
        // From the header. reqwest's content_length() is the body size, always 0 for HEAD.
        let size = res
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or(StorageError::Unexpected(200))?;
        Ok(Some(size))
    }

    /// Streams an object into a file, never into memory. Refuses more than `max_bytes`.
    /// Returns the bytes written.
    pub async fn get_to_file(
        &self,
        bucket: BucketName,
        key: &str,
        path: &Path,
        max_bytes: u64,
    ) -> Result<u64, StorageError> {
        Self::check(key)?;
        let creds = &self.credentials;
        let mut res = self
            .send_with(
                bucket,
                &Method::GET,
                |b| Self::signed(&b.get_object(Some(creds), key)),
                None,
                &[],
                Some(LONG_TRANSFER),
            )
            .await?;
        expect_success(res.status())?;
        if res.content_length().is_some_and(|l| l > max_bytes) {
            return Err(StorageError::Unexpected(413));
        }
        let mut file = tokio::fs::File::create(path)
            .await
            .map_err(|_| StorageError::Unavailable("scratch write failed"))?;
        let mut written: u64 = 0;
        loop {
            let chunk = tokio::time::timeout(IDLE, res.chunk())
                .await
                .map_err(|_| StorageError::Unavailable("download stalled"))?
                .map_err(|_| StorageError::Unavailable("download interrupted"))?;
            let Some(chunk) = chunk else { break };
            written += chunk.len() as u64;
            if written > max_bytes {
                return Err(StorageError::Unexpected(413));
            }
            file.write_all(&chunk)
                .await
                .map_err(|_| StorageError::Unavailable("scratch write failed"))?;
        }
        file.flush()
            .await
            .map_err(|_| StorageError::Unavailable("scratch write failed"))?;
        Ok(written)
    }

    /// Uploads a local file as one object. Media outputs are segments of a few MB.
    pub async fn put_file(
        &self,
        bucket: BucketName,
        key: &str,
        path: &Path,
        content_type: &str,
        cache_control: Option<&str>,
    ) -> Result<u64, StorageError> {
        let data = tokio::fs::read(path)
            .await
            .map_err(|_| StorageError::Unavailable("scratch read failed"))?;
        self.put_object(bucket, key, &data, content_type, cache_control).await?;
        Ok(data.len() as u64)
    }

    /// Every object under `prefix`, following continuation tokens.
    pub async fn list_prefix(&self, bucket: BucketName, prefix: &str) -> Result<Vec<ListedObject>, StorageError> {
        let creds = &self.credentials;
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let res = self
                .send(
                    bucket,
                    &Method::GET,
                    |b| {
                        let mut a = b.list_objects_v2(Some(creds));
                        a.with_prefix(prefix);
                        a.with_max_keys(PAGE_LIMIT);
                        if let Some(t) = &token {
                            a.with_continuation_token(t.clone());
                        }
                        Self::signed(&a)
                    },
                    None,
                    &[],
                )
                .await?;
            expect_success(res.status())?;
            let text = Self::body_text(res).await?;
            let page = ListObjectsV2::parse_response(&text).map_err(|_| StorageError::Unexpected(200))?;
            out.extend(page.contents.iter().map(|c| ListedObject {
                key: c.key.clone(),
                size: c.size.unwrap_or(0),
            }));
            match page.next_continuation_token {
                Some(t) if Some(&t) != token.as_ref() => token = Some(t),
                _ => break,
            }
        }
        Ok(out)
    }

    /// The next level of "directories" under `prefix`, `v/` gives `v/<variant>/`. Follows
    /// continuation tokens. Encode GC walks the media bucket this way instead of listing every
    /// segment (SPEC 7.6).
    pub async fn list_dirs(&self, bucket: BucketName, prefix: &str) -> Result<Vec<String>, StorageError> {
        let creds = &self.credentials;
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let res = self
                .send(
                    bucket,
                    &Method::GET,
                    |b| {
                        let mut a = b.list_objects_v2(Some(creds));
                        a.with_prefix(prefix);
                        a.with_delimiter("/");
                        a.with_max_keys(PAGE_LIMIT);
                        if let Some(t) = &token {
                            a.with_continuation_token(t.clone());
                        }
                        Self::signed(&a)
                    },
                    None,
                    &[],
                )
                .await?;
            expect_success(res.status())?;
            let text = Self::body_text(res).await?;
            let page = ListObjectsV2::parse_response(&text).map_err(|_| StorageError::Unexpected(200))?;
            out.extend(page.common_prefixes.iter().map(|c| c.prefix.clone()));
            // Objects directly under the prefix, such as `s/<id>.vtt`, count as entries too.
            out.extend(page.contents.iter().map(|c| c.key.clone()));
            match page.next_continuation_token {
                Some(t) if Some(&t) != token.as_ref() => token = Some(t),
                _ => break,
            }
        }
        Ok(out)
    }

    /// Deletes every object under `prefix`, one by one. Background work only (encode GC).
    pub async fn delete_prefix(&self, bucket: BucketName, prefix: &str) -> Result<usize, StorageError> {
        let objects = self.list_prefix(bucket, prefix).await?;
        for o in &objects {
            self.delete_object(bucket, &o.key).await?;
        }
        Ok(objects.len())
    }
}
