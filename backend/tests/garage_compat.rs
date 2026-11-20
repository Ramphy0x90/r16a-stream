//! Every Garage behaviour the design relies on, against the pinned `dxflrs/garage:v1.0.1`.
//! SPEC 10.1, 10.5, DECISIONS 036, 053, 054, 055.
//!
//! Needs `R16A_TEST_S3_ENDPOINT` (the S3 port) and `R16A_TEST_GARAGE_ADMIN` (the admin port) of
//! the dev compose Garage, which has the buckets and keys from `deploy/compose/garage/init.sh`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use r16a_stream_backend::storage::{BucketName, GarageClient, StorageError};
use reqwest::StatusCode;
use rusty_s3::{
    Bucket, Credentials, S3Action, UrlStyle,
    actions::{CreateMultipartUpload, ListObjectsV2, ListParts},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MEDIA: &str = "r16a-stream-media";
const UPLOADS: &str = "r16a-stream-uploads";

fn endpoints() -> Option<(String, String)> {
    Some((
        common::integration_env("R16A_TEST_S3_ENDPOINT")?,
        common::integration_env("R16A_TEST_GARAGE_ADMIN")?,
    ))
}

fn client(s3: &str, key: &str, secret: &str) -> GarageClient {
    let mut vars = common::test_vars("http://localhost:4200", s3);
    vars.insert("R16A_S3_ACCESS_KEY_ID", key.into());
    vars.insert("R16A_S3_SECRET_ACCESS_KEY", secret.into());
    GarageClient::new(&common::config_from(&vars).s3).unwrap()
}

fn bucket(base: &str, name: &str) -> Bucket {
    Bucket::new(
        base.parse().unwrap(),
        UrlStyle::Path,
        name.to_owned(),
        "garage".to_owned(),
    )
    .unwrap()
}

fn unique(prefix: &str) -> String {
    format!("_test/{prefix}-{}", uuid::Uuid::new_v4().simple())
}

#[tokio::test]
async fn put_get_delete_keep_content_type_and_cache_control() {
    let Some((s3, _)) = endpoints() else { return };
    let api = client(&s3, common::API_KEY_ID, common::API_SECRET);
    let key = unique("roundtrip");
    api.put_object(
        BucketName::Uploads,
        &key,
        b"hello",
        "text/plain",
        Some("public, max-age=31536000, immutable"),
    )
    .await
    .unwrap();
    let (body, cache_control) = api.get_object(BucketName::Uploads, &key).await.unwrap();
    assert_eq!(body, b"hello");
    assert_eq!(cache_control.as_deref(), Some("public, max-age=31536000, immutable"));
    api.delete_object(BucketName::Uploads, &key).await.unwrap();
    assert!(matches!(
        api.get_object(BucketName::Uploads, &key).await,
        Err(StorageError::NotFound)
    ));
}

#[tokio::test]
async fn keys_are_scoped_per_bucket() {
    let Some((s3, _)) = endpoints() else { return };
    let api = client(&s3, common::API_KEY_ID, common::API_SECRET);
    let worker = client(&s3, common::WORKER_KEY_ID, common::WORKER_SECRET);
    let key = unique("scope");
    assert!(matches!(
        api.put_object(BucketName::Media, &key, b"x", "text/plain", None).await,
        Err(StorageError::Denied(403))
    ));
    assert!(matches!(
        worker
            .put_object(BucketName::Uploads, &key, b"x", "text/plain", None)
            .await,
        Err(StorageError::Denied(403))
    ));
    worker
        .put_object(BucketName::Media, &key, b"x", "text/plain", None)
        .await
        .unwrap();
    // The API key reads media, it only cannot write it.
    assert_eq!(api.get_object(BucketName::Media, &key).await.unwrap().0, b"x");
    worker.delete_object(BucketName::Media, &key).await.unwrap();
}

#[tokio::test]
async fn presigned_urls_validate_with_an_hour_floored_signing_time_and_the_public_host() {
    let Some((s3, _)) = endpoints() else { return };
    let worker = client(&s3, common::WORKER_KEY_ID, common::WORKER_SECRET);
    let key = unique("presign");
    worker
        .put_object(BucketName::Media, &key, b"segment", "video/mp4", None)
        .await
        .unwrap();

    // Signed for the public origin, as playback URLs are, and floored to the hour (SPEC 9.3).
    let public = bucket("http://localhost:4200", MEDIA);
    let creds = Credentials::new(common::WORKER_KEY_ID, common::WORKER_SECRET);
    let now = jiff::Timestamp::now().as_second();
    let floored = jiff::Timestamp::from_second(now - now.rem_euclid(3600)).unwrap();
    let url = public
        .get_object(Some(&creds), &key)
        .sign_with_time(Duration::from_hours(7), &floored);

    // Sent to Garage directly with the public Host header, which is what Traefik and the dev
    // proxy do, since they pass the Host through unchanged.
    let direct = url.as_str().replacen("http://localhost:4200", &s3, 1);
    let res = reqwest::Client::new()
        .get(direct)
        .header(reqwest::header::HOST, "localhost:4200")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.bytes().await.unwrap().as_ref(), b"segment");
    worker.delete_object(BucketName::Media, &key).await.unwrap();
}

