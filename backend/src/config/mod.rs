//! Configuration from environment variables, validated once at startup.
//!
//! Every value is parsed here so that a bad deployment fails before it serves a request.
//! Errors name the variable and the problem, never the value. Secrets have no defaults and
//! are refused when they are missing, look like a template placeholder, are too repetitive
//! to be random, or carry a published development value outside development.

mod secret;

use std::{net::SocketAddr, time::Duration};

use base64::Engine;
use secrecy::{SecretBox, SecretString};
use url::Url;

use crate::http::client_ip::Cidr;

pub use secret::{DEV_GARAGE_KEY_IDS, DEV_SESSION_KEYS, check_secret};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("{name} is required")]
    Missing { name: &'static str },
    #[error("{name} {reason}")]
    Invalid { name: &'static str, reason: String },
}

impl ConfigError {
    fn invalid(name: &'static str, reason: impl Into<String>) -> Self {
        Self::Invalid {
            name,
            reason: reason.into(),
        }
    }
}

/// Where the process runs. Unset means production, so a forgotten variable is strict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Env {
    Development,
    Test,
    Production,
}

impl Env {
    /// Development and test accept the published dev-only credentials.
    pub fn allows_dev_values(self) -> bool {
        matches!(self, Self::Development | Self::Test)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Json,
    Pretty,
}

/// Settings shared by the API and the worker for their Postgres pool.
#[derive(Debug)]
pub struct DbConfig {
    pub url: SecretString,
    pub max_connections: u32,
    pub acquire_timeout: Duration,
    pub statement_timeout: Duration,
}

/// Garage access for one component, with the key that component is allowed to hold.
#[derive(Debug)]
pub struct S3Config {
    /// One URL per Garage node, tried in order with failover.
    pub internal_endpoints: Vec<Url>,
    pub region: String,
    pub media_bucket: String,
    pub uploads_bucket: String,
    pub access_key_id: String,
    pub secret_access_key: SecretString,
    pub connect_timeout: Duration,
    pub timeout: Duration,
    pub probe_interval: Duration,
}

/// OIDC settings for the web BFF and for mobile bearer tokens (SPEC 6).
#[derive(Debug)]
pub struct AuthConfig {
    /// Issuer of the confidential web provider, `r16a-stream` or `r16a-stream-dev`.
    pub web_issuer: Url,
    pub web_client_id: String,
    pub web_client_secret: SecretString,
    /// Expected `iss` and `aud` of mobile access tokens.
    pub mobile_issuer: Url,
    pub mobile_client_id: String,
    pub timeout: Duration,
    /// Seals refresh tokens and PKCE verifiers at rest.
    pub session_key: SecretBox<[u8; 32]>,
    /// The key before the last rotation, still accepted for opening until its rows expire.
    pub session_key_previous: Option<SecretBox<[u8; 32]>>,
    /// Peers whose `X-Forwarded-For` is believed. Empty trusts nobody.
    pub trusted_proxies: Vec<Cidr>,
}

#[derive(Debug)]
pub struct ApiConfig {
    pub env: Env,
    pub log_format: LogFormat,
    /// Browser-visible origin, used for CSRF, redirects and cookies.
    pub public_url: Url,
    pub http_bind: SocketAddr,
    pub request_timeout: Duration,
    pub dev_insecure_cookie: bool,
    pub db: DbConfig,
    pub s3: S3Config,
    /// What presigned URLs are signed for. Always the app origin, so the browser stays same-origin.
    pub s3_public_endpoint: Url,
    pub auth: AuthConfig,
}

#[derive(Debug)]
pub struct WorkerConfig {
    pub env: Env,
    pub log_format: LogFormat,
    pub health_bind: SocketAddr,
    pub db: DbConfig,
    pub s3: S3Config,
    pub scratch_dir: std::path::PathBuf,
}

/// Reads one variable. Empty strings count as missing, because orchestrators often render an
/// unset value as an empty one.
type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

fn get(lookup: Lookup<'_>, name: &str) -> Option<String> {
    lookup(name).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

fn required(lookup: Lookup<'_>, name: &'static str) -> Result<String, ConfigError> {
    get(lookup, name).ok_or(ConfigError::Missing { name })
}

fn parse_or<T: std::str::FromStr>(lookup: Lookup<'_>, name: &'static str, default: T) -> Result<T, ConfigError> {
    match get(lookup, name) {
        None => Ok(default),
        Some(v) => v.parse().map_err(|_| ConfigError::invalid(name, "could not be parsed")),
    }
}

fn millis(lookup: Lookup<'_>, name: &'static str, default: u64) -> Result<Duration, ConfigError> {
    let ms: u64 = parse_or(lookup, name, default)?;
    if ms == 0 {
        return Err(ConfigError::invalid(
            name,
            "must be greater than zero, every outbound call needs a timeout",
        ));
    }
    Ok(Duration::from_millis(ms))
}

fn url(lookup: Lookup<'_>, name: &'static str) -> Result<Url, ConfigError> {
    let raw = required(lookup, name)?;
    let parsed = Url::parse(&raw).map_err(|_| ConfigError::invalid(name, "is not a valid URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(ConfigError::invalid(name, "must be an http or https URL with a host"));
    }
    Ok(parsed)
}

fn env_of(lookup: Lookup<'_>) -> Result<Env, ConfigError> {
    match get(lookup, "R16A_ENV").as_deref() {
        None | Some("production") => Ok(Env::Production),
        Some("development") => Ok(Env::Development),
        Some("test") => Ok(Env::Test),
        Some(_) => Err(ConfigError::invalid(
            "R16A_ENV",
            "must be development, test or production",
        )),
    }
}

fn log_format(lookup: Lookup<'_>, env: Env) -> Result<LogFormat, ConfigError> {
    match get(lookup, "R16A_LOG_FORMAT").as_deref() {
        None if env == Env::Production => Ok(LogFormat::Json),
        None | Some("pretty") => Ok(LogFormat::Pretty),
        Some("json") => Ok(LogFormat::Json),
        Some(_) => Err(ConfigError::invalid("R16A_LOG_FORMAT", "must be json or pretty")),
    }
}

fn db(lookup: Lookup<'_>, env: Env, name: &'static str) -> Result<DbConfig, ConfigError> {
    let raw = required(lookup, name)?;
    let parsed = Url::parse(&raw).map_err(|_| ConfigError::invalid(name, "is not a valid URL"))?;
    if !matches!(parsed.scheme(), "postgres" | "postgresql") {
        return Err(ConfigError::invalid(name, "must be a postgres URL"));
    }
    // The password is the secret part, the rest of the URL is not.
    check_secret(name, parsed.password().unwrap_or_default(), env, 12)?;
    let max_connections: u32 = parse_or(lookup, "R16A_DB_MAX_CONNECTIONS", 10)?;
    if max_connections == 0 || max_connections > 200 {
        return Err(ConfigError::invalid(
            "R16A_DB_MAX_CONNECTIONS",
            "must be between 1 and 200",
        ));
    }
    Ok(DbConfig {
        url: SecretString::from(raw),
        max_connections,
        acquire_timeout: millis(lookup, "R16A_DB_ACQUIRE_TIMEOUT_MS", 2_000)?,
        statement_timeout: millis(lookup, "R16A_DB_STATEMENT_TIMEOUT_MS", 5_000)?,
    })
}

fn s3(lookup: Lookup<'_>, env: Env, key_var: &'static str, secret_var: &'static str) -> Result<S3Config, ConfigError> {
    let endpoints_raw = required(lookup, "R16A_S3_INTERNAL_ENDPOINTS")?;
    let mut internal_endpoints = Vec::new();
    for part in endpoints_raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let u =
            Url::parse(part).map_err(|_| ConfigError::invalid("R16A_S3_INTERNAL_ENDPOINTS", "holds an invalid URL"))?;
        if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
            return Err(ConfigError::invalid(
                "R16A_S3_INTERNAL_ENDPOINTS",
                "must hold http or https URLs",
            ));
        }
        internal_endpoints.push(u);
    }
    if internal_endpoints.is_empty() {
        return Err(ConfigError::Missing {
            name: "R16A_S3_INTERNAL_ENDPOINTS",
        });
    }

    let access_key_id = required(lookup, key_var)?;
    if !secret::looks_like_garage_key_id(&access_key_id) {
        return Err(ConfigError::invalid(
            key_var,
            "must be a Garage key id, GK followed by 24 hex digits",
        ));
    }
    if !env.allows_dev_values() && DEV_GARAGE_KEY_IDS.contains(&access_key_id.as_str()) {
        return Err(ConfigError::invalid(
            key_var,
            "is the published development key, outside development",
        ));
    }
    let secret_raw = required(lookup, secret_var)?;
    check_secret(secret_var, &secret_raw, env, 32)?;

    let probe_secs: u64 = parse_or(lookup, "R16A_S3_HEALTH_PROBE_SECS", 30)?;
    if !(5..=600).contains(&probe_secs) {
        return Err(ConfigError::invalid(
            "R16A_S3_HEALTH_PROBE_SECS",
            "must be between 5 and 600",
        ));
    }

    Ok(S3Config {
        internal_endpoints,
        region: get(lookup, "R16A_S3_REGION").unwrap_or_else(|| String::from("garage")),
        media_bucket: get(lookup, "R16A_S3_MEDIA_BUCKET").unwrap_or_else(|| String::from("r16a-stream-media")),
        uploads_bucket: get(lookup, "R16A_S3_UPLOADS_BUCKET").unwrap_or_else(|| String::from("r16a-stream-uploads")),
        access_key_id,
        secret_access_key: SecretString::from(secret_raw),
        connect_timeout: millis(lookup, "R16A_S3_CONNECT_TIMEOUT_MS", 1_000)?,
        timeout: millis(lookup, "R16A_S3_TIMEOUT_MS", 10_000)?,
        probe_interval: Duration::from_secs(probe_secs),
    })
}

/// An issuer URL. Production requires https, because the discovery document and the signing
/// keys are trusted only as far as the transport that delivered them.
fn issuer(lookup: Lookup<'_>, env: Env, name: &'static str) -> Result<Url, ConfigError> {
    let parsed = url(lookup, name)?;
    if env == Env::Production && parsed.scheme() != "https" {
        return Err(ConfigError::invalid(name, "must be https in production"));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(ConfigError::invalid(name, "must not carry a query or fragment"));
    }
    Ok(parsed)
}

fn client_id(lookup: Lookup<'_>, name: &'static str) -> Result<String, ConfigError> {
    let raw = required(lookup, name)?;
    if raw.len() > 255 || !raw.chars().all(|c| c.is_ascii_graphic()) {
        return Err(ConfigError::invalid(
            name,
            "must be up to 255 printable ASCII characters",
        ));
    }
    Ok(raw)
}

/// A 32 byte key given as standard base64, for example from `openssl rand -base64 32`.
fn key32(lookup: Lookup<'_>, env: Env, name: &'static str) -> Result<SecretBox<[u8; 32]>, ConfigError> {
    let raw = required(lookup, name)?;
    check_secret(name, &raw, env, 43)?;
    if !env.allows_dev_values() && DEV_SESSION_KEYS.contains(&raw.as_str()) {
        return Err(ConfigError::invalid(
            name,
            "is the published development key, outside development",
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw.as_bytes())
        .map_err(|_| ConfigError::invalid(name, "must be standard base64"))?;
    let key: [u8; 32] = bytes
        .try_into()
        .map_err(|_| ConfigError::invalid(name, "must decode to exactly 32 bytes"))?;
    Ok(SecretBox::new(Box::new(key)))
}

fn cidrs(lookup: Lookup<'_>, name: &'static str) -> Result<Vec<Cidr>, ConfigError> {
    get(lookup, name).map_or_else(
        || Ok(Vec::new()),
        |raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(|p| {
                    p.parse()
                        .map_err(|()| ConfigError::invalid(name, "holds an invalid CIDR"))
                })
                .collect()
        },
    )
}

fn auth(lookup: Lookup<'_>, env: Env) -> Result<AuthConfig, ConfigError> {
    let secret = required(lookup, "R16A_OIDC_WEB_CLIENT_SECRET")?;
    check_secret("R16A_OIDC_WEB_CLIENT_SECRET", &secret, env, 32)?;
    let session_key = key32(lookup, env, "R16A_SESSION_KEY")?;
    let session_key_previous = match get(lookup, "R16A_SESSION_KEY_PREVIOUS") {
        None => None,
        Some(_) => Some(key32(lookup, env, "R16A_SESSION_KEY_PREVIOUS")?),
    };
    Ok(AuthConfig {
        web_issuer: issuer(lookup, env, "R16A_OIDC_ISSUER")?,
        web_client_id: client_id(lookup, "R16A_OIDC_WEB_CLIENT_ID")?,
        web_client_secret: SecretString::from(secret),
        mobile_issuer: issuer(lookup, env, "R16A_OIDC_MOBILE_ISSUER")?,
        mobile_client_id: client_id(lookup, "R16A_OIDC_MOBILE_CLIENT_ID")?,
        timeout: millis(lookup, "R16A_OIDC_TIMEOUT_MS", 5_000)?,
        session_key,
        session_key_previous,
        trusted_proxies: cidrs(lookup, "R16A_TRUSTED_PROXY_CIDRS")?,
    })
}

fn flag(lookup: Lookup<'_>, name: &'static str) -> Result<bool, ConfigError> {
    match get(lookup, name).as_deref() {
        None | Some("false" | "0") => Ok(false),
        Some("true" | "1") => Ok(true),
        Some(_) => Err(ConfigError::invalid(name, "must be true or false")),
    }
}

impl ApiConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(&|name| std::env::var(name).ok())
    }

