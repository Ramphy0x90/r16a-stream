//! Presigned URLs and the bucket routing through a real Traefik pinned to the cluster baseline.
//! SPEC 10.3, 20, C1, C5.
//!
//! Needs the dev compose stack and the harness in `deploy/compose/test/traefik`, then
//! `R16A_TEST_TRAEFIK_URL=http://localhost:8097` and `R16A_TEST_S3_ENDPOINT`. The harness puts
//! a dead endpoint next to Garage, so these tests also show what users get with a node down.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use reqwest::StatusCode;
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle, actions::CreateMultipartUpload};

fn env() -> Option<(String, String)> {
    Some((
        common::integration_env("R16A_TEST_TRAEFIK_URL")?,
        common::integration_env("R16A_TEST_S3_ENDPOINT")?,
    ))
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

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

#[tokio::test]
async fn presigned_get_through_traefik_survives_a_dead_node() {
    let Some((traefik, s3)) = env() else { return };
    let creds = Credentials::new(common::WORKER_KEY_ID, common::WORKER_SECRET);
    let key = format!("_test/traefik-{}", uuid::Uuid::new_v4().simple());
    let direct = bucket(&s3, "r16a-stream-media");
    let put = http()
        .put(direct.put_object(Some(&creds), &key).sign(Duration::from_secs(60)))
        .body("via traefik")
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);

    // Signed for the public origin, exactly like playback URLs.
    let public = bucket(&traefik, "r16a-stream-media");
    let url = public.get_object(Some(&creds), &key).sign(Duration::from_secs(600));
    // Round robin alternates between Garage and the dead endpoint. Every GET still succeeds,
    // because the retry middleware sends the failed dial to the other server.
    for _ in 0..4 {
        let res = http()
            .get(url.clone())
            .header("cookie", "__Host-r16a_stream_session=must-not-reach-garage")
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()["content-security-policy"], "default-src 'none'; sandbox");
        assert_eq!(res.text().await.unwrap(), "via traefik");
    }
    http()
        .delete(direct.delete_object(Some(&creds), &key).sign(Duration::from_secs(60)))
        .send()
        .await
        .unwrap();
}

#[tokio::test]
async fn upload_parts_are_never_retried_by_traefik_and_succeed_when_the_client_retries() {
    let Some((traefik, s3)) = env() else { return };
    let creds = Credentials::new(common::API_KEY_ID, common::API_SECRET);
    let key = format!("_test/traefik-part-{}", uuid::Uuid::new_v4().simple());
    let direct = bucket(&s3, "r16a-stream-uploads");
    let created = http()
        .post(
            direct
                .create_multipart_upload(Some(&creds), &key)
                .sign(Duration::from_secs(60)),
        )
        .send()
        .await
        .unwrap();
    let upload_id = CreateMultipartUpload::parse_response(&created.text().await.unwrap())
        .unwrap()
        .upload_id()
        .to_owned();

    let public = bucket(&traefik, "r16a-stream-uploads");
    let url = public
        .upload_part(Some(&creds), &key, 1, &upload_id)
        .sign(Duration::from_secs(600));
    let mut statuses = Vec::new();
    for _ in 0..4 {
        let status = http()
            .put(url.clone())
            .body(vec![1u8; 1024])
            .send()
            .await
            .unwrap()
            .status();
        statuses.push(status);
        if status == StatusCode::OK {
            break;
        }
    }
    assert_eq!(statuses.last(), Some(&StatusCode::OK), "{statuses:?}");
    // A part that landed on the dead endpoint failed instead of being replayed by Traefik.
    // Whether the first attempt hits it depends on the round robin position, so only the
    // shape of the sequence is asserted.
    assert!(
        statuses[..statuses.len() - 1].iter().all(StatusCode::is_server_error),
        "{statuses:?}"
    );

    http()
        .delete(
            direct
                .abort_multipart_upload(Some(&creds), &key, &upload_id)
                .sign(Duration::from_secs(60)),
        )
        .send()
        .await
        .unwrap();
}

#[tokio::test]
async fn credentials_never_reach_the_upstream() {
    let Some((traefik, _)) = env() else { return };
    // The echo host has the same rules and middlewares, but answers with the request it got.
    let base = traefik.replace("localhost", "127.0.0.1");
    for (method, path) in [
        ("GET", "/r16a-stream-media/v/a/seg_1.m4s?X-Amz-Signature=abc"),
        ("PUT", "/r16a-stream-uploads/s/a?partNumber=1&uploadId=abc"),
    ] {
        let echoed = http()
            .request(method.parse().unwrap(), format!("{base}{path}"))
            .header("host", "echo.localhost")
            .header("cookie", "__Host-r16a_stream_session=SESSION-SECRET")
            .header("authorization", "Bearer TOKEN-SECRET")
            .body("x")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(echoed.contains(path), "{echoed}");
        assert!(!echoed.to_ascii_lowercase().contains("cookie:"), "{method} {echoed}");
        assert!(
            !echoed.to_ascii_lowercase().contains("authorization:"),
            "{method} {echoed}"
        );
        assert!(!echoed.contains("SECRET"), "{method} {echoed}");
    }
}

/// Method, path and extra headers of a request that must be refused.
type Case = (&'static str, &'static str, &'static [(&'static str, &'static str)]);

#[tokio::test]
async fn only_the_documented_methods_and_queries_pass() {
    let Some((traefik, _)) = env() else { return };
    let cases: [Case; 7] = [
        ("POST", "/r16a-stream-media/x", &[]),
        ("PUT", "/r16a-stream-media/x", &[]),
        ("DELETE", "/r16a-stream-media/x", &[]),
        ("GET", "/r16a-stream-media/", &[]),
        ("PUT", "/r16a-stream-uploads/a?uploadId=x", &[]),
        (
            "PUT",
            "/r16a-stream-uploads/a?partNumber=1&uploadId=x",
            &[("x-amz-copy-source", "/r16a-stream-media/x")],
        ),
        ("POST", "/r16a-stream-uploads/a?uploads", &[]),
    ];
    for (method, path, headers) in cases {
        let mut req = http()
            .request(method.parse().unwrap(), format!("{traefik}{path}"))
            .body("x");
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        assert_eq!(
            req.send().await.unwrap().status(),
            StatusCode::FORBIDDEN,
            "{method} {path}"
        );
    }
}
