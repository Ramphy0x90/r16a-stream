//! SQL for variants, uploads and the admin job view, run by the API's role.

use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

// ---------------------------------------------------------------- variants

pub struct TitleShape {
    pub kind: String,
    pub runtime_minutes: Option<i32>,
}

pub async fn title_shape(db: &mut PgConnection, title_id: Uuid) -> Result<Option<TitleShape>, sqlx::Error> {
    sqlx::query_as!(
        TitleShape,
        "select kind, runtime_minutes from titles where id = $1",
        title_id
    )
    .fetch_optional(db)
    .await
}

/// Runtime of an episode that belongs to `title_id`, outer `None` when it does not.
pub async fn episode_runtime(
    db: &mut PgConnection,
    title_id: Uuid,
    episode_id: Uuid,
) -> Result<Option<Option<i32>>, sqlx::Error> {
    sqlx::query_scalar!(
        "select runtime_minutes from episodes where id = $1 and title_id = $2",
        episode_id,
        title_id
    )
    .fetch_optional(db)
    .await
}

pub async fn insert_variant(
    db: &mut PgConnection,
    title_id: Uuid,
    kind: &str,
    episode_id: Option<Uuid>,
    language: &str,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar!(
        "insert into media_variants (title_id, title_kind, episode_id, language) values ($1, $2, $3, $4)
         returning id",
        title_id,
        kind,
        episode_id,
        language
    )
    .fetch_one(db)
    .await
}

pub struct VariantRef {
    pub id: Uuid,
    pub title_id: Uuid,
    pub episode_id: Option<Uuid>,
    pub status: String,
    pub runtime_minutes: Option<i32>,
}

/// A variant with the runtime its estimate falls back to, locked for the transaction.
pub async fn variant_for_update(db: &mut PgConnection, id: Uuid) -> Result<Option<VariantRef>, sqlx::Error> {
    sqlx::query_as!(
        VariantRef,
        r#"select v.id, v.title_id, v.episode_id, v.status,
                  coalesce(e.runtime_minutes, t.runtime_minutes) as runtime_minutes
           from media_variants v
           join titles t on t.id = v.title_id
           left join episodes e on e.id = v.episode_id
           where v.id = $1
           for update of v"#,
        id
    )
    .fetch_optional(db)
    .await
}

/// Source objects of a variant still in the uploads bucket, and initiated multipart uploads.
pub struct SourceObject {
    pub id: Uuid,
    pub object_key: String,
    pub s3_upload_id: String,
    pub status: String,
}

pub async fn variant_sources(db: &mut PgConnection, variant_id: Uuid) -> Result<Vec<SourceObject>, sqlx::Error> {
    sqlx::query_as!(
        SourceObject,
        "select id, object_key, s3_upload_id, status from uploads
         where variant_id = $1 and source_deleted_at is null and status <> 'aborted'",
        variant_id
    )
    .fetch_all(db)
    .await
}

pub async fn delete_variant(db: &mut PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
    // Uploads keep their rows (variant_id set null) as the record of what was stored.
    sqlx::query!(
        "update uploads set reserved_bytes = 0, source_deleted_at = coalesce(source_deleted_at, now())
         where variant_id = $1",
        id
    )
    .execute(&mut *db)
    .await?;
    let gone = sqlx::query!(
        "delete from media_variants where id = $1 returning title_id, language",
        id
    )
    .fetch_optional(&mut *db)
    .await?;
    let Some(gone) = gone else { return Ok(false) };
    // The title keeps the language only while another ready variant has it.
    sqlx::query!(
        "delete from title_languages tl where tl.title_id = $1 and tl.language = $2
           and not exists (select 1 from media_variants v
                           where v.title_id = $1 and v.language = $2 and v.status = 'ready')",
        gone.title_id,
        gone.language
    )
    .execute(db)
    .await?;
    Ok(true)
}

