//! Authorisation matrix over every route in `docs/openapi.json` (SPEC 6.6, T-AZ-1).
//!
//! Each route declares the status it must answer for five callers. A route in the document
//! without a row fails the test, and so does a row whose route is gone, so a new endpoint
//! cannot ship without someone deciding who may call it.
//!
//! Callers use bearer tokens. Cookie sessions reach the same `Caller` and are covered, with
//! CSRF, in `auth_web.rs`. Path parameters named `id` are filled with the owner's id, so the
//! owner row says what happens to one's own resource and `other` and `shared` what happens to
//! someone else's. `shared` gains meaning once lists can be shared (Phase 6).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{collections::BTreeSet, net::SocketAddr, path::PathBuf};

use axum::{
    Extension,
    body::Body,
    extract::ConnectInfo,
    http::{Method, Request, StatusCode, header},
};
use common::idp::MockIdp;
use r16a_stream_backend::app;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

#[derive(Clone, Copy)]
enum Who {
    Anonymous,
    Owner,
    Other,
    Shared,
    Admin,
}

const CALLERS: [Who; 5] = [Who::Anonymous, Who::Owner, Who::Other, Who::Shared, Who::Admin];

/// Expected statuses, in the order anonymous, owner, other, shared, admin.
struct Row {
    method: &'static str,
    path: &'static str,
    body: Option<&'static str>,
    expect: [u16; 5],
}

const fn row(method: &'static str, path: &'static str, body: Option<&'static str>, expect: [u16; 5]) -> Row {
    Row {
        method,
        path,
        body,
        expect,
    }
}

const MATRIX: &[Row] = &[
    row("get", "/api/v1/status", None, [200, 200, 200, 200, 200]),
    // Login routes are anonymous by design. The callback without a code redirects to the
    // sign-in page with an error.
    row("get", "/api/v1/auth/login", None, [302, 302, 302, 302, 302]),
    row("get", "/api/v1/auth/callback", None, [302, 302, 302, 302, 302]),
    // Logout is a web session action, bearer callers have nothing to end here.
    row("post", "/api/v1/auth/logout", None, [401, 401, 401, 401, 401]),
    row("get", "/api/v1/me", None, [401, 200, 200, 200, 200]),
    row("patch", "/api/v1/me/settings", Some("{}"), [401, 200, 200, 200, 200]),
    // Admin routes answer 404 to members, never 403 (SPEC 6.6).
    row(
        "post",
        "/api/v1/admin/users/{id}/revoke-sessions",
        None,
        [401, 404, 404, 404, 204],
    ),
];

fn documented_routes() -> BTreeSet<(String, String)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/openapi.json");
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut out = BTreeSet::new();
    for (p, item) in doc["paths"].as_object().unwrap() {
        for m in item.as_object().unwrap().keys() {
            if ["get", "put", "post", "patch", "delete", "head", "options"].contains(&m.as_str()) {
                out.insert((m.clone(), p.clone()));
            }
        }
    }
    out
}

#[test]
fn every_documented_route_has_exactly_one_matrix_row() {
    let documented = documented_routes();
    let mut declared = BTreeSet::new();
    for r in MATRIX {
        assert!(
            declared.insert((r.method.to_owned(), r.path.to_owned())),
            "duplicate row {} {}",
            r.method,
            r.path
        );
    }
    let missing: Vec<_> = documented.difference(&declared).collect();
    let stale: Vec<_> = declared.difference(&documented).collect();
    assert!(missing.is_empty(), "routes without an authz row: {missing:?}");
    assert!(
        stale.is_empty(),
        "authz rows for routes that no longer exist: {stale:?}"
    );
}

#[sqlx::test]
async fn every_route_answers_each_caller_as_declared(pool: PgPool) {
    let idp = MockIdp::start().await;
    let state = common::state_with(pool, &idp.vars("http://localhost:4200"));
    let token = |who: Who| match who {
        Who::Anonymous => None,
        Who::Owner => Some(idp.mobile_token("owner", true, false)),
        Who::Other => Some(idp.mobile_token("other", true, false)),
        Who::Shared => Some(idp.mobile_token("shared", true, false)),
        Who::Admin => Some(idp.mobile_token("admin", true, true)),
    };

    // Seed every caller's user row, and learn the owner's id for path parameters.
    let mut owner_id = String::new();
    for (i, who) in CALLERS.into_iter().enumerate().skip(1) {
        let res = send(&state, i, "get", "/api/v1/me", None, token(who).as_deref()).await;
        assert_eq!(res.0, 200);
        if matches!(who, Who::Owner) {
            owner_id = serde_json::from_str::<Value>(&res.1).unwrap()["id"]
                .as_str()
                .unwrap()
                .to_owned();
        }
    }

    let mut failures = Vec::new();
    for r in MATRIX {
        let path = r.path.replace("{id}", &owner_id);
        for (i, who) in CALLERS.into_iter().enumerate() {
            let (status, _) = send(&state, i, r.method, &path, r.body, token(who).as_deref()).await;
            if status != r.expect[i] {
                failures.push(format!(
                    "{} {} as {}: expected {}, got {status}",
                    r.method,
                    r.path,
                    ["anonymous", "owner", "other", "shared", "admin"][i],
                    r.expect[i]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Sends one request from a caller. Each caller gets its own client address, so the per-IP
/// login limit of one caller does not bleed into the next.
async fn send(
    state: &r16a_stream_backend::state::AppState,
    caller: usize,
    method: &str,
    path: &str,
    body: Option<&str>,
    token: Option<&str>,
) -> (u16, String) {
    let mut b = Request::builder()
        .method(Method::from_bytes(method.to_uppercase().as_bytes()).unwrap())
        .uri(path);
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let body = match body {
        Some(json) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(json.to_owned())
        }
        None => Body::empty(),
    };
    let peer = SocketAddr::from(([192, 0, 2, u8::try_from(caller + 1).unwrap()], 1));
    let res = app::router(state.clone())
        .layer(Extension(ConnectInfo(peer)))
        .oneshot(b.body(body).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
    assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR, "{method} {path}");
    (status.as_u16(), String::from_utf8_lossy(&bytes).into_owned())
}