#[tokio::test]
async fn multipart_upload_list_parts_complete_and_abort() {
    let Some((s3, _)) = endpoints() else { return };
    let http = reqwest::Client::new();
    let creds = Credentials::new(common::API_KEY_ID, common::API_SECRET);
    let b = bucket(&s3, UPLOADS);
    let key = unique("multipart");
    let ttl = Duration::from_secs(600);

    let create = http
        .post(b.create_multipart_upload(Some(&creds), &key).sign(ttl))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let created = CreateMultipartUpload::parse_response(&create.text().await.unwrap()).unwrap();
    let upload_id = created.upload_id().to_owned();

    // Every part but the last must be at least 5 MiB. The real part size is 50 MiB.
    let part1 = vec![7u8; 5 * 1024 * 1024];
    let part2 = vec![9u8; 10];
    let mut etags = Vec::new();
    for (n, data) in [(1u16, part1), (2u16, part2)] {
        let url = b.upload_part(Some(&creds), &key, n, &upload_id).sign(ttl);
        let res = http.put(url).body(data).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        etags.push(res.headers()["etag"].to_str().unwrap().to_owned());
    }

    let listed = http
        .get(b.list_parts(Some(&creds), &key, &upload_id).sign(ttl))
        .send()
        .await
        .unwrap();
    let listing = ListParts::parse_response(&listed.text().await.unwrap()).unwrap();
    assert_eq!(listing.parts.len(), 2, "resume relies on ListParts");

    let complete = b.complete_multipart_upload(Some(&creds), &key, &upload_id, etags.iter().map(String::as_str));
    let url = complete.sign(ttl);
    let res = http.post(url).body(complete.body()).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let head = http
        .head(b.head_object(Some(&creds), &key).sign(ttl))
        .send()
        .await
        .unwrap();
    assert_eq!(
        head.headers()["content-length"],
        (5 * 1024 * 1024 + 10).to_string().as_str()
    );
    http.delete(b.delete_object(Some(&creds), &key).sign(ttl))
        .send()
        .await
        .unwrap();

    // Abort leaves nothing behind.
    let create = http
        .post(b.create_multipart_upload(Some(&creds), &key).sign(ttl))
        .send()
        .await
        .unwrap();
    let upload_id = CreateMultipartUpload::parse_response(&create.text().await.unwrap())
        .unwrap()
        .upload_id()
        .to_owned();
    let res = http
        .delete(b.abort_multipart_upload(Some(&creds), &key, &upload_id).sign(ttl))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());
}

