//! Web login through the BFF against a mock Authentik (SPEC 6.3, T-AUTH-1 to 14).
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
use common::idp::{MockIdp, RefreshMode};
use r16a_stream_backend::{app, state::AppState};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use url::Url;

const ORIGIN: &str = "http://localhost:4200";

struct Web {
    idp: MockIdp,
    state: AppState,
}

impl Web {
    async fn new(pool: PgPool) -> Self {
        let idp = MockIdp::start().await;
        let state = common::state_with(pool, &idp.vars(ORIGIN));
        Self { idp, state }
    }

    fn router(&self, peer: [u8; 4]) -> Router {
        // The same extension `into_make_service_with_connect_info` inserts in the real server.
        app::router(self.state.clone()).layer(Extension(ConnectInfo(SocketAddr::from((peer, 40000)))))
    }

    async fn send(&self, method: Method, uri: &str, headers: &[(&str, &str)], body: Option<Value>) -> Response {
        let mut b = Request::builder().method(method).uri(uri);
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let body = match body {
            Some(v) => {
                b = b.header(header::CONTENT_TYPE.as_str(), "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        self.router([192, 0, 2, 1])
            .oneshot(b.body(body).unwrap())
            .await
            .unwrap()
    }

    /// Starts a login and returns the authorize URL and the flow cookie value.
    async fn start(&self, return_to: &str) -> (Url, String) {
        let uri = format!("/api/v1/auth/login?returnTo={}", urlencode(return_to));
        let res = self.send(Method::GET, &uri, &[], None).await;
        assert_eq!(res.status(), StatusCode::FOUND);
        let location = Url::parse(res.headers()[header::LOCATION].to_str().unwrap()).unwrap();
        (
            location,
            set_cookie(&res, "__Host-r16a_stream_flow").expect("flow cookie"),
        )
    }

    async fn callback(&self, code: &str, state: &str, flow_cookie: Option<&str>) -> Response {
        let uri = format!("/api/v1/auth/callback?code={code}&state={state}&iss=ignored");
        let cookie = flow_cookie.map(|c| format!("__Host-r16a_stream_flow={c}"));
        let headers: Vec<(&str, &str)> = cookie.iter().map(|c| ("cookie", c.as_str())).collect();
        self.send(Method::GET, &uri, &headers, None).await
    }

    /// A full login. Returns the session cookie value.
    async fn login(&self, sub: &str, admin: bool) -> String {
        let (authorize, flow) = self.start("/title/1").await;
        let mut claims = self.idp.web_claims(sub, true, admin);
        claims["nonce"] = json!(param(&authorize, "nonce"));
        let code = format!("code-{sub}");
        self.idp.add_code(&code, claims);
        let res = self.callback(&code, &param(&authorize, "state"), Some(&flow)).await;
        assert_eq!(res.status(), StatusCode::FOUND);
        assert_eq!(res.headers()[header::LOCATION], "/title/1");
        set_cookie(&res, "__Host-r16a_stream_session").expect("session cookie")
    }

    async fn me(&self, session: &str) -> Response {
        let cookie = format!("__Host-r16a_stream_session={session}");
        self.send(Method::GET, "/api/v1/me", &[("cookie", &cookie)], None).await
    }

    async fn age_claims(&self, secs: i32) {
        sqlx::query("update sessions set claims_refreshed_at = now() - make_interval(secs => $1)")
            .bind(f64::from(secs))
            .execute(&self.state.db)
            .await
            .unwrap();
    }

    async fn session_count(&self) -> i64 {
        sqlx::query_scalar("select count(*) from sessions")
            .fetch_one(&self.state.db)
            .await
            .unwrap()
    }
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn param(url: &Url, name: &str) -> String {
    url.query_pairs().find(|(k, _)| k == name).unwrap().1.into_owned()
}

fn set_cookie(res: &Response, name: &str) -> Option<String> {
    res.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|v| {
            v.strip_prefix(&format!("{name}="))
                .map(|rest| rest.split(';').next().unwrap().to_owned())
        })
        .filter(|v| !v.is_empty())
}

async fn json(res: Response) -> Value {
    serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap()
}

#[sqlx::test]
async fn login_redirect_carries_pkce_state_and_nonce(pool: PgPool) {
    let w = Web::new(pool).await;
    let (url, flow) = w.start("/title/1").await;
    assert_eq!(url.path(), "/application/o/authorize/");
    assert_eq!(param(&url, "response_type"), "code");
    assert_eq!(param(&url, "client_id"), common::idp::WEB_CLIENT_ID);
    assert_eq!(param(&url, "redirect_uri"), format!("{ORIGIN}/api/v1/auth/callback"));
    // No profile and no email scope, their default mappings would put every group name and
    // the address into the tokens (DECISIONS 074).
    assert_eq!(param(&url, "scope"), "openid offline_access r16a-stream");
    assert_eq!(param(&url, "code_challenge_method"), "S256");
    assert_eq!(param(&url, "code_challenge").len(), 43);
    assert_eq!(param(&url, "state").len(), 43);
    assert_eq!(param(&url, "nonce").len(), 43);
    assert_eq!(flow.len(), 43);

    // Only hashes and the sealed verifier are stored, never the values in the URL or cookie.
    let row: (Vec<u8>, Vec<u8>, String) =
        sqlx::query_as("select state_hash, pkce_verifier_enc, return_to from oidc_flows")
            .fetch_one(&w.state.db)
            .await
            .unwrap();
    assert_eq!(row.0.len(), 32);
    assert!(!String::from_utf8_lossy(&row.1).contains(&param(&url, "state")));
    assert_eq!(row.2, "/title/1");
}

#[sqlx::test]
async fn full_login_creates_a_sealed_session_and_me_answers(pool: PgPool) {
    let w = Web::new(pool).await;
    let session = w.login("alice", false).await;

    // client_secret_basic toward the token endpoint.
    assert!(w.idp.last_token_auth().unwrap().starts_with("Basic "));

    let res = w.me(&session).await;
    assert_eq!(res.status(), StatusCode::OK);
    let me = json(res).await;
    assert_eq!(me["displayName"], "Name alice");
    assert_eq!(me["isAdmin"], false);
    assert_eq!(me["theme"], "dark");

    let (hash, enc): (Vec<u8>, Vec<u8>) = sqlx::query_as("select token_hash, refresh_token_enc from sessions")
        .fetch_one(&w.state.db)
        .await
        .unwrap();
    assert_eq!(hash.len(), 32, "only the hash of the cookie is stored");
    assert!(
        !String::from_utf8_lossy(&enc).contains("rt-"),
        "refresh token is sealed"
    );
    let issuer: String = sqlx::query_scalar("select oidc_issuer from users")
        .fetch_one(&w.state.db)
        .await
        .unwrap();
    assert!(
        !issuer.contains("/application/"),
        "users are keyed by the instance origin"
    );
}

#[sqlx::test]
async fn session_cookie_has_the_host_prefix_and_strict_flags(pool: PgPool) {
    let w = Web::new(pool).await;
    let (authorize, flow) = w.start("/").await;
    let mut claims = w.idp.web_claims("bob", true, false);
    claims["nonce"] = json!(param(&authorize, "nonce"));
    w.idp.add_code("c", claims);
    let res = w.callback("c", &param(&authorize, "state"), Some(&flow)).await;
    let cookies: Vec<String> = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect();
    let session = cookies
        .iter()
        .find(|c| c.starts_with("__Host-r16a_stream_session="))
        .unwrap();
    for attr in ["Path=/", "HttpOnly", "SameSite=Strict", "Secure"] {
        assert!(session.contains(attr), "{attr} in {session}");
    }
    assert!(
        cookies
            .iter()
            .any(|c| c.starts_with("__Host-r16a_stream_flow=;") && c.contains("Max-Age=0")),
        "flow cookie cleared"
    );
    assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
}

#[sqlx::test]
async fn return_to_outside_the_origin_becomes_root(pool: PgPool) {
    let w = Web::new(pool).await;
    for bad in ["//evil.example/x", "https://evil.example", "/\\evil.example"] {
        w.start(bad).await;
    }
    let rows: Vec<String> = sqlx::query_scalar("select return_to from oidc_flows")
        .fetch_all(&w.state.db)
        .await
        .unwrap();
    assert_eq!(rows, vec!["/", "/", "/"]);
}

#[sqlx::test]
async fn callback_without_the_binding_cookie_fails_and_burns_the_state(pool: PgPool) {
    let w = Web::new(pool).await;
    let (authorize, flow) = w.start("/").await;
    let mut claims = w.idp.web_claims("carol", true, false);
    claims["nonce"] = json!(param(&authorize, "nonce"));
    w.idp.add_code("c", claims);
    let state = param(&authorize, "state");

    let res = w.callback("c", &state, None).await;
    assert_eq!(res.headers()[header::LOCATION], "/signin?error=failed");
    // The flow row is gone, so even the right cookie cannot finish it now (T-AUTH-1).
    let res = w.callback("c", &state, Some(&flow)).await;
    assert_eq!(res.headers()[header::LOCATION], "/signin?error=failed");
    assert_eq!(w.session_count().await, 0);
}

#[sqlx::test]
async fn callback_with_another_browsers_cookie_fails(pool: PgPool) {
    let w = Web::new(pool).await;
    let (authorize, _) = w.start("/").await;
    let (_, other_flow) = w.start("/").await;
    let mut claims = w.idp.web_claims("dave", true, false);
    claims["nonce"] = json!(param(&authorize, "nonce"));
    w.idp.add_code("c", claims);
    let res = w.callback("c", &param(&authorize, "state"), Some(&other_flow)).await;
    assert_eq!(res.headers()[header::LOCATION], "/signin?error=failed");
    assert_eq!(w.session_count().await, 0);
}

#[sqlx::test]
async fn wrong_nonce_or_audience_fails(pool: PgPool) {
    let w = Web::new(pool).await;

    let (authorize, flow) = w.start("/").await;
    let mut claims = w.idp.web_claims("erin", true, false);
    claims["nonce"] = json!("not-the-nonce");
    w.idp.add_code("c1", claims);
    let res = w.callback("c1", &param(&authorize, "state"), Some(&flow)).await;
    assert_eq!(res.headers()[header::LOCATION], "/signin?error=failed");

    let (authorize, flow) = w.start("/").await;
    let mut claims = w.idp.web_claims("erin", true, false);
    claims["nonce"] = json!(param(&authorize, "nonce"));
    claims["aud"] = json!(common::idp::MOBILE_CLIENT_ID);
    w.idp.add_code("c2", claims);
    let res = w.callback("c2", &param(&authorize, "state"), Some(&flow)).await;
    assert_eq!(res.headers()[header::LOCATION], "/signin?error=failed");

    let (authorize, flow) = w.start("/").await;
    let mut claims = w.idp.web_claims("erin", true, false);
    claims["nonce"] = json!(param(&authorize, "nonce"));
    claims["azp"] = json!("some-other-client");
    w.idp.add_code("c3", claims);
    let res = w.callback("c3", &param(&authorize, "state"), Some(&flow)).await;
    assert_eq!(res.headers()[header::LOCATION], "/signin?error=failed");
    assert_eq!(w.session_count().await, 0);
}

#[sqlx::test]
async fn non_member_is_denied_and_their_token_revoked(pool: PgPool) {
    let w = Web::new(pool).await;
    let (authorize, flow) = w.start("/").await;
    let mut claims = w.idp.web_claims("frank", false, false);
    claims["nonce"] = json!(param(&authorize, "nonce"));
    w.idp.add_code("c", claims);
    let res = w.callback("c", &param(&authorize, "state"), Some(&flow)).await;
    assert_eq!(res.headers()[header::LOCATION], "/signin?error=denied");
    assert_eq!(w.session_count().await, 0);
    assert_eq!(w.idp.revoked().len(), 1);
}

#[sqlx::test]
async fn provider_error_on_callback_fails_cleanly(pool: PgPool) {
    let w = Web::new(pool).await;
    let res = w
        .send(
            Method::GET,
            "/api/v1/auth/callback?error=access_denied&state=x",
            &[],
            None,
        )
        .await;
    assert_eq!(res.headers()[header::LOCATION], "/signin?error=failed");
}

#[sqlx::test]
async fn state_changing_cookie_requests_need_the_csrf_header_and_origin(pool: PgPool) {
    let w = Web::new(pool).await;
    let session = w.login("gina", false).await;
    let cookie = format!("__Host-r16a_stream_session={session}");
    let body = json!({ "theme": "light" });

    for headers in [
        vec![("cookie", cookie.as_str())],
        vec![("cookie", cookie.as_str()), ("x-r16a-request", "1")],
        vec![
            ("cookie", cookie.as_str()),
            ("x-r16a-request", "1"),
            ("origin", "https://evil.example"),
        ],
        vec![("cookie", cookie.as_str()), ("origin", ORIGIN)],
    ] {
        let res = w
            .send(Method::PATCH, "/api/v1/me/settings", &headers, Some(body.clone()))
            .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{headers:?}");
    }
    let ok = [("cookie", cookie.as_str()), ("x-r16a-request", "1"), ("origin", ORIGIN)];
    let res = w.send(Method::PATCH, "/api/v1/me/settings", &ok, Some(body)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(json(res).await["theme"], "light");

    let res = w
        .send(
            Method::PATCH,
            "/api/v1/me/settings",
            &ok,
            Some(json!({ "theme": "neon" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let res = w
        .send(
            Method::PATCH,
            "/api/v1/me/settings",
            &ok,
            Some(json!({ "isAdmin": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "unknown fields are refused");
}

#[sqlx::test]
async fn stale_claims_refresh_once_and_pick_up_admin_changes(pool: PgPool) {
    let w = Web::new(pool).await;
    let session = w.login("hana", false).await;
    assert_eq!(json(w.me(&session).await).await["isAdmin"], false);
    assert_eq!(w.idp.refresh_calls(), 0, "fresh claims are not refreshed");

    w.age_claims(301).await;
    w.idp.set_next_refresh_claims(w.idp.web_claims("hana", true, true));
    assert_eq!(json(w.me(&session).await).await["isAdmin"], true);
    assert_eq!(w.idp.refresh_calls(), 1);
    assert_eq!(json(w.me(&session).await).await["isAdmin"], true);
    assert_eq!(w.idp.refresh_calls(), 1, "rotated token stored, no second refresh");
}

#[sqlx::test]
async fn concurrent_requests_refresh_a_session_only_once(pool: PgPool) {
    let w = Web::new(pool).await;
    let session = w.login("ivan", false).await;
    w.age_claims(301).await;
    let (a, b, c, d, e, f) = tokio::join!(
        w.me(&session),
        w.me(&session),
        w.me(&session),
        w.me(&session),
        w.me(&session),
        w.me(&session)
    );
    for res in [a, b, c, d, e, f] {
        assert_eq!(res.status(), StatusCode::OK);
    }
    // A second refresh with the rotated-away token would have been invalid_grant (T-AUTH-14).
    assert_eq!(w.idp.refresh_calls(), 1);
}

#[sqlx::test]
async fn removal_from_the_group_ends_the_session_at_refresh(pool: PgPool) {
    let w = Web::new(pool).await;
    let session = w.login("jo", false).await;
    w.age_claims(301).await;
    w.idp.set_next_refresh_claims(w.idp.web_claims("jo", false, false));
    assert_eq!(w.me(&session).await.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(w.session_count().await, 0);
    assert_eq!(w.idp.revoked().len(), 1, "the new refresh token is revoked too");
}

#[sqlx::test]
async fn refused_refresh_token_ends_the_session(pool: PgPool) {
    let w = Web::new(pool).await;
    let session = w.login("kim", false).await;
    w.age_claims(301).await;
    w.idp.set_refresh_mode(RefreshMode::InvalidGrant);
    assert_eq!(w.me(&session).await.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(w.session_count().await, 0);
}

#[sqlx::test]
async fn authentik_outage_is_tolerated_for_an_hour_then_fails_closed(pool: PgPool) {
    let w = Web::new(pool).await;
    let session = w.login("lea", false).await;
    w.idp.set_refresh_mode(RefreshMode::Down);
    w.age_claims(301).await;
    assert_eq!(w.me(&session).await.status(), StatusCode::OK);
    w.age_claims(300 + 3600 + 1).await;
    assert_eq!(w.me(&session).await.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        w.session_count().await,
        1,
        "the session survives for when Authentik is back"
    );
}

#[sqlx::test]
async fn logout_revokes_clears_and_points_to_end_session(pool: PgPool) {
    let w = Web::new(pool).await;
    let session = w.login("max", false).await;
    let cookie = format!("__Host-r16a_stream_session={session}");

    let res = w
        .send(
            Method::POST,
            "/api/v1/auth/logout",
            &[("cookie", cookie.as_str())],
            None,
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "logout needs the CSRF header too");

    let headers = [("cookie", cookie.as_str()), ("x-r16a-request", "1"), ("origin", ORIGIN)];
    let res = w.send(Method::POST, "/api/v1/auth/logout", &headers, None).await;
    assert_eq!(res.status(), StatusCode::OK);
    let cleared = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .any(|v| v.to_str().unwrap().starts_with("__Host-r16a_stream_session=;"));
    assert!(cleared);
    let body = json(res).await;
    assert_eq!(body["endSessionUrl"], format!("{}end-session/", w.idp.issuer("web")));
    assert_eq!(w.idp.revoked().len(), 1);
    assert_eq!(w.session_count().await, 0);
    assert_eq!(w.me(&session).await.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn unknown_or_malformed_cookies_are_unauthenticated(pool: PgPool) {
    let w = Web::new(pool).await;
    for cookie in [
        format!("__Host-r16a_stream_session={}", "A".repeat(43)),
        String::from("__Host-r16a_stream_session=short"),
    ] {
        let res = w
            .send(Method::GET, "/api/v1/me", &[("cookie", cookie.as_str())], None)
            .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
}

#[sqlx::test]
async fn login_is_rate_limited_per_client_address(pool: PgPool) {
    let w = Web::new(pool).await;
    let mut limited = None;
    for _ in 0..12 {
        let res = w.send(Method::GET, "/api/v1/auth/login", &[], None).await;
        if res.status() == StatusCode::TOO_MANY_REQUESTS {
            limited = Some(res);
            break;
        }
    }
    let res = limited.expect("burst of 10 is exceeded");
    assert!(res.headers().contains_key(header::RETRY_AFTER));
    // Another address is not affected. X-Forwarded-For from an untrusted peer is ignored.
    let other = w
        .router([198, 51, 100, 9])
        .oneshot(Request::get("/api/v1/auth/login").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(other.status(), StatusCode::FOUND);
    let spoofed = w
        .router([192, 0, 2, 1])
        .oneshot(
            Request::get("/api/v1/auth/login")
                .header("x-forwarded-for", "203.0.113.50")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(spoofed.status(), StatusCode::TOO_MANY_REQUESTS);
}

fn setup_reference(res: &Response) -> String {
    let location = res.headers()[header::LOCATION].to_str().unwrap().to_owned();
    let reference = location
        .strip_prefix("/signin?error=setup&ref=")
        .unwrap_or_else(|| panic!("expected a setup error, got {location}"));
    assert_eq!(reference.len(), 32, "the request id, nothing about the setting");
    assert!(reference.bytes().all(|b| b.is_ascii_hexdigit()));
    reference.to_owned()
}

#[sqlx::test]
async fn missing_scope_mappings_stop_the_login_before_authentik(pool: PgPool) {
    let w = Web::new(pool).await;
    w.idp.set_scopes(&["openid", "r16a-stream"]);
    let res = w.send(Method::GET, "/api/v1/auth/login", &[], None).await;
    setup_reference(&res);
    let flows: i64 = sqlx::query_scalar("select count(*) from oidc_flows")
        .fetch_one(&w.state.db)
        .await
        .unwrap();
    assert_eq!(flows, 0);
}

#[sqlx::test]
async fn no_refresh_token_is_a_setup_error(pool: PgPool) {
    // Authentik answers without a refresh token when offline_access is advertised but the
    // mapping does not apply, the failure the first real login hit.
    let w = Web::new(pool).await;
    w.idp.set_issue_refresh(false);
    let (authorize, flow) = w.start("/").await;
    let mut claims = w.idp.web_claims("ann", true, false);
    claims["nonce"] = json!(param(&authorize, "nonce"));
    w.idp.add_code("c", claims);
    let res = w.callback("c", &param(&authorize, "state"), Some(&flow)).await;
    setup_reference(&res);
    assert_eq!(w.session_count().await, 0);
}

#[sqlx::test]
async fn missing_access_claim_is_a_setup_error_not_a_denial(pool: PgPool) {
    let w = Web::new(pool).await;
    let (authorize, flow) = w.start("/").await;
    let mut claims = w.idp.web_claims("ben", true, false);
    let o = claims.as_object_mut().unwrap();
    o.remove("r16a_stream_user");
    o.remove("r16a_stream_admin");
    claims["nonce"] = json!(param(&authorize, "nonce"));
    w.idp.add_code("c", claims);
    let res = w.callback("c", &param(&authorize, "state"), Some(&flow)).await;
    setup_reference(&res);
    assert_eq!(w.session_count().await, 0);
    assert_eq!(w.idp.revoked().len(), 1, "the refresh token is not kept");
}

#[sqlx::test]
async fn a_wrong_client_secret_does_not_sign_everyone_out(pool: PgPool) {
    let w = Web::new(pool).await;
    let session = w.login("cleo", false).await;
    w.age_claims(301).await;
    w.idp.set_refresh_mode(RefreshMode::InvalidClient);
    assert_eq!(w.me(&session).await.status(), StatusCode::OK);
    assert_eq!(w.session_count().await, 1);
}

#[sqlx::test]
async fn rate_limited_refresh_keeps_the_session_and_backs_off(pool: PgPool) {
    let w = Web::new(pool).await;
    let session = w.login("dina", false).await;
    w.age_claims(301).await;
    w.idp.set_refresh_mode(RefreshMode::RateLimited);
    assert_eq!(w.me(&session).await.status(), StatusCode::OK);
    assert_eq!(w.me(&session).await.status(), StatusCode::OK);
    assert_eq!(
        w.idp.refresh_calls(),
        1,
        "the lease makes the session wait before retrying"
    );
    assert_eq!(w.session_count().await, 1);
}
