//! Login, refresh, logout and bearer checks never write a credential to the logs, even at
//! debug level (CLAUDE.md, T-AUTH-9). Its own binary for the reason given in `log_redaction.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::net::SocketAddr;

use axum::{
    Extension, Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
    response::Response,
};
use common::idp::{MockIdp, RefreshMode};
use r16a_stream_backend::app;
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt;
use tracing_subscriber::layer::SubscriberExt;
use url::Url;

async fn send(router: &Router, req: Request<Body>) -> Response {
    router.clone().oneshot(req).await.unwrap()
}

fn cookie(res: &Response, name: &str) -> String {
    res.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|v| v.strip_prefix(&format!("{name}=")))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

#[sqlx::test]
async fn credentials_never_reach_the_logs(pool: PgPool) {
    let capture = common::LogCapture::default();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("debug"))
        .with(tracing_subscriber::fmt::layer().json().with_writer(capture.clone()));
    let _guard = tracing::subscriber::set_default(subscriber);

    let idp = MockIdp::start().await;
    let state = common::state_with(pool.clone(), &idp.vars("http://localhost:4200"));
    let router = app::router(state).layer(Extension(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 1)))));

    let res = send(
        &router,
        Request::get("/api/v1/auth/login?returnTo=/x")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let authorize = Url::parse(res.headers()[header::LOCATION].to_str().unwrap()).unwrap();
    let flow = cookie(&res, "__Host-r16a_stream_flow");
    let q = |n: &str| authorize.query_pairs().find(|(k, _)| k == n).unwrap().1.into_owned();
    let (state_param, nonce) = (q("state"), q("nonce"));
    let mut claims = idp.web_claims("yara", true, false);
    claims["nonce"] = json!(nonce);
    idp.add_code("CODE-SHOULD-NOT-APPEAR", claims);
    let res = send(
        &router,
        Request::get(format!(
            "/api/v1/auth/callback?code=CODE-SHOULD-NOT-APPEAR&state={state_param}"
        ))
        .header(header::COOKIE, format!("__Host-r16a_stream_flow={flow}"))
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    let session = cookie(&res, "__Host-r16a_stream_session");

    // A failing refresh logs a warning, which must not carry the refresh token.
    sqlx::query("update sessions set claims_refreshed_at = now() - interval '10 minutes'")
        .execute(&pool)
        .await
        .unwrap();
    idp.set_refresh_mode(RefreshMode::InvalidGrant);
    let res = send(
        &router,
        Request::get("/api/v1/me")
            .header(header::COOKIE, format!("__Host-r16a_stream_session={session}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Refused bearer tokens are logged at debug level, without the token.
    let mut bad = idp.mobile_claims("zed", false, false);
    bad["aud"] = json!("BEARER-AUD-SHOULD-NOT-MATTER");
    let bearer = common::idp::sign(&bad);
    let res = send(
        &router,
        Request::get("/api/v1/me")
            .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let logs = capture.text();
    assert!(logs.contains("web login"), "the login itself is logged: {logs}");
    let refresh_tokens: Vec<String> = idp.revoked();
    for secret in [
        state_param.as_str(),
        nonce.as_str(),
        flow.as_str(),
        session.as_str(),
        "CODE-SHOULD-NOT-APPEAR",
        bearer.as_str(),
        "rt-",
        "test-web-client-secret",
        "Basic ",
    ]
    .into_iter()
    .chain(refresh_tokens.iter().map(String::as_str))
    {
        assert!(!logs.contains(secret), "{secret} leaked into the logs");
    }
}
