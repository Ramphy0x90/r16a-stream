//! The API binary.
//!
//! Subcommands
//! - `serve` (default) runs the HTTP API
//! - `migrate` applies migrations with `R16A_MIGRATE_DATABASE_URL`, used by the init container
//! - `healthcheck` probes the local `/healthz`, for container health checks without curl
//! - `openapi` prints the `OpenAPI` document
//! - `seed --bench` loads the SPEC 1 scale, prints the hot query report, and removes the data,
//!   development and test only

use std::{net::SocketAddr, process::ExitCode, sync::Arc, time::Duration};

use r16a_stream_backend::{
    app,
    auth::{AuthState, repository as auth_repo},
    config::{self, ApiConfig, Env},
    db,
    http::rate_limit::SHRINK_INTERVAL,
    openapi,
    state::AppState,
    storage::{
        GarageClient,
        probe::{self, Component, WriteHealth},
    },
    telemetry,
};

fn main() -> ExitCode {
    r16a_stream_backend::install_crypto_provider();
    // Debug builds read backend/.env, release builds and containers only the real environment.
    #[cfg(debug_assertions)]
    let _ = dotenvy::dotenv();

    let command = std::env::args().nth(1).unwrap_or_else(|| String::from("serve"));
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("could not start the async runtime, {e}");
            return ExitCode::FAILURE;
        }
    };
    match command.as_str() {
        "serve" => runtime.block_on(serve()),
        "migrate" => runtime.block_on(migrate()),
        "seed" if std::env::args().nth(2).as_deref() == Some("--bench") => runtime.block_on(bench()),
        "healthcheck" => runtime.block_on(r16a_stream_backend::health::check_local(
            &std::env::var("R16A_HTTP_BIND").unwrap_or_else(|_| String::from("127.0.0.1:8081")),
        )),
        "openapi" => match openapi::to_json() {
            Ok(json) => {
                print!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("could not render the OpenAPI document, {e}");
                ExitCode::FAILURE
            }
        },
        other => {
            eprintln!("unknown command {other}, expected serve, migrate, healthcheck, openapi or seed --bench");
            ExitCode::from(2)
        }
    }
}

async fn bench() -> ExitCode {
    let lookup = |n: &str| std::env::var(n).ok();
    let env = std::env::var("R16A_ENV").unwrap_or_default();
    if env != "development" && env != "test" {
        eprintln!("seed --bench writes bulk data and runs only with R16A_ENV development or test");
        return ExitCode::from(2);
    }
    // The owner, because ANALYZE needs the table owner.
    let url = match config::migrate_database_url(&lookup) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("configuration error, {e}");
            return ExitCode::from(2);
        }
    };
    let pool = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(secrecy::ExposeSecret::expose_secret(&url))
        .await
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("database connection failed, {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = async {
        r16a_stream_backend::bench::cleanup(&pool).await?;
        r16a_stream_backend::bench::seed(&pool).await?;
        r16a_stream_backend::bench::report(&pool).await
    }
    .await;
    // `--keep` leaves the data in place to look at plans by hand. Run without it to clean up.
    let keep = std::env::args().nth(3).as_deref() == Some("--keep");
    let cleaned = if keep {
        Ok(())
    } else {
        r16a_stream_backend::bench::cleanup(&pool).await
    };
    match (result, cleaned) {
        (Ok(report), Ok(())) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("bench failed, {e}");
            ExitCode::FAILURE
        }
    }
}

