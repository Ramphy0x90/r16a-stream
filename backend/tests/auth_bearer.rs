//! Mobile bearer tokens against a mock Authentik (SPEC 6.4, 6.8, T-AUTH-8 and 10).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::net::SocketAddr;

use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Method, Request, StatusCode, header},
    response::Response,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use common::idp::{self, MockIdp};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use r16a_stream_backend::{app, state::AppState};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

struct Api {
    idp: MockIdp,
    state: AppState,
}

impl Api {
    async fn new(pool: PgPool) -> Self {
        let idp = MockIdp::start().await;
        let state = common::state_with(pool, &idp.vars("http://localhost:4200"));
        Self { idp, state }
    }

    fn router(&self) -> Router {
        app::router(self.state.clone()).layer(Extension(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 1)))))
    }

    async fn call(&self, method: Method, uri: &str, token: &str) -> Response {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        self.router().oneshot(req).await.unwrap()
    }

    async fn me(&self, token: &str) -> Response {
        self.call(Method::GET, "/api/v1/me", token).await
    }
}

async fn json(res: Response) -> Value {
    serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap()
}

#[sqlx::test]
async fn first_call_provisions_and_later_calls_reuse_the_user(pool: PgPool) {
    let api = Api::new(pool).await;
    let token = api.idp.mobile_token("nina", true, false);
    let first = json(api.me(&token).await).await;
    let second = json(api.me(&token).await).await;
    assert_eq!(first["id"], second["id"]);
    assert_eq!(first["displayName"], "Name nina");
    let users: i64 = sqlx::query_scalar("select count(*) from users")
        .fetch_one(&api.state.db)
        .await
        .unwrap();
    assert_eq!(users, 1);
}

#[sqlx::test]
async fn web_and_mobile_logins_of_one_person_share_the_account(pool: PgPool) {
    // Different providers mean different issuers, but Authentik's hashed user id subject is the
    // same across them, so both resolve to one user (DECISIONS 071).
    let api = Api::new(pool).await;
    let mobile = json(api.me(&api.idp.mobile_token("olga", true, false)).await).await;
    let mut conn = api.state.db.acquire().await.unwrap();
    let web = r16a_stream_backend::auth::repository::upsert_user(
        &mut conn,
        &api.state.auth.account_issuer,
        "olga",
        "Olga",
        false,
    )
    .await
    .unwrap();
    assert_eq!(mobile["id"], web.id.to_string());
}

#[sqlx::test]
async fn admin_comes_from_the_token_of_each_request(pool: PgPool) {
    let api = Api::new(pool).await;
    assert_eq!(
        json(api.me(&api.idp.mobile_token("pia", true, true)).await).await["isAdmin"],
        true
    );
    assert_eq!(
        json(api.me(&api.idp.mobile_token("pia", true, false)).await).await["isAdmin"],
        false
    );
}

#[sqlx::test]
async fn tokens_that_are_not_r16a_stream_access_tokens_are_refused(pool: PgPool) {
    let api = Api::new(pool).await;
    let base = api.idp.mobile_claims("quinn", true, false);
    let with = |f: &dyn Fn(&mut Value)| {
        let mut c = base.clone();
        f(&mut c);
        idp::sign(&c)
    };
    let now = idp::now();
    let cases: Vec<(&str, String)> = vec![
        ("not a member", with(&|c| c["r16a_stream_user"] = json!(false))),
        (
            "member claim missing",
            with(&|c| drop(c.as_object_mut().unwrap().remove("r16a_stream_user"))),
        ),
        (
            "member claim as a string",
            with(&|c| c["r16a_stream_user"] = json!("true")),
        ),
        (
            "ID token, no scope",
            with(&|c| drop(c.as_object_mut().unwrap().remove("scope"))),
        ),
        (
            "scope without r16a-stream",
            with(&|c| c["scope"] = json!("openid profile r16a-streamx")),
        ),
        ("web client audience", with(&|c| c["aud"] = json!(idp::WEB_CLIENT_ID))),
        ("web issuer", with(&|c| c["iss"] = json!(api.idp.issuer("web")))),
        ("other authorized party", with(&|c| c["azp"] = json!("someone-else"))),
        ("expired", with(&|c| c["exp"] = json!(now - 120))),
        ("not yet valid", with(&|c| c["nbf"] = json!(now + 600))),
        ("issued in the future", with(&|c| c["iat"] = json!(now + 600))),
        ("no subject", with(&|c| drop(c.as_object_mut().unwrap().remove("sub")))),
        ("signed by a foreign key", idp::sign_foreign(&base)),
        ("garbage", String::from("not.a.jwt")),
    ];
    for (why, token) in cases {
        assert_eq!(api.me(&token).await.status(), StatusCode::UNAUTHORIZED, "{why}");
    }
    let users: i64 = sqlx::query_scalar("select count(*) from users")
        .fetch_one(&api.state.db)
        .await
        .unwrap();
    assert_eq!(users, 0, "nothing is provisioned from a refused token");
}

