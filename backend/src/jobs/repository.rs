//! SQL for the job queue, run by the worker role.

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

pub struct Claimed {
    pub id: Uuid,
    pub kind: String,
    pub image_id: Option<Uuid>,
    pub variant_id: Option<Uuid>,
    /// Ids only (SPEC 7.2).
    pub payload: serde_json::Value,
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
         returning id, kind, image_id, variant_id, payload, attempts, max_attempts, storage_waits",
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

/// Puts jobs whose worker stopped heartbeating back in the queue and marks their running
/// attempts abandoned (SPEC 7.2). A job an admin was cancelling ends cancelled instead.
pub async fn reap_stale(db: &PgPool, stale_secs: f64) -> Result<u64, sqlx::Error> {
    let mut tx = db.begin().await?;
    let ids = sqlx::query_scalar!(
        "update jobs set locked_by = null, heartbeat_at = null,
                status = case when status = 'cancelling' then 'cancelled' else 'queued' end,
                finished_at = case when status = 'cancelling' then now() end
         where status in ('running', 'cancelling') and heartbeat_at < now() - make_interval(secs => $1)
         returning id",
        stale_secs
    )
    .fetch_all(&mut *tx)
    .await?;
    if !ids.is_empty() {
        sqlx::query!(
            "update transcode_attempts set status = 'abandoned', finished_at = now()
             where status = 'running' and job_id = any($1)",
            &ids
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(ids.len() as u64)
}

/// Writes the heartbeat and returns the job's status, so a worker sees a cancel request. `None`
/// when the job is no longer its own, for example after the reaper requeued it.
pub async fn heartbeat_status(
    db: &PgPool,
    id: Uuid,
    worker: &str,
    progress: f32,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "update jobs set heartbeat_at = now(), progress = $3
         where id = $1 and locked_by = $2 and status in ('running', 'cancelling') returning status",
        id,
        worker,
        progress
    )
    .fetch_optional(db)
    .await
}

/// Requeues a job that waits for space, scratch or Garage, without counting a failure.
pub async fn requeue_blocked(db: &PgPool, id: Uuid, reason: &str, delay_secs: f64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update jobs set status = 'queued', locked_by = null, heartbeat_at = null, progress = null,
                blocked_reason = $2, storage_waits = storage_waits + 1,
                run_after = now() + make_interval(secs => $3)
         where id = $1",
        id,
        reason,
        delay_secs
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn cancelled(db: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update jobs set status = 'cancelled', finished_at = now(), locked_by = null, heartbeat_at = null
         where id = $1",
        id
    )
    .execute(db)
    .await?;
    Ok(())
}
