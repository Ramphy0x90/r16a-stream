//! Authorisation matrix over every route in `docs/openapi.json` (SPEC 6.6, T-AZ-1).
//!
//! Each route declares the status it must answer for five callers. A route in the document
//! without a row fails the test, and so does a row whose route is gone, so a new endpoint
//! cannot ship without someone deciding who may call it.
//!
//! Callers use bearer tokens. Cookie sessions reach the same `Caller` and are covered, with
//! CSRF, in `auth_web.rs`. Every row runs against freshly seeded data, so a delete in one row
//! cannot change the answer of the next. Placeholders in the concrete path pick what the route
//! acts on, `{user}` is the owner's user id and `{title}`, `{season}`, `{episode}` a published
//! show with one season and one episode. `shared` gains meaning once lists can be shared.
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
use r16a_stream_backend::{
    app,
    catalog::{
        dto::{EpisodeInput, SeasonInput, TitleInput, TitleKind},
        service::{self, Actor},
    },
    state::AppState,
};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

#[derive(Clone, Copy)]
enum Who {
    Anonymous,
    Owner,
    Other,
    Shared,
    Admin,
}

const CALLERS: [Who; 5] = [Who::Anonymous, Who::Owner, Who::Other, Who::Shared, Who::Admin];
const NAMES: [&str; 5] = ["anonymous", "owner", "other", "shared", "admin"];

/// Expected statuses, in the order anonymous, owner, other, shared, admin.
struct Row {
    method: &'static str,
    /// The template exactly as in `openapi.json`.
    route: &'static str,
    /// The request path with placeholders, see the module docs.
    path: &'static str,
    body: Option<&'static str>,
    expect: [u16; 5],
}

const fn row(
    method: &'static str,
    route: &'static str,
    path: &'static str,
    body: Option<&'static str>,
    expect: [u16; 5],
) -> Row {
    Row {
        method,
        route,
        path,
        body,
        expect,
    }
}

const MEMBER_READ: [u16; 5] = [401, 200, 200, 200, 200];
/// Admin routes answer 404 to members, never 403 (SPEC 6.6).
const fn admin(ok: u16) -> [u16; 5] {
    [401, 404, 404, 404, ok]
}