    pub fn from_lookup(lookup: Lookup<'_>) -> Result<Self, ConfigError> {
        let env = env_of(lookup)?;
        let public_url = url(lookup, "R16A_PUBLIC_URL")?;
        let dev_insecure_cookie = flag(lookup, "R16A_DEV_INSECURE_COOKIE")?;
        // The insecure cookie exists only for plain HTTP on localhost. With an https origin it
        // would silently drop the cookie protections, so the combination is refused.
        if dev_insecure_cookie && public_url.scheme() == "https" {
            return Err(ConfigError::invalid(
                "R16A_DEV_INSECURE_COOKIE",
                "cannot be true when R16A_PUBLIC_URL is https",
            ));
        }
        if dev_insecure_cookie && env == Env::Production {
            return Err(ConfigError::invalid(
                "R16A_DEV_INSECURE_COOKIE",
                "cannot be true in production",
            ));
        }
        if env == Env::Production && public_url.scheme() != "https" {
            return Err(ConfigError::invalid("R16A_PUBLIC_URL", "must be https in production"));
        }
        let http_bind = parse_or(lookup, "R16A_HTTP_BIND", SocketAddr::from(([127, 0, 0, 1], 8081)))?;
        Ok(Self {
            env,
            log_format: log_format(lookup, env)?,
            public_url,
            http_bind,
            request_timeout: millis(lookup, "R16A_HTTP_REQUEST_TIMEOUT_MS", 15_000)?,
            dev_insecure_cookie,
            db: db(lookup, env, "R16A_DATABASE_URL")?,
            s3: s3(lookup, env, "R16A_S3_ACCESS_KEY_ID", "R16A_S3_SECRET_ACCESS_KEY")?,
            s3_public_endpoint: url(lookup, "R16A_S3_PUBLIC_ENDPOINT")?,
            auth: auth(lookup, env)?,
        })
    }
}

