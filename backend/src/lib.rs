//! r16a-stream backend. One crate, two binaries.
//!
//! `api` serves the HTTP API and the web login (BFF). `worker` claims jobs from Postgres and
//! runs ffmpeg. Both share configuration, error handling, database access and the Garage
//! client from this library. Rules for this crate live in `backend/CLAUDE.md`.

pub mod admin;
pub mod app;
pub mod auth;
pub mod bench;
pub mod capacity;
pub mod catalog;
pub mod config;
pub mod db;
pub mod error;
pub mod health;
pub mod http;
pub mod images;
pub mod jobs;
pub mod me;
pub mod media;
pub mod openapi;
pub mod state;
pub mod storage;
pub mod telemetry;
pub mod tmdb;
pub mod transcode;

/// Installs ring as the process-wide TLS crypto provider.
///
/// sqlx uses ring, and reqwest is built without a provider so that aws-lc never enters the
/// tree. rustls refuses to pick a provider on its own once more than one could apply, so
/// both binaries call this first thing in `main`. A second call is harmless.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Name of this process instance, used in health rows and probe object keys.
///
/// Kubernetes sets `HOSTNAME` to the pod name. Outside a cluster the value only has to be
/// stable for the life of the process.
pub fn instance_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.is_empty() && h.len() <= 128 && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or_else(|| String::from("local"))
}
