//! Authentication. Web BFF login with sessions, mobile bearer tokens, the extractors every
//! handler uses to learn who is calling, and the admin check (SPEC 6).

pub mod cookies;
pub mod crypto;
pub mod extract;
pub mod handlers;
pub mod oidc;
pub mod repository;
pub mod service;

use std::net::IpAddr;

use uuid::Uuid;

use crate::{
    config::{ApiConfig, Env},
    http::{client_ip::Cidr, rate_limit::KeyedLimit},
};
use cookies::CookiePolicy;
use crypto::SessionKeys;
use oidc::{Provider, Providers};

pub use extract::{Admin, WebSession};

/// Who is calling. Built by the extractors, never by handlers.
#[derive(Debug, Clone, Copy)]
pub struct Caller {
    pub user_id: Uuid,
    /// From the web session row or the bearer token of this request, never from `users`.
    pub is_admin: bool,
    /// Set for cookie sessions, `None` for bearer tokens.
    pub session_id: Option<Uuid>,
}

/// Everything authentication needs at request time.
#[derive(Debug)]
pub struct AuthState {
    pub providers: Providers,
    pub keys: SessionKeys,
    pub cookies: CookiePolicy,
    /// The Authentik instance origin. Users are keyed by it and `sub`, so the web and the mobile
    /// provider of one instance resolve to the same account (DECISIONS 071).
    pub account_issuer: String,
    pub redirect_uri: String,
    /// The public origin, compared with the `Origin` header of cookie requests that change state.
    pub public_origin: String,
    pub login_limit: KeyedLimit<IpAddr>,
    pub user_limit: KeyedLimit<Uuid>,
    pub trusted_proxies: Vec<Cidr>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthInitError {
    #[error(transparent)]
    Keys(#[from] crypto::KeyError),
    #[error("could not build the HTTP client for Authentik, {0}")]
    Http(String),
    #[error("R16A_OIDC_ISSUER and R16A_OIDC_MOBILE_ISSUER must belong to the same Authentik instance")]
    IssuerOrigins,
}

impl AuthState {
    pub fn new(cfg: &ApiConfig) -> Result<Self, AuthInitError> {
        let a = &cfg.auth;
        // Subjects are only comparable within one Authentik instance, so both providers must
        // live on the same origin for the shared user key to be sound.
        let origin = a.web_issuer.origin().ascii_serialization();
        if a.mobile_issuer.origin().ascii_serialization() != origin {
            return Err(AuthInitError::IssuerOrigins);
        }
        let http =
            oidc::http_client(a.timeout, cfg.env == Env::Production).map_err(|e| AuthInitError::Http(e.to_string()))?;
        let public_origin = cfg.public_url.origin().ascii_serialization();
        Ok(Self {
            providers: Providers {
                web: Provider::new(
                    a.web_issuer.clone(),
                    a.web_client_id.clone(),
                    Some(a.web_client_secret.clone()),
                    http.clone(),
                ),
                mobile: Provider::new(a.mobile_issuer.clone(), a.mobile_client_id.clone(), None, http),
            },
            keys: SessionKeys::from_config(a)?,
            cookies: CookiePolicy {
                insecure: cfg.dev_insecure_cookie,
            },
            account_issuer: origin,
            redirect_uri: format!("{public_origin}/api/v1/auth/callback"),
            public_origin,
            login_limit: KeyedLimit::per_minute(20, 10),
            user_limit: KeyedLimit::per_minute(600, 100),
            trusted_proxies: a.trusted_proxies.clone(),
        })
    }

    /// Fetches both discovery documents once, so the first login does not pay for it, and logs
    /// any missing or unwanted scope mapping at startup. Failure only logs, the API serves
    /// everything else while Authentik is down.
    pub async fn warm_up(&self) {
        for (name, p) in [("web", &self.providers.web), ("mobile", &self.providers.mobile)] {
            match p.discovery().await {
                Err(e) => tracing::warn!(provider = name, error = %e, "OIDC discovery failed, retried on first use"),
                Ok(d) => {
                    let missing = d.missing_scopes();
                    if !missing.is_empty() {
                        tracing::error!(
                            provider = name,
                            client_id = p.client_id(),
                            missing = %missing.join(", "),
                            "provider lacks required scope mappings, logins will fail (OPERATING 3.1)"
                        );
                    }
                    let unwanted = d.unwanted_scopes();
                    if !unwanted.is_empty() {
                        tracing::warn!(
                            provider = name,
                            client_id = p.client_id(),
                            scopes = %unwanted.join(", "),
                            "provider offers scopes that put group names or email into tokens, remove them (DECISIONS 074)"
                        );
                    }
                }
            }
        }
    }
}