#[tokio::test]
async fn list_objects_v2_groups_by_delimiter() {
    let Some((s3, _)) = endpoints() else { return };
    let worker = client(&s3, common::WORKER_KEY_ID, common::WORKER_SECRET);
    let root = unique("list");
    for k in ["a/1", "a/2", "b/1"] {
        worker
            .put_object(BucketName::Media, &format!("{root}/{k}"), b"x", "text/plain", None)
            .await
            .unwrap();
    }
    let creds = Credentials::new(common::WORKER_KEY_ID, common::WORKER_SECRET);
    let b = bucket(&s3, MEDIA);
    let mut list: ListObjectsV2<'_> = b.list_objects_v2(Some(&creds));
    let prefix = format!("{root}/");
    list.with_prefix(prefix.as_str());
    list.with_delimiter("/");
    let res = reqwest::get(list.sign(Duration::from_secs(60))).await.unwrap();
    let parsed = ListObjectsV2::parse_response(&res.text().await.unwrap()).unwrap();
    let mut prefixes: Vec<_> = parsed.common_prefixes.iter().map(|p| p.prefix.clone()).collect();
    prefixes.sort();
    assert_eq!(prefixes, vec![format!("{root}/a/"), format!("{root}/b/")]);
    for k in ["a/1", "a/2", "b/1"] {
        worker
            .delete_object(BucketName::Media, &format!("{root}/{k}"))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn admin_port_grants_nothing_beyond_metrics_to_the_metrics_token() {
    let Some((_, admin)) = endpoints() else { return };
    let http = reqwest::Client::new();
    let status = |path: &'static str, token: Option<&'static str>| {
        let mut req = http.get(format!("{admin}{path}"));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        async move { req.send().await.unwrap().status() }
    };
    assert_eq!(
        status("/health", None).await,
        StatusCode::OK,
        "health is open by design"
    );
    assert_eq!(status("/metrics", None).await, StatusCode::FORBIDDEN);
    assert_eq!(status("/metrics", Some(common::METRICS_TOKEN)).await, StatusCode::OK);
    for path in ["/v1/status", "/v1/key?list", "/v1/bucket", "/v1/layout"] {
        assert_eq!(
            status(path, Some(common::METRICS_TOKEN)).await,
            StatusCode::FORBIDDEN,
            "{path}"
        );
        assert_eq!(status(path, None).await, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test]
async fn metrics_expose_the_capacity_values_the_api_parses() {
    let Some((_, admin)) = endpoints() else { return };
    let text = reqwest::Client::new()
        .get(format!("{admin}/metrics"))
        .bearer_auth(common::METRICS_TOKEN)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    for name in [
        "garage_local_disk_avail{volume=\"data\"}",
        "garage_local_disk_total{volume=\"data\"}",
        "garage_replication_factor",
        "cluster_storage_nodes",
        "cluster_storage_nodes_ok",
    ] {
        assert!(text.lines().any(|l| l.starts_with(name)), "{name} missing");
    }
}

#[tokio::test]
async fn anonymous_get_on_the_s3_port_answers_403() {
    // The Traefik 3.7 health check in DECISIONS 054 expects exactly this from a live node.
    let Some((s3, _)) = endpoints() else { return };
    assert_eq!(
        reqwest::get(format!("{s3}/")).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

/// Sends a raw request line, so bytes that no HTTP client would produce reach the server.
async fn raw_status(base: &str, target: &[u8]) -> Option<u16> {
    let addr = base.trim_start_matches("http://");
    let mut stream = tokio::net::TcpStream::connect(addr).await.ok()?;
    let mut req = b"GET ".to_vec();
    req.extend_from_slice(target);
    req.extend_from_slice(b" HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(&req).await.ok()?;
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut buf))
        .await
        .ok()?
        .ok()?;
    std::str::from_utf8(buf.get(9..12)?).ok()?.parse().ok()
}

#[tokio::test]
async fn invalid_utf8_in_path_and_query_does_not_crash_garage() {
    // DECISIONS 055. Garage 2.5.0 fixed a panic on invalid UTF-8 after percent-decoding. This
    // proves 1.0.1 answers such requests without dying, on both ports.
    let Some((s3, admin)) = endpoints() else { return };
    let targets: [&[u8]; 7] = [
        b"/r16a-stream-media/%FF%FE",
        b"/r16a-stream-media/\xff\xfe",
        b"/r16a-stream-media/a?x=%FF%FE",
        b"/r16a-stream-media/a?%FF%FE=1",
        b"/r16a-stream-media/a?x=\xff\xfe",
        b"/r16a-stream-uploads/a?partNumber=1&uploadId=%C0%AF",
        b"/%FF%FE?list-type=2",
    ];
    for base in [&s3, &admin] {
        for t in targets {
            let status = raw_status(base, t).await;
            assert!(
                matches!(status, Some(400..=499)),
                "{base} {} answered {status:?}, expected a 4xx and a live server",
                String::from_utf8_lossy(t)
            );
        }
    }
    // Still alive afterwards.
    assert_eq!(
        reqwest::get(format!("{admin}/health")).await.unwrap().status(),
        StatusCode::OK
    );
}
