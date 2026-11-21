//! SQL of the transcode worker, run as `r16a_stream_worker`. Every column here is in the grants
//! of SPEC 4.9, `tests/db_roles.rs` and `tests/transcode.rs` run it as that role.

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

pub struct VariantRow {
    pub id: Uuid,
    pub title_id: Uuid,
    pub title_kind: String,
    pub episode_id: Option<Uuid>,
    pub language: String,
    pub status: String,
}

pub async fn variant(db: &PgPool, id: Uuid) -> Result<Option<VariantRow>, sqlx::Error> {
    sqlx::query_as!(
        VariantRow,
        "select id, title_id, title_kind, episode_id, language, status from media_variants where id = $1",
        id
    )
    .fetch_optional(db)
    .await
}

pub struct SourceRow {
    pub id: Uuid,
    pub variant_id: Option<Uuid>,
    pub object_key: String,
    pub size_bytes: i64,
    pub status: String,
}

pub async fn source(db: &PgPool, upload_id: Uuid) -> Result<Option<SourceRow>, sqlx::Error> {
    sqlx::query_as!(
        SourceRow,
        "select id, variant_id, object_key, size_bytes, status from uploads
         where id = $1 and purpose = 'video_source'",
        upload_id
    )
    .fetch_optional(db)
    .await
}

pub async fn insert_attempt(
    db: &PgPool,
    id: Uuid,
    variant_id: Uuid,
    upload_id: Uuid,
    job_id: Uuid,
    encoder: &str,
    prefix: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "insert into transcode_attempts (id, variant_id, upload_id, job_id, encoder, storage_prefix)
         values ($1, $2, $3, $4, $5, $6)",
        id,
        variant_id,
        upload_id,
        job_id,
        encoder,
        prefix
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Ends an attempt that will not become current. `failed` for a run that failed, `abandoned`
/// for one cut short by a requeue or a cancel. Its objects are left for encode GC.
pub async fn end_attempt(db: &mut PgConnection, id: Uuid, status: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update transcode_attempts set status = $2, finished_at = now() where id = $1 and status = 'running'",
        id,
        status
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn mark_verified(db: &PgPool, id: Uuid, total_bytes: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update transcode_attempts set total_bytes = $2, verified_at = now() where id = $1",
        id,
        total_bytes
    )
    .execute(db)
    .await?;
    Ok(())
}

pub struct CapacityRow {
    pub data_avail: i64,
    pub replication: i16,
    pub recent: bool,
}

/// Capacity rows with whether each is younger than 10 minutes (SPEC 7.3 step 7).
pub async fn capacity(db: &PgPool) -> Result<Vec<CapacityRow>, sqlx::Error> {
    sqlx::query_as!(
        CapacityRow,
        r#"select data_avail, replication, checked_at > now() - interval '10 minutes' as "recent!"
           from garage_capacity"#
    )
    .fetch_all(db)
    .await
}

/// Ends the transcode of a variant that did not become ready. The reservation is released and
/// a variant without a playable attempt shows the failure.
pub async fn release(db: &mut PgConnection, variant_id: Uuid, upload_id: Option<Uuid>) -> Result<(), sqlx::Error> {
    if let Some(u) = upload_id {
        sqlx::query!("update uploads set reserved_bytes = 0 where id = $1", u)
            .execute(&mut *db)
            .await?;
    }
    sqlx::query!(
        "update media_variants set status = 'failed', updated_at = now() where id = $1 and status <> 'ready'",
        variant_id
    )
    .execute(db)
    .await?;
    Ok(())
}

pub struct RenditionRow {
    pub height: i16,
    pub width: i16,
    pub bandwidth: i32,
    pub average_bandwidth: i32,
    pub codecs: &'static str,
    pub frame_rate: f64,
    pub playlist_key: String,
}

pub struct Switch<'a> {
    pub job_id: Uuid,
    pub variant_id: Uuid,
    pub attempt_id: Uuid,
    pub upload_id: Uuid,
    pub duration_ms: i64,
    pub audio_channels: i16,
    pub audio_playlist_key: &'a str,
    pub scrub_vtt_key: &'a str,
    pub renditions: &'a [RenditionRow],
}

