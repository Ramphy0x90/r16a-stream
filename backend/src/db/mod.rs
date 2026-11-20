//! Postgres pool and migrations.

use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

use crate::config::DbConfig;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    // The URL is never part of these messages, it holds the password.
    #[error("database URL could not be parsed")]
    BadUrl,
    #[error("could not connect to the database, {0}")]
    Connect(sqlx::Error),
    #[error("migration failed, {0}")]
    Migrate(sqlx::migrate::MigrateError),
}

/// The migrations embedded in both binaries. Only the `migrate` subcommand applies them.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Opens the runtime pool. Every connection starts with a statement timeout, sent as a
/// startup parameter, so a slow query cannot hold a connection forever.
pub fn connect(cfg: &DbConfig, application_name: &str) -> Result<PgPool, DbError> {
    let statement_timeout_ms = cfg.statement_timeout.as_millis().to_string();
    let opts: PgConnectOptions = cfg
        .url
        .expose_secret()
        .parse::<PgConnectOptions>()
        .map_err(|_| DbError::BadUrl)?
        .application_name(application_name)
        .options([("statement_timeout", statement_timeout_ms.as_str())]);
    Ok(PgPoolOptions::new()
        .max_connections(cfg.max_connections)
        .acquire_timeout(cfg.acquire_timeout)
        .idle_timeout(Some(Duration::from_secs(300)))
        .connect_lazy_with(opts))
}

/// Applies pending migrations with the owner role. Used by the `migrate` subcommand only.
pub async fn migrate(owner_url: &SecretString) -> Result<(), DbError> {
    let opts: PgConnectOptions = owner_url
        .expose_secret()
        .parse::<PgConnectOptions>()
        .map_err(|_| DbError::BadUrl)?
        .application_name("r16a-stream-migrate");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(opts)
        .await
        .map_err(DbError::Connect)?;
    MIGRATOR.run(&pool).await.map_err(DbError::Migrate)?;
    pool.close().await;
    Ok(())
}

/// Cheap round trip used by readiness probes.
pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query_scalar!("select 1 as \"one!\"")
        .fetch_one(pool)
        .await
        .map(|_| ())
}
