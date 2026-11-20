//! A minimal OIDC client for Authentik (DECISIONS 070).
//!
//! It covers what the BFF and the bearer check need and nothing more. Discovery, the JWKS
//! cache, the authorization code exchange with PKCE, refresh, revocation, and verification of
//! RS256 JWTs. Signature checks are done by `jsonwebtoken`. Everything that is a protocol rule
//! (state, nonce, PKCE, `azp`, `scope`, the two access claims) is checked here and in the
//! session service, with tests against a mock provider in `tests/auth_*.rs`.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, jwk::JwkSet};
use reqwest::{Client, StatusCode, header};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, de::DeserializeOwned};
use tokio::sync::{Mutex, RwLock};
use url::Url;

/// Largest response body accepted from Authentik. Discovery, JWKS and token responses are a
/// few KiB, so anything larger is a fault or an attack and is refused before it is buffered.
const MAX_BODY: usize = 256 * 1024;
/// Unknown key ids trigger at most one JWKS fetch per minute (T-AUTH-10).
const JWKS_REFETCH_INTERVAL: Duration = Duration::from_secs(60);
/// Clock skew accepted on `exp`, `nbf` and `iat`.
const LEEWAY_SECS: u64 = 60;

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    /// Authentik could not be reached or answered with a server error. Callers keep state and
    /// may retry, nothing about the user is known.
    #[error("identity provider unavailable, {0}")]
    Unavailable(String),
    /// The refresh token or code was refused. The session it belonged to is over.
    #[error("grant refused")]
    InvalidGrant,
    /// The provider answered with something that breaks the protocol.
    #[error("identity provider protocol error, {0}")]
    Protocol(String),
    /// A token failed verification. The reason is for logs only.
    #[error("token rejected, {0}")]
    Rejected(String),
}

/// Endpoints from the discovery document.
#[derive(Debug, Clone, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: Url,
    pub token_endpoint: Url,
    pub jwks_uri: Url,
    #[serde(default)]
    pub revocation_endpoint: Option<Url>,
    #[serde(default)]
    pub end_session_endpoint: Option<Url>,
}

/// Claims read from Authentik's ID and access tokens. Both access claims default to false when
/// missing, so a broken scope mapping locks people out instead of letting them in. A claim of
/// the wrong JSON type fails deserialisation, which also refuses the token.
#[derive(Debug, Clone, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub iat: i64,
    #[serde(default)]
    pub nonce: Option<String>,
    #[serde(default)]
    pub azp: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub preferred_username: Option<String>,
    #[serde(default)]
    pub r16a_stream_user: bool,
    #[serde(default)]
    pub r16a_stream_admin: bool,
}

/// Token endpoint response. Every field is a credential, so `Debug` prints nothing of them.
#[derive(Deserialize)]
pub struct TokenResponse {
    pub id_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_type: String,
}

impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenResponse([redacted])")
    }
}

/// Error body of the token endpoint, RFC 6749 section 5.2.
#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

#[derive(Default)]
struct Jwks {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Option<Instant>,
}