async fn migrate() -> ExitCode {
    let url = match config::migrate_database_url(&|n| std::env::var(n).ok()) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("configuration error, {e}");
            return ExitCode::from(2);
        }
    };
    match db::migrate(&url).await {
        Ok(()) => {
            println!("migrations applied");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> ExitCode {
    let config = match ApiConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("configuration error, {e}");
            return ExitCode::from(2);
        }
    };
    telemetry::init(config.log_format);

    // In development the API migrates itself when the owner URL is present, so `cargo run`
    // needs no separate step. Production always uses the init container.
    if config.env == Env::Development
        && let Ok(url) = config::migrate_database_url(&|n| std::env::var(n).ok())
        && let Err(e) = db::migrate(&url).await
    {
        tracing::error!(error = %e, "development migration failed");
        return ExitCode::FAILURE;
    }

    let pool = match db::connect(&config.db, "r16a-stream-api") {
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
    probe::spawn(
        storage.clone(),
        pool.clone(),
        write_health.clone(),
        Component::Api,
        r16a_stream_backend::instance_name(),
        config.s3.probe_interval,
    );

    let auth = match AuthState::new(&config) {
        Ok(a) => Arc::new(a),
        Err(e) => {
            tracing::error!(error = %e, "authentication setup failed");
            return ExitCode::FAILURE;
        }
    };
    let Some((tmdb, tmdb_images, image_urls)) = outbound_clients(&config) else {
        return ExitCode::FAILURE;
    };
    let limits = Arc::new(r16a_stream_backend::http::rate_limit::FeatureLimits::default());
    spawn_auth_tasks(auth.clone(), limits.clone(), pool.clone());

    let bind = config.http_bind;
    let state = AppState {
        config: Arc::new(config),
        db: pool,
        storage,
        write_health,
        auth,
        limits,
        tmdb,
        tmdb_images,
        image_urls,
    };
    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, %bind, "could not bind");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(%bind, "api listening");
    // The peer address feeds the client IP for login rate limits (SPEC 16).
    match axum::serve(
        listener,
        app::router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(r16a_stream_backend::health::shutdown_signal())
    .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "server error");
            ExitCode::FAILURE
        }
    }
}

/// Discovery warm-up, hourly removal of expired sessions, flows and TMDB cache entries, and
/// trimming of idle rate limiter entries.
fn spawn_auth_tasks(
    auth: Arc<AuthState>,
    limits: Arc<r16a_stream_backend::http::rate_limit::FeatureLimits>,
    pool: sqlx::PgPool,
) {
    let warm = auth.clone();
    tokio::spawn(async move { warm.warm_up().await });
    tokio::spawn(async move {
        let mut purge = tokio::time::interval(Duration::from_secs(3600));
        let mut shrink = tokio::time::interval(SHRINK_INTERVAL);
        loop {
            tokio::select! {
                _ = purge.tick() => {
                    match auth_repo::purge_expired(&pool).await {
                        Ok(n) if n > 0 => tracing::info!(sessions = n, "expired sessions removed"),
                        Ok(_) => {}
                        Err(e) => tracing::warn!(error = %e, "session cleanup failed"),
                    }
                    if let Err(e) = r16a_stream_backend::tmdb::repository::purge_expired(&pool).await {
                        tracing::warn!(error = %e, "TMDB cache cleanup failed");
                    }
                }
                _ = shrink.tick() => {
                    auth.login_limit.shrink();
                    auth.user_limit.shrink();
                    limits.shrink();
                }
            }
        }
    });
}

type Clients = (
    Arc<r16a_stream_backend::tmdb::client::TmdbClient>,
    Arc<r16a_stream_backend::images::ingest::TmdbImages>,
    Arc<r16a_stream_backend::images::urls::ImageUrls>,
);

/// The TMDB API and image clients and the image URL signer. Logs and returns `None` on failure.
fn outbound_clients(config: &ApiConfig) -> Option<Clients> {
    let https_only = config.env == Env::Production;
    let tmdb = match r16a_stream_backend::tmdb::client::TmdbClient::new(&config.tmdb, https_only) {
        Ok(t) => Arc::new(t),
        Err(e) => {
            tracing::error!(error = %e, "TMDB client setup failed");
            return None;
        }
    };
    let tmdb_images =
        match r16a_stream_backend::images::ingest::TmdbImages::new(config.tmdb.image_base_url.clone(), https_only) {
            Ok(t) => Arc::new(t),
            Err(e) => {
                tracing::error!(error = %e, "TMDB image client setup failed");
                return None;
            }
        };
    let Some(image_urls) =
        r16a_stream_backend::images::urls::ImageUrls::new(&config.s3_public_endpoint, &config.s3).map(Arc::new)
    else {
        tracing::error!("R16A_S3_PUBLIC_ENDPOINT cannot sign image URLs");
        return None;
    };
    Some((tmdb, tmdb_images, image_urls))
}