impl WorkerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(&|name| std::env::var(name).ok())
    }

    pub fn from_lookup(lookup: Lookup<'_>) -> Result<Self, ConfigError> {
        let env = env_of(lookup)?;
        Ok(Self {
            env,
            log_format: log_format(lookup, env)?,
            health_bind: parse_or(
                lookup,
                "R16A_WORKER_HEALTH_BIND",
                SocketAddr::from(([127, 0, 0, 1], 8082)),
            )?,
            // Its own variable, so a shared backend/.env can never hand the worker the app role.
            db: db(lookup, env, "R16A_WORKER_DATABASE_URL")?,
            s3: s3(
                lookup,
                env,
                "R16A_WORKER_S3_ACCESS_KEY_ID",
                "R16A_WORKER_S3_SECRET_ACCESS_KEY",
            )?,
            scratch_dir: get(lookup, "R16A_WORKER_SCRATCH_DIR")
                .map_or_else(|| std::path::PathBuf::from("./.scratch"), std::path::PathBuf::from),
        })
    }
}

/// Owner URL for the `migrate` subcommand, the only place it is read.
pub fn migrate_database_url(lookup: Lookup<'_>) -> Result<SecretString, ConfigError> {
    let env = env_of(lookup)?;
    let name = "R16A_MIGRATE_DATABASE_URL";
    let raw = required(lookup, name)?;
    let parsed = Url::parse(&raw).map_err(|_| ConfigError::invalid(name, "is not a valid URL"))?;
    check_secret(name, parsed.password().unwrap_or_default(), env, 12)?;
    Ok(SecretString::from(raw))
}

#[cfg(test)]
mod tests;
