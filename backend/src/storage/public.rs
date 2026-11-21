//! Presigned URLs for browsers, signed for the public origin (SPEC 7.1, 10.3).

use std::time::Duration;

use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use secrecy::ExposeSecret;
use url::Url;

use crate::config::S3Config;

/// Upload part URLs live one hour (SPEC 7.1). A browser asks for the next batch as it goes.
const PART_LIFETIME: Duration = Duration::from_hours(1);

/// Signs `UploadPart` requests for the admin's browser with the API key.
pub struct UploadUrls {
    uploads: Bucket,
    credentials: Credentials,
}

impl std::fmt::Debug for UploadUrls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UploadUrls([redacted])")
    }
}

impl UploadUrls {
    pub fn new(public_endpoint: &Url, s3: &S3Config) -> Option<Self> {
        Some(Self {
            uploads: Bucket::new(
                public_endpoint.clone(),
                UrlStyle::Path,
                s3.uploads_bucket.clone(),
                s3.region.clone(),
            )
            .ok()?,
            credentials: Credentials::new(
                s3.access_key_id.clone(),
                s3.secret_access_key.expose_secret().to_owned(),
            ),
        })
    }

    pub fn part(&self, key: &str, upload_id: &str, part: u16) -> String {
        self.uploads
            .upload_part(Some(&self.credentials), key, part, upload_id)
            .sign_with_time(PART_LIFETIME, &jiff::Timestamp::now())
            .into()
    }
}