#[sqlx::test]
async fn only_rs256_is_accepted(pool: PgPool) {
    let api = Api::new(pool).await;
    let claims = api.idp.mobile_claims("rita", true, false);
    let mut h = Header::new(Algorithm::HS256);
    h.kid = Some(idp::KID.into());
    let hs = jsonwebtoken::encode(&h, &claims, &EncodingKey::from_secret(b"guessable").unwrap()).unwrap();
    assert_eq!(api.me(&hs).await.status(), StatusCode::UNAUTHORIZED);

    // alg none, hand built, since no library signs it.
    let none = format!(
        "{}.{}.",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"none","kid":"test-key"}"#),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    assert_eq!(api.me(&none).await.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn unknown_key_ids_refetch_the_jwks_at_most_once_a_minute(pool: PgPool) {
    let api = Api::new(pool).await;
    assert_eq!(
        api.me(&api.idp.mobile_token("sam", true, false)).await.status(),
        StatusCode::OK
    );
    assert_eq!(api.idp.jwks_fetches(), 1);
    let claims = api.idp.mobile_claims("sam", true, false);
    for i in 0..5 {
        let t = idp::sign_with_kid(&claims, &format!("random-{i}"));
        assert_eq!(api.me(&t).await.status(), StatusCode::UNAUTHORIZED);
    }
    assert_eq!(api.idp.jwks_fetches(), 1, "T-AUTH-10, no refetch storm");
}

#[sqlx::test]
async fn malformed_authorization_headers_are_unauthenticated(pool: PgPool) {
    let api = Api::new(pool).await;
    for value in ["Bearer", "Bearer ", "Basic abc", "bearer x"] {
        let req = Request::get("/api/v1/me")
            .header(header::AUTHORIZATION, value)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            api.router().oneshot(req).await.unwrap().status(),
            StatusCode::UNAUTHORIZED,
            "{value}"
        );
    }
    let long = "a".repeat(9000);
    assert_eq!(api.me(&long).await.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn revoke_sessions_cuts_tokens_in_hand_and_writes_audit(pool: PgPool) {
    let api = Api::new(pool).await;
    let user_token = api.idp.mobile_token("tess", true, false);
    let user_id = json(api.me(&user_token).await).await["id"].as_str().unwrap().to_owned();
    let admin_token = api.idp.mobile_token("uma", true, true);

    let uri = format!("/api/v1/admin/users/{user_id}/revoke-sessions");
    let res = api.call(Method::POST, &uri, &user_token).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "members do not see admin routes");
    let res = api.call(Method::POST, &uri, &admin_token).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    assert_eq!(api.me(&user_token).await.status(), StatusCode::UNAUTHORIZED);
    // A refresh after the revocation keeps the old auth_time, so it does not bring access back.
    let mut refreshed = api.idp.mobile_claims("tess", true, false);
    refreshed["iat"] = json!(idp::now() + 2);
    // As Authentik does, the refreshed token keeps the sign-in time from before the revocation.
    // Set explicitly, a fresh now() could fall into the next second and look like a new sign-in.
    refreshed["auth_time"] = json!(idp::now() - 5);
    assert_eq!(api.me(&idp::sign(&refreshed)).await.status(), StatusCode::UNAUTHORIZED);
    let mut no_auth_time = refreshed.clone();
    no_auth_time.as_object_mut().unwrap().remove("auth_time");
    assert_eq!(
        api.me(&idp::sign(&no_auth_time)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    // Only a new interactive sign-in, with a new auth_time, works again.
    let mut signed_in_again = refreshed;
    signed_in_again["auth_time"] = json!(idp::now() + 2);
    assert_eq!(api.me(&idp::sign(&signed_in_again)).await.status(), StatusCode::OK);

    let (action, actor): (String, Option<uuid::Uuid>) = sqlx::query_as("select action, actor_id from audit_log")
        .fetch_one(&api.state.db)
        .await
        .unwrap();
    assert_eq!(action, "user.revoke_sessions");
    assert!(actor.is_some());

    let missing = format!("/api/v1/admin/users/{}/revoke-sessions", uuid::Uuid::new_v4());
    assert_eq!(
        api.call(Method::POST, &missing, &admin_token).await.status(),
        StatusCode::NOT_FOUND
    );
    let malformed = "/api/v1/admin/users/not-a-uuid/revoke-sessions";
    assert_eq!(
        api.call(Method::POST, malformed, &admin_token).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test]
async fn disabled_users_are_refused(pool: PgPool) {
    let api = Api::new(pool).await;
    let token = api.idp.mobile_token("vic", true, false);
    assert_eq!(api.me(&token).await.status(), StatusCode::OK);
    sqlx::query("update users set disabled_at = now()")
        .execute(&api.state.db)
        .await
        .unwrap();
    assert_eq!(api.me(&token).await.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn bearer_requests_need_no_csrf_header(pool: PgPool) {
    let api = Api::new(pool).await;
    let token = api.idp.mobile_token("wes", true, false);
    let req = Request::patch("/api/v1/me/settings")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"uiLanguage":"it","preferredAudioLanguage":"it"}"#))
        .unwrap();
    let res = api.router().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let me = json(res).await;
    assert_eq!(me["uiLanguage"], "it");
    assert_eq!(me["preferredAudioLanguage"], "it");
}

#[sqlx::test]
async fn unreachable_authentik_is_a_503_not_a_401(pool: PgPool) {
    // Default test config points the issuers at a closed port.
    let state = common::state_with(pool, &common::test_vars("http://localhost:4200", "http://127.0.0.1:9"));
    let token = MockIdp::start().await.mobile_token("xia", true, false);
    let req = Request::get("/api/v1/me")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app::router(state).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}