const MATRIX: &[Row] = &[
    row(
        "get",
        "/api/v1/status",
        "/api/v1/status",
        None,
        [200, 200, 200, 200, 200],
    ),
    // Login routes are anonymous by design. The callback without a code redirects to the
    // sign-in page with an error.
    row(
        "get",
        "/api/v1/auth/login",
        "/api/v1/auth/login",
        None,
        [302, 302, 302, 302, 302],
    ),
    row(
        "get",
        "/api/v1/auth/callback",
        "/api/v1/auth/callback",
        None,
        [302, 302, 302, 302, 302],
    ),
    // Logout is a web session action, bearer callers have nothing to end here.
    row(
        "post",
        "/api/v1/auth/logout",
        "/api/v1/auth/logout",
        None,
        [401, 401, 401, 401, 401],
    ),
    row("get", "/api/v1/me", "/api/v1/me", None, MEMBER_READ),
    row(
        "patch",
        "/api/v1/me/settings",
        "/api/v1/me/settings",
        Some("{}"),
        MEMBER_READ,
    ),
    // Catalog reads, published titles for every member.
    row("get", "/api/v1/genres", "/api/v1/genres", None, MEMBER_READ),
    row("get", "/api/v1/languages", "/api/v1/languages", None, MEMBER_READ),
    row("get", "/api/v1/titles", "/api/v1/titles", None, MEMBER_READ),
    row(
        "get",
        "/api/v1/home/recently-added",
        "/api/v1/home/recently-added",
        None,
        MEMBER_READ,
    ),
    row(
        "get",
        "/api/v1/titles/{id}",
        "/api/v1/titles/{title}",
        None,
        MEMBER_READ,
    ),
    row(
        "get",
        "/api/v1/titles/{id}/seasons/{number}/episodes",
        "/api/v1/titles/{title}/seasons/1/episodes",
        None,
        MEMBER_READ,
    ),
    // Catalog administration. The TMDB lookup passes authorisation and then finds TMDB
    // unreachable in this test, which is what 503 shows.
    row(
        "get",
        "/api/v1/admin/tmdb/lookup",
        "/api/v1/admin/tmdb/lookup?ref=603&kind=movie",
        None,
        admin(503),
    ),
    row(
        "post",
        "/api/v1/admin/genres",
        "/api/v1/admin/genres",
        Some(r#"{"name":"Film noir"}"#),
        admin(201),
    ),
    row("get", "/api/v1/admin/titles", "/api/v1/admin/titles", None, admin(200)),
    row(
        "post",
        "/api/v1/admin/titles",
        "/api/v1/admin/titles",
        Some(r#"{"kind":"movie","name":"Heat"}"#),
        admin(201),
    ),
    row(
        "get",
        "/api/v1/admin/titles/{id}",
        "/api/v1/admin/titles/{title}",
        None,
        admin(200),
    ),
    row(
        "patch",
        "/api/v1/admin/titles/{id}",
        "/api/v1/admin/titles/{title}",
        Some(r#"{"year":2001}"#),
        admin(204),
    ),
    row(
        "delete",
        "/api/v1/admin/titles/{id}",
        "/api/v1/admin/titles/{title}",
        None,
        admin(204),
    ),
    row(
        "post",
        "/api/v1/admin/titles/{id}/publish",
        "/api/v1/admin/titles/{title}/publish",
        None,
        admin(204),
    ),
    row(
        "post",
        "/api/v1/admin/titles/{id}/unpublish",
        "/api/v1/admin/titles/{title}/unpublish",
        None,
        admin(204),
    ),
    row(
        "post",
        "/api/v1/admin/titles/{id}/seasons",
        "/api/v1/admin/titles/{title}/seasons",
        Some(r#"{"number":2}"#),
        admin(201),
    ),
    row(
        "patch",
        "/api/v1/admin/seasons/{id}",
        "/api/v1/admin/seasons/{season}",
        Some(r#"{"name":"S1"}"#),
        admin(204),
    ),
    row(
        "delete",
        "/api/v1/admin/seasons/{id}",
        "/api/v1/admin/seasons/{season}",
        None,
        admin(204),
    ),
    row(
        "post",
        "/api/v1/admin/seasons/{id}/episodes",
        "/api/v1/admin/seasons/{season}/episodes",
        Some(r#"{"number":2}"#),
        admin(201),
    ),
    row(
        "patch",
        "/api/v1/admin/episodes/{id}",
        "/api/v1/admin/episodes/{episode}",
        Some(r#"{"name":"E1"}"#),
        admin(204),
    ),
    row(
        "delete",
        "/api/v1/admin/episodes/{id}",
        "/api/v1/admin/episodes/{episode}",
        None,
        admin(204),
    ),
    // Last, because it revokes the owner's tokens, which in the same second would refuse the
    // owner's freshly minted tokens in the rows after it.
    row(
        "post",
        "/api/v1/admin/users/{id}/revoke-sessions",
        "/api/v1/admin/users/{user}/revoke-sessions",
        None,
        admin(204),
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
            declared.insert((r.method.to_owned(), r.route.to_owned())),
            "duplicate row {} {}",
            r.method,
            r.route
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

struct Seed {
    title: Uuid,
    season: Uuid,
    episode: Uuid,
}

/// A published show with season 1 and episode 1, created through the service as the admin.
async fn seed(state: &AppState, admin_id: Uuid) -> Seed {
    let actor = Actor {
        user_id: admin_id,
        request_id: None,
    };
    let input = TitleInput {
        kind: TitleKind::Show,
        name: format!("Show {}", Uuid::new_v4().simple()),
        original_name: None,
        overview: None,
        year: Some(2008),
        runtime_minutes: None,
        tmdb_id: None,
        genre_ids: vec![],
        new_genres: vec![],
        credits: vec![],
        poster_path: None,
        backdrop_path: None,
        seasons: vec![SeasonInput {
            number: 1,
            name: None,
            overview: None,
            year: None,
            episodes: vec![EpisodeInput {
                number: 1,
                name: None,
                overview: None,
                runtime_minutes: Some(47),
                air_date: None,
            }],
        }],
    };
    let title = service::create_title(&state.db, actor, &input, service::Imported::default())
        .await
        .unwrap();
    service::set_published(&state.db, actor, title, true).await.unwrap();
    let detail = service::admin_title(&state.db, &state.image_urls, title).await.unwrap();
    Seed {
        title,
        season: detail.seasons[0].id,
        episode: detail.seasons[0].episodes[0].id,
    }
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

    // Provision every caller and learn the owner's and the admin's ids.
    let mut ids = [Uuid::nil(); 5];
    for (i, who) in CALLERS.into_iter().enumerate().skip(1) {
        let (status, body) = send(&state, i, "get", "/api/v1/me", None, token(who).as_deref()).await;
        assert_eq!(status, 200);
        ids[i] = serde_json::from_str::<Value>(&body).unwrap()["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
    }

    let mut failures = Vec::new();
    for r in MATRIX {
        for (i, who) in CALLERS.into_iter().enumerate() {
            let s = seed(&state, ids[4]).await;
            let path = r
                .path
                .replace("{user}", &ids[1].to_string())
                .replace("{title}", &s.title.to_string())
                .replace("{season}", &s.season.to_string())
                .replace("{episode}", &s.episode.to_string());
            let (status, _) = send(&state, i, r.method, &path, r.body, token(who).as_deref()).await;
            if status != r.expect[i] {
                failures.push(format!(
                    "{} {} as {}: expected {}, got {status}",
                    r.method, r.route, NAMES[i], r.expect[i]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[sqlx::test]
async fn drafts_are_invisible_to_members_and_listed_for_admins(pool: PgPool) {
    let idp = MockIdp::start().await;
    let state = common::state_with(pool, &idp.vars("http://localhost:4200"));
    let member = idp.mobile_token("m", true, false);
    let admin = idp.mobile_token("a", true, true);
    let (_, body) = send(&state, 1, "get", "/api/v1/me", None, Some(&admin)).await;
    let admin_id: Uuid = serde_json::from_str::<Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let s = seed(&state, admin_id).await;
    service::set_published(
        &state.db,
        Actor {
            user_id: admin_id,
            request_id: None,
        },
        s.title,
        false,
    )
    .await
    .unwrap();
    for path in [
        format!("/api/v1/titles/{}", s.title),
        format!("/api/v1/titles/{}/seasons/1/episodes", s.title),
    ] {
        assert_eq!(
            send(&state, 2, "get", &path, None, Some(&member)).await.0,
            404,
            "{path}"
        );
        assert_eq!(
            send(&state, 3, "get", &path, None, Some(&admin)).await.0,
            404,
            "admins use the admin routes"
        );
    }
    let (_, list) = send(&state, 2, "get", "/api/v1/titles", None, Some(&member)).await;
    assert_eq!(
        serde_json::from_str::<Value>(&list).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let (status, _) = send(
        &state,
        3,
        "get",
        &format!("/api/v1/admin/titles/{}", s.title),
        None,
        Some(&admin),
    )
    .await;
    assert_eq!(status, 200);
}

/// Sends one request from a caller. Each caller gets its own client address, so the per-IP
/// login limit of one caller does not bleed into the next.
async fn send(
    state: &AppState,
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
