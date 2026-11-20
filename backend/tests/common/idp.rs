//! A mock Authentik for the auth tests.
//!
//! It serves discovery, JWKS, token and revocation endpoints in the shape Authentik 2026.2.1
//! uses (SPEC 6.1), and signs RS256 tokens with a key generated once per test binary, so no
//! private key is ever committed (DECISIONS 070). Tests script it, register a code, choose how
//! the next refresh answers, and read what was revoked.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::{pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};
use serde_json::{Value, json};

pub const WEB_CLIENT_ID: &str = "r16a-stream-test";
pub const MOBILE_CLIENT_ID: &str = "r16a-stream-mobile-test";
pub const KID: &str = "test-key";

struct Keys {
    signing: EncodingKey,
    /// A second key that the JWKS never lists, for forged tokens.
    foreign: EncodingKey,
    n: String,
    e: String,
}

fn keys() -> &'static Keys {
    static KEYS: OnceLock<Keys> = OnceLock::new();
    KEYS.get_or_init(|| {
        let gen_key = || rsa::PrivateKey::new(&mut rand::rngs::SysRng, 2048).expect("rsa key");
        let k = gen_key();
        let foreign = gen_key();
        let der = |k: &rsa::PrivateKey| EncodingKey::from_rsa_der(k.to_pkcs1_der().expect("der").as_bytes());
        Keys {
            signing: der(&k),
            foreign: der(&foreign),
            n: URL_SAFE_NO_PAD.encode(k.n_bytes()),
            e: URL_SAFE_NO_PAD.encode(k.e_bytes()),
        }
    })
}

/// How the token endpoint answers the next refresh grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshMode {
    Ok,
    InvalidGrant,
    Down,
}

struct Inner {
    base: String,
    codes: Mutex<HashMap<String, Value>>,
    refresh_tokens: Mutex<HashMap<String, Value>>,
    next_refresh_claims: Mutex<Option<Value>>,
    refresh_mode: Mutex<RefreshMode>,
    refresh_calls: AtomicUsize,
    jwks_fetches: AtomicUsize,
    revoked: Mutex<Vec<String>>,
    last_token_auth: Mutex<Option<String>>,
}

#[derive(Clone)]
pub struct MockIdp {
    inner: Arc<Inner>,
}

impl MockIdp {
    pub async fn start() -> Self {
        let _ = keys();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let inner = Arc::new(Inner {
            base,
            codes: Mutex::default(),
            refresh_tokens: Mutex::default(),
            next_refresh_claims: Mutex::default(),
            refresh_mode: Mutex::new(RefreshMode::Ok),
            refresh_calls: AtomicUsize::new(0),
            jwks_fetches: AtomicUsize::new(0),
            revoked: Mutex::default(),
            last_token_auth: Mutex::default(),
        });
        let app = Router::new()
            .route("/application/o/{slug}/.well-known/openid-configuration", get(discovery))
            .route("/application/o/{slug}/jwks/", get(jwks))
            .route("/application/o/token/", post(token))
            .route("/application/o/revoke/", post(revoke))
            .with_state(inner.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock idp");
        });
        Self { inner }
    }

    pub fn issuer(&self, slug: &str) -> String {
        format!("{}/application/o/{slug}/", self.inner.base)
    }

    /// Test config pointing both providers at this mock.
    pub fn vars(&self, public_url: &str) -> HashMap<&'static str, String> {
        let mut v = super::test_vars(public_url, "http://127.0.0.1:9");
        v.insert("R16A_OIDC_ISSUER", self.issuer("web"));
        v.insert("R16A_OIDC_MOBILE_ISSUER", self.issuer("mobile"));
        v
    }

    /// Claims as Authentik puts them in an ID token of the web provider.
    pub fn web_claims(&self, sub: &str, user: bool, admin: bool) -> Value {
        let now = now();
        json!({
            "iss": self.issuer("web"), "aud": WEB_CLIENT_ID, "azp": WEB_CLIENT_ID, "sub": sub,
            "iat": now, "exp": now + 300, "name": format!("Name {sub}"), "preferred_username": sub,
            "r16a_stream_user": user, "r16a_stream_admin": admin,
        })
    }

    /// Claims of a mobile access token, which unlike an ID token carry `scope`.
    pub fn mobile_claims(&self, sub: &str, user: bool, admin: bool) -> Value {
        let now = now();
        json!({
            "iss": self.issuer("mobile"), "aud": MOBILE_CLIENT_ID, "azp": MOBILE_CLIENT_ID, "sub": sub,
            "iat": now, "exp": now + 300, "name": format!("Name {sub}"), "preferred_username": sub,
            "scope": "openid profile offline_access r16a-stream",
            "r16a_stream_user": user, "r16a_stream_admin": admin,
        })
    }

    pub fn mobile_token(&self, sub: &str, user: bool, admin: bool) -> String {
        sign(&self.mobile_claims(sub, user, admin))
    }

    /// Registers an authorization code that redeems to an ID token with these claims.
    pub fn add_code(&self, code: &str, claims: Value) {
        self.inner.codes.lock().unwrap().insert(code.into(), claims);
    }

    pub fn set_refresh_mode(&self, mode: RefreshMode) {
        *self.inner.refresh_mode.lock().unwrap() = mode;
    }

    /// Claims of the ID token the next successful refresh returns.
    pub fn set_next_refresh_claims(&self, claims: Value) {
        *self.inner.next_refresh_claims.lock().unwrap() = Some(claims);
    }

    pub fn refresh_calls(&self) -> usize {
        self.inner.refresh_calls.load(Ordering::SeqCst)
    }

    pub fn jwks_fetches(&self) -> usize {
        self.inner.jwks_fetches.load(Ordering::SeqCst)
    }

    pub fn revoked(&self) -> Vec<String> {
        self.inner.revoked.lock().unwrap().clone()
    }

    pub fn last_token_auth(&self) -> Option<String> {
        self.inner.last_token_auth.lock().unwrap().clone()
    }
}