/// What the switch found when it took its locks.
#[derive(Debug, PartialEq, Eq)]
pub enum Switched {
    Done,
    /// An admin asked to cancel while the outputs were written.
    Cancelled,
    /// The job is no longer this worker's, the reaper requeued it.
    Lost,
    /// The variant was deleted meanwhile.
    Gone,
}

/// SPEC 7.3 step 10, in one transaction. The previous attempt stays for encode GC.
pub async fn switch(db: &PgPool, worker: &str, s: Switch<'_>) -> Result<Switched, sqlx::Error> {
    let mut tx = db.begin().await?;
    let Some(v) = sqlx::query!(
        "select title_id, title_kind, episode_id, language from media_variants where id = $1 for update",
        s.variant_id
    )
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(Switched::Gone);
    };
    let job = sqlx::query_scalar!(
        "select status from jobs where id = $1 and locked_by = $2 for update",
        s.job_id,
        worker
    )
    .fetch_optional(&mut *tx)
    .await?;
    match job.as_deref() {
        Some("running") => {}
        Some("cancelling") => return Ok(Switched::Cancelled),
        _ => return Ok(Switched::Lost),
    }
    for r in s.renditions {
        sqlx::query!(
            "insert into renditions (attempt_id, height, width, bandwidth, average_bandwidth, codecs, frame_rate,
                                     playlist_key)
             values ($1, $2, $3, $4, $5, $6, $7::float8::numeric(6, 3), $8)",
            s.attempt_id,
            r.height,
            r.width,
            r.bandwidth,
            r.average_bandwidth,
            r.codecs,
            r.frame_rate,
            r.playlist_key
        )
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query!(
        "update transcode_attempts set status = 'ready', duration_ms = $2, audio_codecs = 'mp4a.40.2',
                audio_channels = $3, audio_playlist_key = $4, scrub_vtt_key = $5, finished_at = now()
         where id = $1",
        s.attempt_id,
        s.duration_ms,
        s.audio_channels,
        s.audio_playlist_key,
        s.scrub_vtt_key
    )
    .execute(&mut *tx)
    .await?;
    // A new episode of a published show moves the show up in "recently added".
    let new_episode =
        match v.episode_id {
            Some(e) => !sqlx::query_scalar!(
                r#"select exists (select 1 from media_variants where episode_id = $1 and status = 'ready') as "e!""#,
                e
            )
            .fetch_one(&mut *tx)
            .await?,
            None => false,
        };
    sqlx::query!(
        "update media_variants set status = 'ready', current_attempt_id = $2, duration_ms = $3, ready_at = now(),
                updated_at = now()
         where id = $1",
        s.variant_id,
        s.attempt_id,
        s.duration_ms
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "insert into title_languages (title_id, language) values ($1, $2) on conflict do nothing",
        v.title_id,
        v.language
    )
    .execute(&mut *tx)
    .await?;
    if new_episode && v.title_kind == "show" {
        sqlx::query!(
            "update titles set listed_at = now() where id = $1 and status = 'published'",
            v.title_id
        )
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query!("update uploads set reserved_bytes = 0 where id = $1", s.upload_id)
        .execute(&mut *tx)
        .await?;
    crate::jobs::repository::succeed(&mut tx, s.job_id).await?;
    tx.commit().await?;
    Ok(Switched::Done)
}

/// Records which encoder this worker uses, for `/readyz` and the admin storage page (SPEC 7.5).
pub async fn report_encoder(db: &PgPool, instance: &str, encoder: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "insert into component_health (component, instance, storage_writable, encoder, checked_at)
         values ('worker', $1, false, $2, now())
         on conflict (component, instance) do update set encoder = excluded.encoder",
        instance,
        encoder
    )
    .execute(db)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------- subtitles

pub struct SubtitleRow {
    pub status: String,
    pub staged_key: String,
}

pub async fn subtitle(db: &PgPool, id: Uuid) -> Result<Option<SubtitleRow>, sqlx::Error> {
    sqlx::query_as!(
        SubtitleRow,
        "select status, staged_key from subtitle_tracks where id = $1",
        id
    )
    .fetch_optional(db)
    .await
}

