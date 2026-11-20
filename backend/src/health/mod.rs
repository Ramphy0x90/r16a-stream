//! Liveness, readiness and the public status endpoint.
//!
//! `/healthz` and `/readyz` are served on the container port for the kubelet and are not
//! routed by the Ingress. The API's readiness checks only the database, never Garage, so a
//! storage outage cannot take playback offline (SPEC 7.8, C2).

use std::time::Duration;

use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;
use utoipa::ToSchema;

use crate::state::AppState;

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

pub async fn healthz() -> &'static str {
    "ok"
}

pub async fn api_readyz(State(state): State<AppState>) -> StatusCode {
    match tokio::time::timeout(PROBE_TIMEOUT, crate::db::ping(&state.db)).await {
        Ok(Ok(())) => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// Answer of `GET /api/v1/status`. Deliberately free of versions and hostnames.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    /// Always `r16a-stream`.
    pub service: &'static str,
    /// Major version of this API, the `v1` in the path.
    pub api_version: u8,
}

/// Lets the web app check that the API is reachable through the same origin.
#[utoipa::path(
    get,
    path = "/api/v1/status",
    tag = "status",
    responses((status = 200, description = "The API is up", body = StatusResponse))
)]
pub async fn status() -> Json<StatusResponse> {
    Json(StatusResponse {
        service: "r16a-stream",
        api_version: 1,
    })
}

/// Resolves on SIGTERM or Ctrl-C, so Kubernetes rollouts drain in-flight requests.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = term => {},
    }
}

/// Container health check without curl. Sends `GET /healthz` to the local port given by the
/// bind address and exits 0 only on a 200 answer.
pub async fn check_local(bind: &str) -> std::process::ExitCode {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let port = bind.rsplit(':').next().unwrap_or("8081");
    let attempt = async {
        let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await?;
        stream
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await?;
        let mut buf = [0u8; 32];
        let n = stream.read(&mut buf).await?;
        Ok::<bool, std::io::Error>(buf[..n].starts_with(b"HTTP/1.1 200"))
    };
    match tokio::time::timeout(PROBE_TIMEOUT, attempt).await {
        Ok(Ok(true)) => std::process::ExitCode::SUCCESS,
        _ => std::process::ExitCode::FAILURE,
    }
}