pub fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

/// Signs with the key the JWKS publishes.
pub fn sign(claims: &Value) -> String {
    let mut h = Header::new(Algorithm::RS256);
    h.kid = Some(KID.into());
    jsonwebtoken::encode(&h, claims, &keys().signing).expect("sign")
}

/// Signs with a key the JWKS does not publish, under the published key id.
pub fn sign_foreign(claims: &Value) -> String {
    let mut h = Header::new(Algorithm::RS256);
    h.kid = Some(KID.into());
    jsonwebtoken::encode(&h, claims, &keys().foreign).expect("sign")
}

/// Signs under a key id the JWKS does not list.
pub fn sign_with_kid(claims: &Value, kid: &str) -> String {
    let mut h = Header::new(Algorithm::RS256);
    h.kid = Some(kid.into());
    jsonwebtoken::encode(&h, claims, &keys().signing).expect("sign")
}

async fn discovery(State(s): State<Arc<Inner>>, Path(slug): Path<String>) -> Json<Value> {
    let b = &s.base;
    Json(json!({
        "issuer": format!("{b}/application/o/{slug}/"),
        "authorization_endpoint": format!("{b}/application/o/authorize/"),
        "token_endpoint": format!("{b}/application/o/token/"),
        "jwks_uri": format!("{b}/application/o/{slug}/jwks/"),
        "revocation_endpoint": format!("{b}/application/o/revoke/"),
        "end_session_endpoint": format!("{b}/application/o/{slug}/end-session/"),
    }))
}

async fn jwks(State(s): State<Arc<Inner>>) -> Json<Value> {
    s.jwks_fetches.fetch_add(1, Ordering::SeqCst);
    let k = keys();
    Json(json!({ "keys": [{ "kty": "RSA", "alg": "RS256", "use": "sig", "kid": KID, "n": k.n, "e": k.e }] }))
}

fn form(body: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(body.as_bytes()).into_owned().collect()
}

fn invalid_grant() -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": "invalid_grant" }))).into_response()
}

fn issue(s: &Inner, mut claims: Value) -> Response {
    let rt = format!("rt-{}", uuid::Uuid::new_v4().simple());
    let now = now();
    claims["iat"] = json!(now);
    claims["exp"] = json!(now + 300);
    s.refresh_tokens.lock().unwrap().insert(rt.clone(), claims.clone());
    Json(json!({
        "access_token": "opaque-for-the-bff",
        "token_type": "Bearer",
        "expires_in": 300,
        "id_token": sign(&claims),
        "refresh_token": rt,
    }))
    .into_response()
}

async fn token(State(s): State<Arc<Inner>>, headers: axum::http::HeaderMap, body: String) -> Response {
    *s.last_token_auth.lock().unwrap() = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let f = form(&body);
    match f.get("grant_type").map(String::as_str) {
        Some("authorization_code") => {
            if f.get("code_verifier").is_none_or(|v| v.len() < 43) {
                return invalid_grant();
            }
            let Some(claims) = f.get("code").and_then(|c| s.codes.lock().unwrap().remove(c)) else {
                return invalid_grant();
            };
            issue(&s, claims)
        }
        Some("refresh_token") => {
            s.refresh_calls.fetch_add(1, Ordering::SeqCst);
            match *s.refresh_mode.lock().unwrap() {
                RefreshMode::Down => return StatusCode::BAD_GATEWAY.into_response(),
                RefreshMode::InvalidGrant => return invalid_grant(),
                RefreshMode::Ok => {}
            }
            // Rotation as in Authentik with threshold 0, the old token is gone after one use.
            let Some(old) = f
                .get("refresh_token")
                .and_then(|t| s.refresh_tokens.lock().unwrap().remove(t))
            else {
                return invalid_grant();
            };
            let mut claims = s.next_refresh_claims.lock().unwrap().take().unwrap_or(old);
            if let Some(o) = claims.as_object_mut() {
                o.remove("nonce");
            }
            issue(&s, claims)
        }
        _ => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "unsupported_grant_type" })),
        )
            .into_response(),
    }
}

async fn revoke(State(s): State<Arc<Inner>>, body: String) -> StatusCode {
    if let Some(t) = form(&body).get("token") {
        s.revoked.lock().unwrap().push(t.clone());
        s.refresh_tokens.lock().unwrap().remove(t);
    }
    StatusCode::OK
}