pub async fn subtitle_ready(
    db: &mut PgConnection,
    id: Uuid,
    object_key: &str,
    bytes: i32,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query!(
        "update subtitle_tracks set status = 'ready', object_key = $2, byte_size = $3 where id = $1",
        id,
        object_key,
        bytes
    )
    .execute(db)
    .await?
    .rows_affected()
        == 1)
}

pub async fn subtitle_failed(db: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!("update subtitle_tracks set status = 'failed' where id = $1", id)
        .execute(db)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------- encode GC

/// Everything in the media bucket that must stay, by id (SPEC 7.6 step 2).
pub struct LiveSet {
    pub attempts: Vec<Uuid>,
    pub images: Vec<Uuid>,
    pub subtitles: Vec<Uuid>,
}

pub async fn live_set(db: &PgPool) -> Result<LiveSet, sqlx::Error> {
    let attempts = sqlx::query_scalar!(
        r#"select id as "id!" from transcode_attempts where status = 'running'
           union
           select current_attempt_id as "id!" from media_variants where current_attempt_id is not null"#
    )
    .fetch_all(db)
    .await?;
    let images = sqlx::query_scalar!("select id from images").fetch_all(db).await?;
    let subtitles = sqlx::query_scalar!("select id from subtitle_tracks")
        .fetch_all(db)
        .await?;
    Ok(LiveSet {
        attempts,
        images,
        subtitles,
    })
}

/// Re-checks one attempt under a share lock on its variant, so a switch cannot make it current
/// between the check and the commit. A switch only ever makes a new attempt current, so an
/// attempt found dead here stays dead.
pub async fn attempt_is_live(db: &PgPool, variant: Uuid, attempt: Uuid) -> Result<bool, sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query_scalar!(
        r#"select 1 as "one!" from media_variants where id = $1 for share"#,
        variant
    )
    .fetch_optional(&mut *tx)
    .await?;
    let live = sqlx::query_scalar!(
        r#"select exists (select 1 from transcode_attempts where id = $1 and status = 'running')
              or exists (select 1 from media_variants where current_attempt_id = $1) as "live!""#,
        attempt
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(live)
}

pub async fn image_exists(db: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(r#"select exists (select 1 from images where id = $1) as "e!""#, id)
        .fetch_one(db)
        .await
}

pub async fn subtitle_exists(db: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"select exists (select 1 from subtitle_tracks where id = $1) as "e!""#,
        id
    )
    .fetch_one(db)
    .await
}

/// Records the orphans of this round and forgets prefixes that are live again or gone.
pub async fn record_orphans(db: &PgPool, orphans: &[String]) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query!(
        "insert into gc_orphans (bucket, prefix) select 'media', p from unnest($1::text[]) p
         on conflict (bucket, prefix) do update set last_seen_at = now()",
        orphans
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "delete from gc_orphans where bucket = 'media' and not (prefix = any($1))",
        orphans
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Orphans first seen longer ago than the grace (SPEC 7.6 step 4).
pub async fn due_orphans(db: &PgPool, grace_secs: f64) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "select prefix from gc_orphans where bucket = 'media'
           and first_seen_at <= now() - make_interval(secs => $1) order by first_seen_at",
        grace_secs
    )
    .fetch_all(db)
    .await
}

pub async fn forget_orphan(db: &PgPool, prefix: &str) -> Result<(), sqlx::Error> {
    sqlx::query!("delete from gc_orphans where bucket = 'media' and prefix = $1", prefix)
        .execute(db)
        .await?;
    Ok(())
}

/// Ends this GC run and queues the next one in the same transaction, so exactly one stays
/// pending (SPEC 7.2).
pub async fn finish_gc(db: &PgPool, job_id: Uuid, next_secs: f64) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    crate::jobs::repository::succeed(&mut tx, job_id).await?;
    enqueue_gc(&mut tx, next_secs).await?;
    tx.commit().await?;
    Ok(())
}

/// Queues an encode GC run unless one is pending already.
pub async fn enqueue_gc(db: &mut PgConnection, after_secs: f64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "insert into jobs (kind, dedupe_key, run_after) values ('encode_gc', 'encode_gc', now() + make_interval(secs => $1))
         on conflict (dedupe_key) where status in ('queued', 'running', 'cancelling') do nothing",
        after_secs
    )
    .execute(db)
    .await?;
    Ok(())
}