pub struct AdminVariantRow {
    pub id: Uuid,
    pub episode_id: Option<Uuid>,
    pub language: String,
    pub status: String,
    pub duration_ms: Option<i64>,
    pub ready_at: Option<OffsetDateTime>,
    pub current_bytes: Option<i64>,
    pub source_bytes: Option<i64>,
    pub source_kept: bool,
    pub job_id: Option<Uuid>,
    pub job_status: Option<String>,
    pub job_progress: Option<f32>,
    pub job_failure: Option<String>,
    pub open_upload_id: Option<Uuid>,
}

/// Every variant of a title with its latest job, current encode size and source state.
pub async fn admin_variants(db: &PgPool, title_id: Uuid) -> Result<Vec<AdminVariantRow>, sqlx::Error> {
    sqlx::query_as!(
        AdminVariantRow,
        r#"select v.id, v.episode_id, v.language, v.status, v.duration_ms, v.ready_at,
                  a.total_bytes as current_bytes,
                  src.size_bytes as "source_bytes?",
                  (src.id is not null) as "source_kept!",
                  j.id as "job_id?", j.status as "job_status?", j.progress as job_progress,
                  j.failure_detail as job_failure,
                  open.id as "open_upload_id?"
           from media_variants v
           left join transcode_attempts a on a.id = v.current_attempt_id
           left join lateral (
             select u.id, u.size_bytes from uploads u
             where u.variant_id = v.id and u.status = 'completed' and u.source_deleted_at is null
             order by u.completed_at desc limit 1) src on true
           left join lateral (
             select u.id from uploads u
             where u.variant_id = v.id and u.status = 'initiated' and u.expires_at > now()
             order by u.created_at desc limit 1) open on true
           left join lateral (
             select j.id, j.status, j.progress, j.failure_detail from jobs j
             where j.variant_id = v.id and j.kind = 'transcode'
             order by j.created_at desc limit 1) j on true
           where v.title_id = $1
           order by v.episode_id nulls first, v.language"#,
        title_id
    )
    .fetch_all(db)
    .await
}

// ---------------------------------------------------------------- uploads

pub struct NewUpload<'a> {
    pub id: Uuid,
    pub variant_id: Uuid,
    pub object_key: &'a str,
    pub s3_upload_id: &'a str,
    pub size_bytes: i64,
    pub part_size: i32,
    pub part_count: i32,
    pub original_filename: Option<&'a str>,
    pub estimated_output_bytes: i64,
    pub reserved_bytes: i64,
    pub created_by: Uuid,
}

pub async fn insert_upload(db: &mut PgConnection, u: NewUpload<'_>) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "insert into uploads (id, purpose, variant_id, object_key, s3_upload_id, size_bytes, part_size, part_count,
                              original_filename, estimated_output_bytes, reserved_bytes, created_by, expires_at)
         values ($1, 'video_source', $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, now() + interval '24 hours')",
        u.id,
        u.variant_id,
        u.object_key,
        u.s3_upload_id,
        u.size_bytes,
        u.part_size,
        u.part_count,
        u.original_filename,
        u.estimated_output_bytes,
        u.reserved_bytes,
        u.created_by,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn has_open_upload(db: &mut PgConnection, variant_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"select exists (select 1 from uploads where variant_id = $1 and status = 'initiated'
                          and expires_at > now()) as "e!""#,
        variant_id
    )
    .fetch_one(db)
    .await
}

pub async fn has_active_job(db: &mut PgConnection, variant_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"select exists (select 1 from jobs where variant_id = $1
                          and status in ('queued', 'running', 'cancelling')) as "e!""#,
        variant_id
    )
    .fetch_one(db)
    .await
}

pub struct UploadRow {
    pub id: Uuid,
    pub purpose: String,
    pub variant_id: Option<Uuid>,
    pub image_id: Option<Uuid>,
    pub image_kind: Option<String>,
    pub image_title_id: Option<Uuid>,
    pub image_episode_id: Option<Uuid>,
    pub object_key: String,
    pub s3_upload_id: String,
    pub size_bytes: i64,
    pub part_size: i32,
    pub part_count: i32,
    pub status: String,
    pub expired: bool,
    pub source_deleted: bool,
}

