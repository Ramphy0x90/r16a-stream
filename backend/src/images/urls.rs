//! Presigned image URLs for clients (SPEC 8.3).
//!
//! Signed for the public origin with the signing time floored to the UTC day and 48 hours of
//! validity. Every URL is then identical for every user for a whole day, so browsers and the
//! mobile cache reuse it, and the catalog's entity tags change at most daily.

use std::time::Duration;

use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use secrecy::ExposeSecret;
use url::Url;

use crate::config::S3Config;

const LIFETIME: Duration = Duration::from_hours(48);
const DAY: i64 = 86_400;

pub struct ImageUrls {
    bucket: Bucket,
    credentials: Credentials,
}

impl std::fmt::Debug for ImageUrls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ImageUrls([redacted])")
    }
}

impl ImageUrls {
    pub fn new(public_endpoint: &Url, s3: &S3Config) -> Option<Self> {
        let bucket = Bucket::new(
            public_endpoint.clone(),
            UrlStyle::Path,
            s3.media_bucket.clone(),
            s3.region.clone(),
        )
        .ok()?;
        Some(Self {
            bucket,
            credentials: Credentials::new(
                s3.access_key_id.clone(),
                s3.secret_access_key.expose_secret().to_owned(),
            ),
        })
    }

    /// The URL of one object, the same for the whole UTC day of `now`.
    pub fn url(&self, key: &str, now: jiff::Timestamp) -> String {
        let secs = now.as_second();
        let day = jiff::Timestamp::from_second(secs - secs.rem_euclid(DAY)).unwrap_or(now);
        self.bucket
            .get_object(Some(&self.credentials), key)
            .sign_with_time(LIFETIME, &day)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls() -> ImageUrls {
        let s3 = S3Config {
            internal_endpoints: vec![],
            region: "garage".into(),
            media_bucket: "r16a-stream-media".into(),
            uploads_bucket: "r16a-stream-uploads".into(),
            access_key_id: "GK0123456789abcdef01234567".into(),
            secret_access_key: "9be2320de5bf8e5b07297443f66bcb7ac546fee5bf2c3f91d7c070e385a43d99".into(),
            connect_timeout: Duration::from_secs(1),
            timeout: Duration::from_secs(1),
            probe_interval: Duration::from_secs(30),
        };
        ImageUrls::new(&Url::parse("https://stream.example.test").expect("url"), &s3).expect("urls")
    }

    #[test]
    fn urls_are_stable_within_a_utc_day_and_change_after() {
        let u = urls();
        let morning = jiff::Timestamp::from_second(1_800_000_000 - 1_800_000_000 % DAY + 60).expect("t");
        let evening = jiff::Timestamp::from_second(morning.as_second() + 20 * 3600).expect("t");
        let tomorrow = jiff::Timestamp::from_second(morning.as_second() + DAY).expect("t");
        let a = u.url("i/x/342.avif", morning);
        assert_eq!(a, u.url("i/x/342.avif", evening));
        assert_ne!(a, u.url("i/x/342.avif", tomorrow));
        assert!(a.starts_with("https://stream.example.test/r16a-stream-media/i/x/342.avif?"));
        assert!(a.contains("X-Amz-Expires=172800"));
    }
}