/// One Authentik provider, identified by its issuer and client id.
pub struct Provider {
    issuer: Url,
    client_id: String,
    client_secret: Option<SecretString>,
    http: Client,
    discovery: RwLock<Option<Discovery>>,
    jwks: RwLock<Jwks>,
    /// Serialises JWKS fetches, so a burst of unknown key ids causes one request, not many.
    refetch: Mutex<()>,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("issuer", &self.issuer.as_str())
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

/// Builds the HTTP client used for every call to Authentik. Redirects are not followed, so a
/// compromised or misconfigured endpoint cannot bounce the client secret to another host.
pub fn http_client(timeout: Duration, https_only: bool) -> Result<Client, reqwest::Error> {
    Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .https_only(https_only)
        .user_agent("r16a-stream")
        .build()
}

impl Provider {
    pub fn new(issuer: Url, client_id: String, client_secret: Option<SecretString>, http: Client) -> Self {
        Self {
            issuer,
            client_id,
            client_secret,
            http,
            discovery: RwLock::new(None),
            jwks: RwLock::new(Jwks::default()),
            refetch: Mutex::new(()),
        }
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The discovery document, fetched on first use and then kept for the life of the process.
    /// Endpoints of an Authentik provider do not move without a reconfiguration, which comes
    /// with a restart.
    pub async fn discovery(&self) -> Result<Discovery, OidcError> {
        if let Some(d) = self.discovery.read().await.as_ref() {
            return Ok(d.clone());
        }
        let mut guard = self.discovery.write().await;
        if let Some(d) = guard.as_ref() {
            return Ok(d.clone());
        }
        let base = self.issuer.as_str().trim_end_matches('/');
        let url = Url::parse(&format!("{base}/.well-known/openid-configuration"))
            .map_err(|_| OidcError::Protocol("discovery url".into()))?;
        let doc: Discovery = self.get_json(url).await?;
        // The document must describe the configured issuer, otherwise tokens it vouches for
        // could come from another provider.
        if Url::parse(&doc.issuer).ok().as_ref() != Some(&self.issuer) {
            return Err(OidcError::Protocol(
                "discovery issuer does not match the configured issuer".into(),
            ));
        }
        let scheme = self.issuer.scheme();
        for endpoint in [
            Some(&doc.authorization_endpoint),
            Some(&doc.token_endpoint),
            Some(&doc.jwks_uri),
            doc.revocation_endpoint.as_ref(),
            doc.end_session_endpoint.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if endpoint.scheme() != scheme {
                return Err(OidcError::Protocol("discovery endpoint downgrades the scheme".into()));
            }
        }
        *guard = Some(doc.clone());
        Ok(doc)
    }

    /// Verifies an RS256 JWT from this provider. Checks signature, `iss`, `aud`, `exp`, `nbf`
    /// with 60 s leeway, and that `iat` is not in the future. Callers check the rest.
    pub async fn verify(&self, token: &str) -> Result<Claims, OidcError> {
        let header = jsonwebtoken::decode_header(token).map_err(|e| OidcError::Rejected(format!("header {e}")))?;
        if header.alg != Algorithm::RS256 {
            return Err(OidcError::Rejected("algorithm not allowed".into()));
        }
        let kid = header.kid.ok_or_else(|| OidcError::Rejected("no key id".into()))?;
        let discovery = self.discovery().await?;
        let key = self.key(&discovery, &kid).await?;

        let mut v = Validation::new(Algorithm::RS256);
        v.algorithms = vec![Algorithm::RS256];
        v.leeway = LEEWAY_SECS;
        v.validate_nbf = true;
        v.set_issuer(&[discovery.issuer.as_str()]);
        v.set_audience(&[self.client_id.as_str()]);
        v.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        let data = jsonwebtoken::decode::<Claims>(token, &key, &v)
            .map_err(|e| OidcError::Rejected(format!("{:?}", e.kind())))?;
        let claims = data.claims;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        if claims.iat > now.saturating_add(i64::try_from(LEEWAY_SECS).unwrap_or(60)) {
            return Err(OidcError::Rejected("issued in the future".into()));
        }
        if claims.sub.is_empty() || claims.sub.len() > 255 {
            return Err(OidcError::Rejected("subject length".into()));
        }
        Ok(claims)
    }

    async fn key(&self, discovery: &Discovery, kid: &str) -> Result<DecodingKey, OidcError> {
        if let Some(k) = self.jwks.read().await.keys.get(kid) {
            return Ok(k.clone());
        }
        let _single = self.refetch.lock().await;
        // Another request may have fetched while this one waited.
        {
            let jwks = self.jwks.read().await;
            if let Some(k) = jwks.keys.get(kid) {
                return Ok(k.clone());
            }
            if jwks.fetched_at.is_some_and(|t| t.elapsed() < JWKS_REFETCH_INTERVAL) {
                return Err(OidcError::Rejected("unknown key id".into()));
            }
        }
        let set: JwkSet = self.get_json(discovery.jwks_uri.clone()).await?;
        let mut keys = HashMap::new();
        for jwk in &set.keys {
            let Some(id) = jwk.common.key_id.clone() else { continue };
            if !matches!(jwk.algorithm, jsonwebtoken::jwk::AlgorithmParameters::RSA(_)) {
                continue;
            }
            if let Ok(k) = DecodingKey::from_jwk(jwk) {
                keys.insert(id, k);
            }
        }
        let found = keys.get(kid).cloned();
        *self.jwks.write().await = Jwks {
            keys,
            fetched_at: Some(Instant::now()),
        };
        found.ok_or_else(|| OidcError::Rejected("unknown key id".into()))
    }

    /// Redeems an authorization code with the PKCE verifier.
    pub async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<TokenResponse, OidcError> {
        self.token_request(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
        ])
        .await
    }

    /// Uses a refresh token. Authentik rotates it, the response carries the only valid one.
    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, OidcError> {
        self.token_request(&[("grant_type", "refresh_token"), ("refresh_token", refresh_token)])
            .await
    }