pub async fn upload(db: &mut PgConnection, id: Uuid, lock: bool) -> Result<Option<UploadRow>, sqlx::Error> {
    if lock {
        sqlx::query_as!(
            UploadRow,
            r#"select id, purpose, variant_id, image_id, image_kind, image_title_id, image_episode_id, object_key,
                      s3_upload_id, size_bytes, part_size, part_count, status,
                      expires_at <= now() as "expired!", source_deleted_at is not null as "source_deleted!"
               from uploads where id = $1 for update"#,
            id
        )
        .fetch_optional(db)
        .await
    } else {
        sqlx::query_as!(
            UploadRow,
            r#"select id, purpose, variant_id, image_id, image_kind, image_title_id, image_episode_id, object_key,
                      s3_upload_id, size_bytes, part_size, part_count, status,
                      expires_at <= now() as "expired!", source_deleted_at is not null as "source_deleted!"
               from uploads where id = $1"#,
            id
        )
        .fetch_optional(db)
        .await
    }
}

pub async fn mark_completed(db: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update uploads set status = 'completed', completed_at = now() where id = $1",
        id
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Marks an open upload aborted. A manual image upload also loses its pending image row.
pub async fn mark_aborted(db: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    let image = sqlx::query_scalar!(
        "update uploads set status = 'aborted', reserved_bytes = 0 where id = $1 and status = 'initiated'
         returning image_id",
        id
    )
    .fetch_optional(&mut *db)
    .await?
    .flatten();
    if let Some(image) = image {
        delete_pending_image(db, image).await?;
    }
    Ok(())
}

/// Adds a re-transcode's estimate to the reservation of the upload it reads.
pub async fn add_reservation(db: &mut PgConnection, id: Uuid, bytes: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update uploads set reserved_bytes = reserved_bytes + $2 where id = $1",
        id,
        bytes
    )
    .execute(db)
    .await?;
    Ok(())
}

/// The newest completed upload of a variant whose source still exists.
pub async fn usable_source(db: &mut PgConnection, variant_id: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "select id from uploads where variant_id = $1 and status = 'completed' and source_deleted_at is null
         order by completed_at desc limit 1",
        variant_id
    )
    .fetch_optional(db)
    .await
}

