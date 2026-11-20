//! Brings a TMDB image into the uploads bucket (SPEC 8.2 step 1).

use std::time::Duration;

use reqwest::{Client, StatusCode, header};
use url::Url;
use uuid::Uuid;

use crate::{
    error::AppError,
    storage::{BucketName, GarageClient},
};

/// Largest image accepted from TMDB.
pub const MAX_BYTES: usize = 20 * 1024 * 1024;

/// A TMDB file path such as `/kU3B75TyRiCgE270EyZnHjfivoq.jpg`. Nothing else is ever appended to
/// the image base URL.
pub fn valid_tmdb_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix('/') else {
        return false;
    };
    let Some((stem, ext)) = rest.rsplit_once('.') else {
        return false;
    };
    (1..=64).contains(&stem.len()) && stem.bytes().all(|b| b.is_ascii_alphanumeric()) && matches!(ext, "jpg" | "png")
}

pub struct TmdbImages {
    http: Client,
    base: Url,
}

impl std::fmt::Debug for TmdbImages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TmdbImages")
            .field("base", &self.base.as_str())
            .finish_non_exhaustive()
    }
}

impl TmdbImages {
    pub fn new(base: Url, https_only: bool) -> Result<Self, reqwest::Error> {
        let http = Client::builder()
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(https_only)
            .user_agent("r16a-stream")
            .build()?;
        Ok(Self { http, base })
    }

    /// Downloads one image and stores it as the source of a new image. Returns the new image id
    /// and the stored size. The image row and its job are written by the caller.
    pub async fn import(&self, storage: &GarageClient, tmdb_path: &str) -> Result<(Uuid, i64), AppError> {
        if !valid_tmdb_path(tmdb_path) {
            return Err(AppError::BadRequest("invalid-image-path"));
        }
        let url = self
            .base
            .join(&tmdb_path[1..])
            .map_err(|_| AppError::BadRequest("invalid-image-path"))?;
        let mut res = self.http.get(url).send().await.map_err(|e| {
            tracing::warn!(timeout = e.is_timeout(), "TMDB image download failed");
            AppError::Unavailable
        })?;
        match res.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => return Err(AppError::BadRequest("image-not-found")),
            s => {
                tracing::warn!(status = %s, "TMDB image answered with an error");
                return Err(AppError::Unavailable);
            }
        }
        let content_type = match res.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) {
            Some(ct) if ct.starts_with("image/jpeg") => "image/jpeg",
            Some(ct) if ct.starts_with("image/png") => "image/png",
            _ => return Err(AppError::BadRequest("image-type")),
        };
        if res.content_length().is_some_and(|l| l > MAX_BYTES as u64) {
            return Err(AppError::BadRequest("image-too-large"));
        }
        let mut body = Vec::new();
        while let Some(chunk) = res.chunk().await.map_err(|_| AppError::Unavailable)? {
            if body.len() + chunk.len() > MAX_BYTES {
                return Err(AppError::BadRequest("image-too-large"));
            }
            body.extend_from_slice(&chunk);
        }
        if body.is_empty() {
            return Err(AppError::BadRequest("image-type"));
        }
        let id = Uuid::new_v4();
        storage
            .put_object(BucketName::Uploads, &super::source_key(id), &body, content_type, None)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "storing an image source failed");
                if e.is_retryable() {
                    AppError::StorageDegraded
                } else {
                    AppError::internal(e)
                }
            })?;
        Ok((id, i64::try_from(body.len()).unwrap_or(i64::MAX)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_tmdb_file_paths() {
        assert!(valid_tmdb_path("/kU3B75TyRiCgE270EyZnHjfivoq.jpg"));
        assert!(valid_tmdb_path("/a.png"));
        for bad in [
            "kU3.jpg",
            "/../etc/passwd.jpg",
            "/a/b.jpg",
            "/a.gif",
            "/a.jpg?x=1",
            "//evil.example/a.jpg",
            "/.jpg",
            &format!("/{}.jpg", "a".repeat(65)),
        ] {
            assert!(!valid_tmdb_path(bad), "{bad}");
        }
    }
}
