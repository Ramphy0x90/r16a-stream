//! The worker binary. Claims jobs from Postgres and runs ffmpeg. Images since Phase 3b,
//! transcodes from Phase 4.
//!
//! Subcommands
//! - `run` (default) starts the health server and the background tasks
//! - `healthcheck` probes the local `/healthz`
//!
//! The worker connects as `r16a_stream_worker`, which only holds the grants in SPEC 4.9, and
//! holds the Garage worker key, which cannot write the uploads bucket.

use std::{process::ExitCode, sync::Arc};

use axum::{Router, extract::State, http::StatusCode, routing::get};
use r16a_stream_backend::{
    config::WorkerConfig,
    db,
    health::{healthz, shutdown_signal},
    images::worker::ImageWorker,
    jobs::repository as jobs,
    storage::{
        GarageClient,
        probe::{self, Component, WriteHealth},
    },
    telemetry,
};

#[derive(Clone)]
struct WorkerState {
    db: sqlx::PgPool,
    write_health: Arc<WriteHealth>,
    probe_interval: std::time::Duration,
    scratch_dir: std::path::PathBuf,
}

fn main() -> ExitCode {
    r16a_stream_backend::install_crypto_provider();
    #[cfg(debug_assertions)]
    let _ = dotenvy::dotenv();

    let command = std::env::args().nth(1).unwrap_or_else(|| String::from("run"));
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("could not start the async runtime, {e}");
            return ExitCode::FAILURE;
        }
    };
    match command.as_str() {
        "run" => runtime.block_on(run()),
        "healthcheck" => runtime.block_on(r16a_stream_backend::health::check_local(
            &std::env::var("R16A_WORKER_HEALTH_BIND").unwrap_or_else(|_| String::from("127.0.0.1:8082")),
        )),
        other => {
            eprintln!("unknown command {other}, expected run or healthcheck");
            ExitCode::from(2)
        }
    }
}

/// Ready means the database answers, the last Garage write probe succeeded, and scratch space
/// is writable. Unlike the API, the worker includes Garage, because it only writes (SPEC 7.8).
async fn readyz(State(s): State<WorkerState>) -> StatusCode {
    let db_ok = matches!(
        tokio::time::timeout(std::time::Duration::from_secs(2), db::ping(&s.db)).await,
        Ok(Ok(()))
    );
    let scratch_ok = tokio::fs::metadata(&s.scratch_dir).await.is_ok_and(|m| m.is_dir());
    if db_ok && scratch_ok && s.write_health.is_writable(s.probe_interval) {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn run() -> ExitCode {
    let config = match WorkerConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("configuration error, {e}");
            return ExitCode::from(2);
        }
    };
    telemetry::init(config.log_format);

    if let Err(e) = tokio::fs::create_dir_all(&config.scratch_dir).await {
        tracing::error!(error = %e, "scratch directory is not usable");
        return ExitCode::FAILURE;
    }
    let pool = match db::connect(&config.db, "r16a-stream-worker") {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "database setup failed");
            return ExitCode::FAILURE;
        }
    };
    let storage = match GarageClient::new(&config.s3) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!(error = %e, "storage setup failed");
            return ExitCode::FAILURE;
        }
    };
    let write_health = Arc::new(WriteHealth::default());
    spawn_job_workers(&config, &pool, &storage).await;
    spawn_reaper(pool.clone());
    probe::spawn(
        storage,
        pool.clone(),
        write_health.clone(),
        Component::Worker,
        r16a_stream_backend::instance_name(),
        config.s3.probe_interval,
    );

    let state = WorkerState {
        db: pool,
        write_health,
        probe_interval: config.s3.probe_interval,
        scratch_dir: config.scratch_dir.clone(),
    };
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state);
    let listener = match tokio::net::TcpListener::bind(config.health_bind).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, bind = %config.health_bind, "could not bind");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(bind = %config.health_bind, "worker health listening");
    match axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "health server error");
            ExitCode::FAILURE
        }
    }
}

/// Starts the image, transcode and housekeeping loops (SPEC 7.2), after the encoder check of
/// SPEC 7.5.
async fn spawn_job_workers(config: &WorkerConfig, pool: &sqlx::PgPool, storage: &Arc<GarageClient>) {
    let image_worker = Arc::new(ImageWorker {
        db: pool.clone(),
        storage: storage.clone(),
        ffmpeg: config.ffmpeg.clone(),
        ffprobe: config.ffprobe.clone(),
        scratch_root: config.scratch_dir.clone(),
        instance: r16a_stream_backend::instance_name(),
    });
    for _ in 0..config.image_concurrency {
        tokio::spawn(image_worker.clone().run());
    }
    let (encoder, encoder_label) =
        r16a_stream_backend::transcode::worker::choose_encoder(&config.ffmpeg, config.encoder).await;
    let instance = r16a_stream_backend::instance_name();
    if let Err(e) = r16a_stream_backend::transcode::repository::report_encoder(pool, &instance, encoder_label).await {
        tracing::warn!(error = %e, "reporting the encoder failed");
    }
    tracing::info!(encoder = encoder_label, "transcode encoder");
    let transcode_worker = Arc::new(r16a_stream_backend::transcode::worker::TranscodeWorker {
        db: pool.clone(),
        storage: storage.clone(),
        ffmpeg: config.ffmpeg.clone(),
        ffprobe: config.ffprobe.clone(),
        scratch_root: config.scratch_dir.clone(),
        instance,
        encoder,
        scratch_reserve: config.scratch_reserve_bytes,
        min_free: config.min_free_bytes,
        scratch_in_use: Arc::default(),
    });
    for _ in 0..config.transcode_concurrency {
        tokio::spawn(transcode_worker.clone().run());
    }
    let housekeeping = Arc::new(r16a_stream_backend::transcode::housekeeping::HousekeepingWorker {
        db: pool.clone(),
        storage: storage.clone(),
        instance: r16a_stream_backend::instance_name(),
        gc_grace: config.gc_grace,
    });
    // The first encode GC after start, unless one is pending already (SPEC 7.2).
    match pool.acquire().await {
        Ok(mut conn) => {
            if let Err(e) = r16a_stream_backend::transcode::repository::enqueue_gc(&mut conn, 60.0).await {
                tracing::warn!(error = %e, "queueing encode GC failed");
            }
        }
        Err(e) => tracing::warn!(error = %e, "queueing encode GC failed"),
    }
    tokio::spawn(housekeeping.run());
}

/// Puts jobs back whose worker stopped heartbeating for two minutes, for example after a crash
/// or a node loss (SPEC 7.2). Any worker may do it, the update is idempotent.
fn spawn_reaper(pool: sqlx::PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            match jobs::reap_stale(&pool, 120.0).await {
                Ok(n) if n > 0 => tracing::warn!(jobs = n, "stale jobs requeued"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "job reaper failed"),
            }
        }
    });
}