pub async fn set_variant_processing(db: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    // A ready variant stays ready on its current attempt until the new one switches (SPEC 4.5).
    sqlx::query!(
        "update media_variants set status = case when status = 'ready' then 'ready' else 'processing' end,
                updated_at = now()
         where id = $1",
        id
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Queues a transcode of `upload_id` for the variant. One pending per variant (dedupe key).
pub async fn enqueue_transcode(db: &mut PgConnection, variant_id: Uuid, upload_id: Uuid) -> Result<Uuid, sqlx::Error> {
    let id = sqlx::query_scalar!(
        "insert into jobs (kind, payload, variant_id, dedupe_key)
         values ('transcode', jsonb_build_object('uploadId', $2::uuid), $1::uuid, 'transcode:' || $1::uuid::text)
         returning id",
        variant_id,
        upload_id
    )
    .fetch_one(&mut *db)
    .await?;
    sqlx::query!("select pg_notify('r16a_stream_jobs', 'transcode')")
        .execute(db)
        .await?;
    Ok(id)
}

pub struct StaleUpload {
    pub id: Uuid,
    pub object_key: String,
    pub s3_upload_id: String,
}

pub async fn stale_uploads(db: &PgPool) -> Result<Vec<StaleUpload>, sqlx::Error> {
    sqlx::query_as!(
        StaleUpload,
        "select id, object_key, s3_upload_id from uploads
         where status = 'initiated' and expires_at <= now() order by expires_at limit 100"
    )
    .fetch_all(db)
    .await
}

pub struct RetentionCandidate {
    pub id: Uuid,
    pub object_key: String,
    pub variant_id: Option<Uuid>,
}

/// Sources whose variant's current, verified attempt was made from them, older than the delay,
/// and whose variant has no pending job (SPEC 7.9).
pub async fn retention_candidates(db: &PgPool, delay_secs: f64) -> Result<Vec<RetentionCandidate>, sqlx::Error> {
    sqlx::query_as!(
        RetentionCandidate,
        "select u.id, u.object_key, u.variant_id
         from uploads u
         join media_variants v on v.id = u.variant_id
         join transcode_attempts a on a.id = v.current_attempt_id
         where u.status = 'completed' and u.source_deleted_at is null and u.purpose = 'video_source'
           and a.upload_id = u.id and a.status = 'ready' and a.verified_at is not null
           and a.verified_at <= now() - make_interval(secs => $1)
           and not exists (select 1 from jobs j where j.variant_id = v.id
                           and j.status in ('queued', 'running', 'cancelling'))
         order by u.completed_at limit 50",
        delay_secs
    )
    .fetch_all(db)
    .await
}

pub async fn mark_source_deleted(db: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!("update uploads set source_deleted_at = now() where id = $1", id)
        .execute(db)
        .await?;
    Ok(())
}

/// A session-level advisory lock so one API replica runs retention at a time.
pub async fn try_retention_lock(db: &mut PgConnection) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(r#"select pg_try_advisory_lock(1377214465) as "got!""#)
        .fetch_one(db)
        .await
}

pub async fn release_retention_lock(db: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query_scalar!(r#"select pg_advisory_unlock(1377214465) as "released!""#)
        .fetch_one(db)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------- jobs

pub struct JobRow {
    pub id: Uuid,
    pub kind: String,
    pub status: String,
    pub progress: Option<f32>,
    pub attempts: i16,
    pub max_attempts: i16,
    pub storage_waits: i32,
    pub blocked_reason: Option<String>,
    pub failure_code: Option<String>,
    pub failure_detail: Option<String>,
    pub variant_id: Option<Uuid>,
    pub image_id: Option<Uuid>,
    pub payload: Value,
    pub created_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
    pub run_after: OffsetDateTime,
}

pub async fn jobs(
    db: &PgPool,
    status: Option<&str>,
    kind: Option<&str>,
    after: Option<(OffsetDateTime, Uuid)>,
    limit: i64,
) -> Result<Vec<JobRow>, sqlx::Error> {
    let (after_at, after_id) = after.map_or((None, None), |(a, i)| (Some(a), Some(i)));
    sqlx::query_as!(
        JobRow,
        "select id, kind, status, progress, attempts, max_attempts, storage_waits, blocked_reason, failure_code,
                failure_detail, variant_id, image_id, payload, created_at, started_at, finished_at, run_after
         from jobs
         where ($1::text is null or status = $1) and ($2::text is null or kind = $2)
           and ($3::timestamptz is null or (created_at, id) < ($3, $4::uuid))
         order by created_at desc, id desc limit $5",
        status,
        kind,
        after_at,
        after_id,
        limit
    )
    .fetch_all(db)
    .await
}

pub async fn job(db: &PgPool, id: Uuid) -> Result<Option<JobRow>, sqlx::Error> {
    sqlx::query_as!(
        JobRow,
        "select id, kind, status, progress, attempts, max_attempts, storage_waits, blocked_reason, failure_code,
                failure_detail, variant_id, image_id, payload, created_at, started_at, finished_at, run_after
         from jobs where id = $1",
        id
    )
    .fetch_optional(db)
    .await
}

/// Puts a failed or cancelled job back with fresh attempts. `None` when no such job exists,
/// `Some(false)` when it is not in a state that can be retried.
pub async fn retry_job(db: &mut PgConnection, id: Uuid) -> Result<Option<bool>, sqlx::Error> {
    let status = sqlx::query_scalar!("select status from jobs where id = $1 for update", id)
        .fetch_optional(&mut *db)
        .await?;
    match status.as_deref() {
        None => Ok(None),
        Some("failed" | "cancelled") => {
            sqlx::query!(
                "update jobs set status = 'queued', attempts = 0, run_after = now(), failure_code = null,
                        failure_detail = null, finished_at = null, blocked_reason = null, progress = null
                 where id = $1",
                id
            )
            .execute(&mut *db)
            .await?;
            sqlx::query!("select pg_notify('r16a_stream_jobs', 'retry')")
                .execute(db)
                .await?;
            Ok(Some(true))
        }
        Some(_) => Ok(Some(false)),
    }
}

/// The transcode job's variant and upload, for reservations on retry and cancel.
pub struct TranscodeRef {
    pub variant_id: Uuid,
    pub upload_id: Uuid,
}

pub async fn transcode_ref(db: &mut PgConnection, job_id: Uuid) -> Result<Option<TranscodeRef>, sqlx::Error> {
    sqlx::query_as!(
        TranscodeRef,
        r#"select variant_id as "variant_id!", (payload ->> 'uploadId')::uuid as "upload_id!" from jobs
           where id = $1 and kind = 'transcode' and variant_id is not null and payload ? 'uploadId'"#,
        job_id
    )
    .fetch_optional(db)
    .await
}

/// Releases the reservation of a transcode that ended without running, and shows the variant
/// as failed unless an earlier attempt keeps it playable.
pub async fn release_transcode(db: &mut PgConnection, t: &TranscodeRef) -> Result<(), sqlx::Error> {
    sqlx::query!("update uploads set reserved_bytes = 0 where id = $1", t.upload_id)
        .execute(&mut *db)
        .await?;
    sqlx::query!(
        "update media_variants set status = 'failed', updated_at = now() where id = $1 and status <> 'ready'",
        t.variant_id
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Cancels a queued job at once, asks a running one to stop (SPEC 7.2). `Some(Some(status))`
/// with the new status, `Some(None)` when the job cannot be cancelled, `None` when it does
/// not exist.
pub async fn cancel_job(db: &mut PgConnection, id: Uuid) -> Result<Option<Option<String>>, sqlx::Error> {
    let r = sqlx::query_scalar!(
        "update jobs set
           status = case when status = 'queued' then 'cancelled' else 'cancelling' end,
           finished_at = case when status = 'queued' then now() else finished_at end
         where id = $1 and status in ('queued', 'running') returning status",
        id
    )
    .fetch_optional(&mut *db)
    .await?;
    if r.is_some() {
        return Ok(Some(r));
    }
    let exists = sqlx::query_scalar!(r#"select exists (select 1 from jobs where id = $1) as "e!""#, id)
        .fetch_one(db)
        .await?;
    Ok(exists.then_some(None))
}

// ---------------------------------------------------------------- storage view

pub struct TitleUsage {
    pub id: Uuid,
    pub name: String,
    pub bytes: i64,
}

/// Bytes per title from our records, largest first, keyset on `(bytes, id)` (SPEC 7.10).
pub async fn usage_by_title(
    db: &PgPool,
    after: Option<(i64, Uuid)>,
    limit: i64,
) -> Result<Vec<TitleUsage>, sqlx::Error> {
    let (after_bytes, after_id) = after.map_or((None, None), |(a, i)| (Some(a), Some(i)));
    sqlx::query_as!(
        TitleUsage,
        r#"with usage as (
             select t.id, t.name,
                    (coalesce((select sum(a.total_bytes) from transcode_attempts a
                               join media_variants v on v.id = a.variant_id
                               where v.title_id = t.id and a.status in ('ready', 'running')), 0)
                   + coalesce((select sum(u.size_bytes) from uploads u
                               join media_variants v on v.id = u.variant_id
                               where v.title_id = t.id and u.status = 'completed'
                                 and u.source_deleted_at is null), 0)
                   + coalesce((select sum(iv.byte_size) from image_variants iv
                               where iv.image_id in (t.poster_image_id, t.backdrop_image_id)), 0))::bigint as bytes
             from titles t)
           select id, name, bytes as "bytes!" from usage
           where ($1::bigint is null or (bytes, id) < ($1, $2::uuid))
           order by bytes desc, id desc limit $3"#,
        after_bytes,
        after_id,
        limit
    )
    .fetch_all(db)
    .await
}

pub struct HealthRow {
    pub component: String,
    pub instance: String,
    pub storage_writable: bool,
    pub encoder: Option<String>,
    pub fresh: bool,
}

pub async fn component_health(db: &PgPool) -> Result<Vec<HealthRow>, sqlx::Error> {
    sqlx::query_as!(
        HealthRow,
        r#"select component, instance, storage_writable, encoder,
                  checked_at > now() - interval '5 minutes' as "fresh!"
           from component_health order by component, instance"#
    )
    .fetch_all(db)
    .await
}

/// Re-checks a retention candidate and locks its variant, so a re-transcode queued meanwhile
/// cannot lose its source. Re-transcode takes the same row lock.
pub async fn lock_retention_candidate(
    db: &mut PgConnection,
    upload_id: Uuid,
    delay_secs: f64,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query_scalar!(
        "select v.id from uploads u
         join media_variants v on v.id = u.variant_id
         join transcode_attempts a on a.id = v.current_attempt_id
         where u.id = $1 and u.status = 'completed' and u.source_deleted_at is null
           and a.upload_id = u.id and a.status = 'ready' and a.verified_at is not null
           and a.verified_at <= now() - make_interval(secs => $2)
           and not exists (select 1 from jobs j where j.variant_id = v.id
                           and j.status in ('queued', 'running', 'cancelling'))
         for update of v",
        upload_id,
        delay_secs
    )
    .fetch_optional(db)
    .await?
    .is_some())
}

// ---------------------------------------------------------------- manual images

pub struct NewImageUpload<'a> {
    pub id: Uuid,
    pub image_id: Uuid,
    pub kind: &'a str,
    pub title_id: Option<Uuid>,
    pub episode_id: Option<Uuid>,
    pub object_key: &'a str,
    pub s3_upload_id: &'a str,
    pub size_bytes: i64,
    pub created_by: Uuid,
}

pub async fn insert_image_upload(db: &mut PgConnection, u: NewImageUpload<'_>) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "insert into uploads (id, purpose, image_id, image_kind, image_title_id, image_episode_id, object_key,
                              s3_upload_id, size_bytes, part_size, part_count, created_by, expires_at)
         values ($1, 'image_source', $2, $3, $4, $5, $6, $7, $8, 52428800, 1, $9, now() + interval '24 hours')",
        u.id,
        u.image_id,
        u.kind,
        u.title_id,
        u.episode_id,
        u.object_key,
        u.s3_upload_id,
        u.size_bytes,
        u.created_by,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Whether the target of an image still exists, title for posters and backdrops, episode for
/// stills.
pub async fn image_target_exists(
    db: &mut PgConnection,
    title_id: Option<Uuid>,
    episode_id: Option<Uuid>,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"select (exists (select 1 from titles where id = $1) or exists (select 1 from episodes where id = $2)) as "e!""#,
        title_id,
        episode_id
    )
    .fetch_one(db)
    .await
}

/// Points the title or episode at the new image. The replaced image is left to encode GC.
pub async fn attach_image(
    db: &mut PgConnection,
    kind: &str,
    title_id: Option<Uuid>,
    episode_id: Option<Uuid>,
    image_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let n = match (kind, title_id, episode_id) {
        ("poster", Some(t), _) => sqlx::query!(
            "update titles set poster_image_id = $2, updated_at = now() where id = $1",
            t,
            image_id
        )
        .execute(db)
        .await?
        .rows_affected(),
        ("backdrop", Some(t), _) => sqlx::query!(
            "update titles set backdrop_image_id = $2, updated_at = now() where id = $1",
            t,
            image_id
        )
        .execute(db)
        .await?
        .rows_affected(),
        ("still", _, Some(e)) => sqlx::query!(
            "update episodes set still_image_id = $2, updated_at = now() where id = $1",
            e,
            image_id
        )
        .execute(db)
        .await?
        .rows_affected(),
        _ => 0,
    };
    Ok(n == 1)
}

/// Removes the pending image of an upload that never completed.
pub async fn delete_pending_image(db: &mut PgConnection, image_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "delete from images where id = $1 and status = 'pending' and source_bytes is null",
        image_id
    )
    .execute(db)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------- subtitles

pub async fn variant_exists(db: &mut PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"select exists (select 1 from media_variants where id = $1) as "e!""#,
        id
    )
    .fetch_one(db)
    .await
}

pub async fn subtitle_exists(
    db: &mut PgConnection,
    variant_id: Uuid,
    language: &str,
    kind: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"select exists (select 1 from subtitle_tracks where variant_id = $1 and language = $2 and kind = $3) as "e!""#,
        variant_id,
        language,
        kind
    )
    .fetch_one(db)
    .await
}

pub struct NewSubtitle<'a> {
    pub id: Uuid,
    pub variant_id: Uuid,
    pub language: &'a str,
    pub label: Option<&'a str>,
    pub kind: &'a str,
    pub staged_key: &'a str,
}

