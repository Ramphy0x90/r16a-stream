//! SQL for the job queue, run by the worker role.

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

pub struct Claimed {
    pub id: Uuid,
    pub kind: String,
    pub image_id: Option<Uuid>,
    pub attempts: i16,
    pub max_attempts: i16,
    pub storage_waits: i32,
}

/// Claims the next due job of one of `kinds` (H11). Concurrent workers skip each other's rows.
pub async fn claim(db: &PgPool, worker: &str, kinds: &[String]) -> Result<Option<Claimed>, sqlx::Error> {
    sqlx::query_as!(
        Claimed,
        "update jobs set status = 'running', locked_by = $1, started_at = now(), heartbeat_at = now(),
                blocked_reason = null
         where id = (
           select id from jobs
           where status = 'queued' and run_after <= now() and kind = any($2)
           order by priority desc, run_after, id
           for update skip locked
           limit 1)
         returning id, kind, image_id, attempts, max_attempts, storage_waits",
        worker,
        kinds
    )
    .fetch_optional(db)
    .await
}

pub async fn heartbeat(db: &PgPool, id: Uuid, progress: f32) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update jobs set heartbeat_at = now(), progress = $2 where id = $1 and status = 'running'",
        id,
        progress
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn succeed(db: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update jobs set status = 'succeeded', progress = 1, finished_at = now(), locked_by = null,
                failure_code = null, failure_detail = null
         where id = $1",
        id
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Puts the job back after a storage problem. Never counts as a failure (SPEC 7.2).
pub async fn requeue_storage(db: &PgPool, id: Uuid, delay_secs: f64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update jobs set status = 'queued', locked_by = null, heartbeat_at = null,
                blocked_reason = 'storage_degraded', storage_waits = storage_waits + 1,
                run_after = now() + make_interval(secs => $2)
         where id = $1",
        id,
        delay_secs
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Counts a transient failure, and queues the job again after `delay_secs` unless it is out of
/// attempts. Returns true when the job is now failed for good.
pub async fn fail_transient(
    db: &mut PgConnection,
    id: Uuid,
    code: &str,
    detail: &str,
    delay_secs: f64,
) -> Result<bool, sqlx::Error> {
    let status = sqlx::query_scalar!(
        "update jobs set attempts = attempts + 1, locked_by = null, heartbeat_at = null,
                failure_code = $2, failure_detail = $3,
                status = case when attempts + 1 >= max_attempts then 'failed' else 'queued' end,
                finished_at = case when attempts + 1 >= max_attempts then now() end,
                run_after = now() + make_interval(secs => $4)
         where id = $1 returning status",
        id,
        code,
        detail,
        delay_secs
    )
    .fetch_one(db)
    .await?;
    Ok(status == "failed")
}

/// Ends the job for an input that will never work, without retries.
pub async fn fail_invalid(db: &mut PgConnection, id: Uuid, code: &str, detail: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update jobs set status = 'failed', attempts = attempts + 1, locked_by = null, finished_at = now(),
                failure_code = $2, failure_detail = $3
         where id = $1",
        id,
        code,
        detail
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Puts jobs whose worker stopped heartbeating back in the queue (SPEC 7.2).
pub async fn reap_stale(db: &PgPool, stale_secs: f64) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query!(
        "update jobs set status = 'queued', locked_by = null, heartbeat_at = null
         where status in ('running', 'cancelling') and heartbeat_at < now() - make_interval(secs => $1)",
        stale_secs
    )
    .execute(db)
    .await?
    .rows_affected())
}
