//! SQL for the TMDB response cache.

use serde_json::Value;
use sqlx::PgPool;

pub async fn cached(db: &PgPool, key: &str) -> Result<Option<Value>, sqlx::Error> {
    sqlx::query_scalar!(
        "select payload from tmdb_cache where cache_key = $1 and expires_at > now()",
        key
    )
    .fetch_optional(db)
    .await
}

pub async fn store(db: &PgPool, key: &str, payload: &Value, ttl_secs: f64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "insert into tmdb_cache (cache_key, payload, expires_at)
         values ($1, $2, now() + make_interval(secs => $3))
         on conflict (cache_key) do update
           set payload = excluded.payload, fetched_at = now(), expires_at = excluded.expires_at",
        key,
        payload,
        ttl_secs,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn purge_expired(db: &PgPool) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query!("delete from tmdb_cache where expires_at < now()")
        .execute(db)
        .await?
        .rows_affected())
}
