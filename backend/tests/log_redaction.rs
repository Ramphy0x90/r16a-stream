//! Playback tokens and presigned URLs never reach the logs. SPEC 9.5, threat T-PLAY-1.
//!
//! Kept as its own test binary with a single test. A thread-local subscriber can miss events
//! whose interest was cached while other tests ran in parallel, so sharing a binary would make
//! this check flaky.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::get,
};
use r16a_stream_backend::{app, state::AppState};
use sqlx::PgPool;
use tower::ServiceExt;
use tracing_subscriber::layer::SubscriberExt;

async fn send(router: Router, uri: &str) -> axum::response::Response {
    router
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn logs_record_route_templates_never_raw_paths_or_queries(pool: PgPool) {
    let capture = common::LogCapture::default();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("info"))
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(capture.clone())
                .with_current_span(true),
        );
    let _guard = tracing::subscriber::set_default(subscriber);

    let state: AppState = common::state(pool, "http://localhost:4200");
    // Stands in for the playback route, whose path carries the token (SPEC 9.5).
    let extra = Router::new().route("/api/v1/play/{token}/master.m3u8", get(|| async { "#EXTM3U" }));
    let router = app::router_with(state, extra);
    let token = "v1.1.PLAYBACK-TOKEN-SHOULD-NOT-APPEAR.mac";
    let res = send(
        router.clone(),
        &format!("/api/v1/play/{token}/master.m3u8?X-Amz-Signature=SIG-SHOULD-NOT-APPEAR"),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = send(router, "/api/v1/unrouted/ANOTHER-SECRET-SEGMENT").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let logs = capture.text();
    assert!(
        logs.contains("/api/v1/play/{token}/master.m3u8"),
        "route template missing in {logs}"
    );
    assert!(logs.contains("unmatched"));
    for secret in [
        "PLAYBACK-TOKEN-SHOULD-NOT-APPEAR",
        "SIG-SHOULD-NOT-APPEAR",
        "X-Amz-Signature",
        "ANOTHER-SECRET-SEGMENT",
    ] {
        assert!(!logs.contains(secret), "{secret} leaked into logs");
    }
}
