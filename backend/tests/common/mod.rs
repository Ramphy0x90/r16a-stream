//! Helpers shared by the integration tests.
//!
//! Tests that need running infrastructure read its address from an environment variable. When
//! the variable is missing they skip with a notice, unless `R16A_REQUIRE_INTEGRATION=1` is set,
//! which CI does, so a missing service fails the run instead of silently skipping.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

pub mod idp;
pub mod tmdb;

use std::{
    collections::HashMap,
    io::Write,
    sync::{Arc, Mutex},
};

use r16a_stream_backend::{
    auth::AuthState,
    config::ApiConfig,
    state::AppState,
    storage::{GarageClient, probe::WriteHealth},
};
use sqlx::PgPool;

/// Published dev-only credentials from `deploy/compose/docker-compose.dev.yml`.
pub const API_KEY_ID: &str = "GKe781357014cf1870ce72ffb3";
pub const API_SECRET: &str = "9be2320de5bf8e5b07297443f66bcb7ac546fee5bf2c3f91d7c070e385a43d99";
pub const WORKER_KEY_ID: &str = "GKb21e6a01028af364e9ab7821";
pub const WORKER_SECRET: &str = "115f67a9ad320bb660d52232aad8eeb83be06e0f5ca9accae166c5572af89c69";
pub const METRICS_TOKEN: &str = "dev-only-metrics-f74cb949c7b1a28ba6b29daef067730a5a8d04f9770eaa03";

/// Returns the variable, skips the test when it is missing, or fails when integration is required.
pub fn integration_env(name: &str) -> Option<String> {
    r16a_stream_backend::install_crypto_provider();
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Some(v),
        _ if std::env::var("R16A_REQUIRE_INTEGRATION").as_deref() == Ok("1") => {
            panic!("{name} is required because R16A_REQUIRE_INTEGRATION=1")
        }
        _ => {
            eprintln!("skipping, {name} is not set");
            None
        }
    }
}

pub fn test_vars(public_url: &str, s3_endpoint: &str) -> HashMap<&'static str, String> {
    HashMap::from([
        ("R16A_ENV", "test".into()),
        ("R16A_PUBLIC_URL", public_url.into()),
        (
            "R16A_DATABASE_URL",
            "postgres://r16a_stream_app:dev-only-app-password@127.0.0.1:5433/r16a_stream".into(),
        ),
        ("R16A_S3_INTERNAL_ENDPOINTS", s3_endpoint.into()),
        ("R16A_S3_PUBLIC_ENDPOINT", public_url.into()),
        ("R16A_S3_ACCESS_KEY_ID", API_KEY_ID.into()),
        ("R16A_S3_SECRET_ACCESS_KEY", API_SECRET.into()),
        ("R16A_S3_CONNECT_TIMEOUT_MS", "300".into()),
        ("R16A_S3_TIMEOUT_MS", "2000".into()),
        // Point at a closed port unless a test starts the mock Authentik and overrides them.
        ("R16A_OIDC_ISSUER", "http://127.0.0.1:9/application/o/web/".into()),
        ("R16A_OIDC_WEB_CLIENT_ID", idp::WEB_CLIENT_ID.into()),
        (
            "R16A_OIDC_WEB_CLIENT_SECRET",
            "test-web-client-secret-5b1f0c2e9d7a4f38".into(),
        ),
        (
            "R16A_OIDC_MOBILE_ISSUER",
            "http://127.0.0.1:9/application/o/mobile/".into(),
        ),
        ("R16A_OIDC_MOBILE_CLIENT_ID", idp::MOBILE_CLIENT_ID.into()),
        ("R16A_OIDC_TIMEOUT_MS", "2000".into()),
        (
            "R16A_SESSION_KEY",
            "BwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyAhIiMkJSY=".into(),
        ),
        // Closed port unless a test starts the mock TMDB.
        ("R16A_TMDB_TOKEN", "test-tmdb-read-token-9c41e07b2d5a8f36".into()),
        ("R16A_TMDB_BASE_URL", "http://127.0.0.1:9/3/".into()),
    ])
}

pub fn config_from(vars: &HashMap<&'static str, String>) -> ApiConfig {
    r16a_stream_backend::install_crypto_provider();
    ApiConfig::from_lookup(&|n| vars.get(n).cloned()).expect("test config")
}

/// App state around a pool. Garage points at a closed port unless a test passes a real one.
pub fn state(pool: PgPool, public_url: &str) -> AppState {
    state_with(pool, &test_vars(public_url, "http://127.0.0.1:9"))
}

pub fn state_with(pool: PgPool, vars: &HashMap<&'static str, String>) -> AppState {
    let cfg = config_from(vars);
    let storage = GarageClient::new(&cfg.s3).expect("storage client");
    let auth = AuthState::new(&cfg).expect("auth state");
    let tmdb = r16a_stream_backend::tmdb::client::TmdbClient::new(&cfg.tmdb, false).expect("tmdb client");
    let tmdb_images =
        r16a_stream_backend::images::ingest::TmdbImages::new(cfg.tmdb.image_base_url.clone(), false).expect("images");
    let image_urls =
        r16a_stream_backend::images::urls::ImageUrls::new(&cfg.s3_public_endpoint, &cfg.s3).expect("image urls");
    AppState {
        config: Arc::new(cfg),
        db: pool,
        storage: Arc::new(storage),
        write_health: Arc::new(WriteHealth::default()),
        auth: Arc::new(auth),
        limits: Arc::default(),
        tmdb: Arc::new(tmdb),
        tmdb_images: Arc::new(tmdb_images),
        image_urls: Arc::new(image_urls),
    }
}

/// Collects everything the tracing subscriber writes, so tests can assert what is logged.
#[derive(Clone, Default)]
pub struct LogCapture(pub Arc<Mutex<Vec<u8>>>);

impl LogCapture {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
    type Writer = LogCapture;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