/// Inserts the track and its promotion job, returns the job id.
pub async fn insert_subtitle(db: &mut PgConnection, t: NewSubtitle<'_>) -> Result<Uuid, sqlx::Error> {
    sqlx::query!(
        "insert into subtitle_tracks (id, variant_id, language, label, kind, staged_key) values ($1, $2, $3, $4, $5, $6)",
        t.id,
        t.variant_id,
        t.language,
        t.label,
        t.kind,
        t.staged_key
    )
    .execute(&mut *db)
    .await?;
    let job = sqlx::query_scalar!(
        "insert into jobs (kind, payload, variant_id, dedupe_key, priority)
         values ('subtitle', jsonb_build_object('subtitleId', $1::uuid), $2, 'subtitle:' || $1::uuid::text, 5)
         returning id",
        t.id,
        t.variant_id
    )
    .fetch_one(&mut *db)
    .await?;
    sqlx::query!("select pg_notify('r16a_stream_jobs', 'subtitle')")
        .execute(db)
        .await?;
    Ok(job)
}

pub struct SubtitleRow {
    pub status: String,
    pub staged_key: String,
}

pub async fn subtitle_for_update(db: &mut PgConnection, id: Uuid) -> Result<Option<SubtitleRow>, sqlx::Error> {
    sqlx::query_as!(
        SubtitleRow,
        "select status, staged_key from subtitle_tracks where id = $1 for update",
        id
    )
    .fetch_optional(db)
    .await
}

/// Deletes the track and cancels a promotion that has not started. A running one finds the row
/// gone and ends as invalid input.
pub async fn delete_subtitle(db: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update jobs set status = 'cancelled', finished_at = now()
         where kind = 'subtitle' and status = 'queued' and payload ->> 'subtitleId' = $1::uuid::text",
        id
    )
    .execute(&mut *db)
    .await?;
    sqlx::query!("delete from subtitle_tracks where id = $1", id)
        .execute(db)
        .await?;
    Ok(())
}

pub struct StagedSubtitle {
    pub id: Uuid,
    pub staged_key: String,
}

/// Tracks whose staged file is no longer needed, promoted or refused by the worker.
pub async fn staged_subtitles(db: &PgPool) -> Result<Vec<StagedSubtitle>, sqlx::Error> {
    sqlx::query_as!(
        StagedSubtitle,
        "select id, staged_key from subtitle_tracks
         where staged_removed_at is null and status in ('ready', 'failed') limit 100"
    )
    .fetch_all(db)
    .await
}

pub async fn mark_staged_removed(db: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!("update subtitle_tracks set staged_removed_at = now() where id = $1", id)
        .execute(db)
        .await?;
    Ok(())
}
