//! The middleware stack every API route runs through. SPEC 5.1, 9.5, 11.3.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    routing::get,
};
use r16a_stream_backend::{app, state::AppState};
use serde_json::Value;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;

async fn send(router: Router, uri: &str) -> axum::response::Response {
    router
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn json(res: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap()
}

#[sqlx::test(migrations = false)]
async fn healthz_and_readyz_answer(pool: PgPool) {
    let state = common::state(pool, "http://localhost:4200");
    let res = send(app::router(state.clone()), "/healthz").await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = send(app::router(state), "/readyz").await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn readyz_fails_without_database_and_ignores_storage() {
    // Port 9 is closed, so the pool cannot connect. Storage also points at a closed port, and
    // readiness must depend on the database alone (SPEC 7.8).
    let pool = PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(300))
        .connect_lazy("postgres://nobody:nothing-here@127.0.0.1:9/none")
        .unwrap();
    let state = common::state(pool, "http://localhost:4200");
    let res = send(app::router(state), "/readyz").await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[sqlx::test(migrations = false)]
async fn every_response_carries_the_security_headers(pool: PgPool) {
    let state = common::state(pool, "http://localhost:4200");
    let res = send(app::router(state), "/api/v1/status").await;
    assert_eq!(res.status(), StatusCode::OK);
    let h = res.headers();
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
    assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(h[header::REFERRER_POLICY], "no-referrer");
    assert_eq!(h[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(h["cross-origin-resource-policy"], "same-origin");
    assert!(
        h[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .starts_with("default-src 'none'")
    );
    assert!(h.get("x-request-id").is_some());
    assert!(
        h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
        "the API sends no CORS headers"
    );
    assert!(
        h.get(header::STRICT_TRANSPORT_SECURITY).is_none(),
        "no HSTS on plain HTTP"
    );
    let body = json(res).await;
    assert_eq!(body["service"], "r16a-stream");
    assert_eq!(body["apiVersion"], 1);
}

#[sqlx::test(migrations = false)]
async fn https_origin_adds_hsts(pool: PgPool) {
    let mut vars = common::test_vars("https://stream.example.test", "http://127.0.0.1:9");
    vars.insert("R16A_ENV", "test".into());
    let mut state = common::state(pool, "https://stream.example.test");
    state.config = std::sync::Arc::new(common::config_from(&vars));
    let res = send(app::router(state), "/api/v1/status").await;
    assert!(res.headers().get(header::STRICT_TRANSPORT_SECURITY).is_some());
}

#[sqlx::test(migrations = false)]
async fn unknown_routes_get_a_problem_document(pool: PgPool) {
    let state = common::state(pool, "http://localhost:4200");
    let res = send(app::router(state), "/api/v1/nothing-here").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/problem+json");
    assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
    let request_id = res.headers()["x-request-id"].to_str().unwrap().to_owned();
    let body = json(res).await;
    assert_eq!(body["type"], "urn:r16a-stream:problem:not-found");
    assert_eq!(body["status"], 404);
    assert_eq!(body["requestId"], request_id);
}

#[sqlx::test(migrations = false)]
async fn a_panicking_handler_becomes_a_generic_500(pool: PgPool) {
    let state: AppState = common::state(pool, "http://localhost:4200");
    let extra = Router::new().route(
        "/api/v1/boom",
        get(|| async {
            panic!("secret detail that must not leak");
            #[allow(unreachable_code)]
            ""
        }),
    );
    let res = send(app::router_with(state, extra), "/api/v1/boom").await;
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(res.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    let text = String::from_utf8(to_bytes(res.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap();
    assert!(text.contains("urn:r16a-stream:problem:internal"));
    assert!(!text.contains("secret detail"));
}

#[sqlx::test(migrations = false)]
async fn oversized_bodies_are_refused(pool: PgPool) {
    let state = common::state(pool, "http://localhost:4200");
    let extra = Router::new().route("/api/v1/echo", axum::routing::post(|body: String| async move { body }));
    let big = "x".repeat(app::BODY_LIMIT + 1);
    let res = app::router_with(state, extra)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/echo")
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(big))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/problem+json");
}