    /// Revokes a refresh token, RFC 7009. Failure is reported but callers treat it as best
    /// effort, the session row is already gone.
    pub async fn revoke(&self, refresh_token: &str) -> Result<(), OidcError> {
        let discovery = self.discovery().await?;
        let Some(endpoint) = discovery.revocation_endpoint else {
            return Ok(());
        };
        let res = self
            .post_form(
                endpoint,
                &[("token", refresh_token), ("token_type_hint", "refresh_token")],
            )
            .await?;
        if res.status().is_success() {
            Ok(())
        } else {
            Err(OidcError::Protocol(format!("revocation answered {}", res.status())))
        }
    }

    async fn token_request(&self, form: &[(&str, &str)]) -> Result<TokenResponse, OidcError> {
        let discovery = self.discovery().await?;
        let res = self.post_form(discovery.token_endpoint, form).await?;
        let status = res.status();
        let body = read_capped(res).await?;
        if status.is_success() {
            let t: TokenResponse =
                serde_json::from_slice(&body).map_err(|_| OidcError::Protocol("token response".into()))?;
            if !t.token_type.eq_ignore_ascii_case("bearer") {
                return Err(OidcError::Protocol("token type".into()));
            }
            return Ok(t);
        }
        if status.is_server_error() {
            return Err(OidcError::Unavailable(format!("token endpoint answered {status}")));
        }
        match serde_json::from_slice::<ErrorBody>(&body) {
            Ok(e) if e.error == "invalid_grant" => Err(OidcError::InvalidGrant),
            Ok(e) => Err(OidcError::Protocol(format!(
                "token endpoint error {}",
                e.error
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .take(40)
                    .collect::<String>()
            ))),
            Err(_) => Err(OidcError::Protocol(format!("token endpoint answered {status}"))),
        }
    }

    /// Posts a form with `client_secret_basic` when this provider has a secret. The body is
    /// built here because reqwest is compiled without its form feature.
    async fn post_form(&self, url: Url, form: &[(&str, &str)]) -> Result<reqwest::Response, OidcError> {
        // Built in a block, the serializer is not Send and must not live across the await.
        let body = {
            let mut ser = url::form_urlencoded::Serializer::new(String::new());
            for (k, v) in form {
                ser.append_pair(k, v);
            }
            if self.client_secret.is_none() {
                ser.append_pair("client_id", &self.client_id);
            }
            ser.finish()
        };
        let mut req = self
            .http
            .post(url)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::ACCEPT, "application/json")
            .body(body);
        if let Some(secret) = &self.client_secret {
            // RFC 6749 section 2.3.1 wants both parts form-encoded before base64.
            let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
            let basic = STANDARD.encode(format!("{}:{}", enc(&self.client_id), enc(secret.expose_secret())));
            req = req.header(header::AUTHORIZATION, format!("Basic {basic}"));
        }
        req.send().await.map_err(|e| OidcError::Unavailable(transport(&e)))
    }

    async fn get_json<T: DeserializeOwned>(&self, url: Url) -> Result<T, OidcError> {
        let res = self
            .http
            .get(url)
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| OidcError::Unavailable(transport(&e)))?;
        let status = res.status();
        if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(OidcError::Unavailable(format!("answered {status}")));
        }
        if !status.is_success() {
            return Err(OidcError::Protocol(format!("answered {status}")));
        }
        let body = read_capped(res).await?;
        serde_json::from_slice(&body).map_err(|_| OidcError::Protocol("unexpected json".into()))
    }
}

/// Describes a transport failure without the URL, which reqwest includes in its `Display`.
fn transport(e: &reqwest::Error) -> String {
    let kind = if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connect"
    } else if e.is_redirect() {
        "redirect"
    } else {
        "transport"
    };
    kind.to_owned()
}

async fn read_capped(mut res: reqwest::Response) -> Result<Vec<u8>, OidcError> {
    if res.content_length().is_some_and(|l| l > MAX_BODY as u64) {
        return Err(OidcError::Protocol("response too large".into()));
    }
    let mut out = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|e| OidcError::Unavailable(transport(&e)))? {
        if out.len() + chunk.len() > MAX_BODY {
            return Err(OidcError::Protocol("response too large".into()));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// The two Authentik providers the API talks to.
#[derive(Debug)]
pub struct Providers {
    /// Confidential client for the BFF, holds the client secret.
    pub web: Provider,
    /// Public client of the app, used only to verify its access tokens.
    pub mobile: Provider,
}
