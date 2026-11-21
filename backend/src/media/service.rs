//! Rules of variant, upload, job and storage administration (SPEC 4.5, 7.1, 7.2, 7.9, 7.10).

use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::{
    dto::{
        AdminJob, AdminJobPage, AdminVariant, CompletedPart, ComponentHealthDto, CursorQuery, ImageUploadStart,
        ImageUploadStarted, JobListQuery, JobSummary, MAX_IMAGE_BYTES, MAX_PART_URLS, MAX_SUBTITLE_BYTES, NodeCapacity,
        PART_SIZE, PartUrl, PartUrls, PartsRequest, StorageOverview, StoredPartDto, SubtitleQuery, TitleStorage,
        TitleUsageDto, TitleUsagePage, UploadComplete, UploadStart, UploadStarted, UploadState, VariantCreate,
        is_webvtt, normalize_language, part_count, valid_etag,
    },
    repository as repo,
};
use crate::{
    admin::audit,
    capacity::{self, repository as cap_repo},
    catalog::service::Actor,
    config::StoragePolicy,
    error::AppError,
    http::cursor,
    images::repository as image_repo,
    state::AppState,
    storage::{BucketName, StorageError},
};

/// Object key of an uploaded source, `sources/<variant>/<upload>` (SPEC 10.4).
pub fn source_key(variant_id: Uuid, upload_id: Uuid) -> String {
    format!("sources/{variant_id}/{upload_id}")
}

/// Storage failures in the API. Retryable ones are a degraded store the admin can wait out,
/// the rest are configuration errors and logged.
fn storage_error(e: &StorageError, what: &'static str) -> AppError {
    tracing::warn!(error = %e, what, "storage call failed");
    if e.is_retryable() {
        AppError::StorageDegraded
    } else {
        AppError::Internal(format!("{what} failed"))
    }
}

fn writable(state: &AppState) -> Result<(), AppError> {
    if state.storage_writable() {
        Ok(())
    } else {
        Err(AppError::StorageDegraded)
    }
}

fn i64_of(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn u64_of(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

async fn audit(
    db: &mut PgConnection,
    actor: Actor<'_>,
    action: &str,
    kind: &str,
    id: Uuid,
    details: serde_json::Value,
) -> Result<(), AppError> {
    audit::record(
        db,
        audit::Entry {
            actor_id: actor.user_id,
            action,
            target_kind: kind,
            target_id: Some(id.to_string()),
            details,
            request_id: actor.request_id,
        },
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------- variants

pub async fn create_variant(db: &PgPool, actor: Actor<'_>, input: VariantCreate) -> Result<Uuid, AppError> {
    let language = normalize_language(&input.language).ok_or(AppError::BadRequest("invalid-language"))?;
    let mut tx = db.begin().await?;
    let shape = repo::title_shape(&mut tx, input.title_id)
        .await?
        .ok_or(AppError::NotFound)?;
    match (shape.kind.as_str(), input.episode_id) {
        ("movie", None) => {}
        ("show", Some(e)) => {
            repo::episode_runtime(&mut tx, input.title_id, e)
                .await?
                .ok_or(AppError::NotFound)?;
        }
        ("movie", Some(_)) => return Err(AppError::BadRequest("movie-has-no-episodes")),
        _ => return Err(AppError::BadRequest("episode-required")),
    }
    let id = repo::insert_variant(&mut tx, input.title_id, &shape.kind, input.episode_id, &language)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(d) if d.is_unique_violation() => AppError::Conflict("variant-exists"),
            _ => e.into(),
        })?;
    audit(
        &mut tx,
        actor,
        "variant.create",
        "variant",
        id,
        json!({ "titleId": input.title_id, "episodeId": input.episode_id, "language": language }),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Removes a variant with its sources. Encodes in the media bucket are left to encode GC, which
/// deletes prefixes no variant references (SPEC 7.6).
pub async fn delete_variant(state: &AppState, actor: Actor<'_>, id: Uuid) -> Result<(), AppError> {
    let mut tx = state.db.begin().await?;
    let v = repo::variant_for_update(&mut tx, id).await?.ok_or(AppError::NotFound)?;
    if repo::has_active_job(&mut tx, id).await? {
        return Err(AppError::Conflict("job-active"));
    }
    let sources = repo::variant_sources(&mut tx, id).await?;
    if !sources.is_empty() {
        writable(state)?;
    }
    // Garage first. Should the database step then fail, the row survives and a retry deletes
    // nothing twice, because both calls treat a missing object as done.
    for s in &sources {
        let r = if s.status == "initiated" {
            state
                .storage
                .abort_multipart(BucketName::Uploads, &s.object_key, &s.s3_upload_id)
                .await
        } else {
            state.storage.delete_object(BucketName::Uploads, &s.object_key).await
        };
        match r {
            Ok(()) | Err(StorageError::NotFound) => {}
            Err(e) => return Err(storage_error(&e, "deleting a source")),
        }
    }
    repo::delete_variant(&mut tx, id).await?;
    audit(
        &mut tx,
        actor,
        "variant.delete",
        "variant",
        id,
        json!({ "titleId": v.title_id, "episodeId": v.episode_id, "sources": sources.len() }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn title_storage(db: &PgPool, title_id: Uuid) -> Result<TitleStorage, AppError> {
    let mut conn = db.acquire().await?;
    repo::title_shape(&mut conn, title_id)
        .await?
        .ok_or(AppError::NotFound)?;
    drop(conn);
    let variants = repo::admin_variants(db, title_id)
        .await?
        .into_iter()
        .map(|r| AdminVariant {
            id: r.id,
            episode_id: r.episode_id,
            language: r.language,
            status: r.status,
            duration_ms: r.duration_ms,
            current_bytes: r.current_bytes,
            source_bytes: r.source_bytes,
            source_kept: r.source_kept,
            open_upload_id: r.open_upload_id,
            job: r.job_id.map(|id| JobSummary {
                id,
                status: r.job_status.unwrap_or_default(),
                progress: r.job_progress,
                failure_detail: r.job_failure,
            }),
        })
        .collect();
    Ok(TitleStorage { variants })
}

// ---------------------------------------------------------------- admission

/// Free space for a new reservation, or the budget left while capacity is unknown.
enum Room {
    Known(u64),
    Unknown(u64),
}

async fn room(db: &mut PgConnection, policy: &StoragePolicy) -> Result<(Room, u64), AppError> {
    let reserved = u64_of(cap_repo::reserved(&mut *db).await?);
    let nodes = cap_repo::nodes(&mut *db, capacity::FRESH.as_secs_f64()).await?;
    let fresh: Vec<(u64, u16)> = nodes
        .iter()
        .filter(|n| n.fresh)
        .map(|n| (u64_of(n.data_avail), u16::try_from(n.replication).unwrap_or(2)))
        .collect();
    let free = if fresh.len() == nodes.len() {
        capacity::logical_free(&fresh, policy.metrics_endpoints.len())
    } else {
        None
    };
    Ok(match free {
        Some(f) => (
            Room::Known(f.saturating_sub(reserved).saturating_sub(policy.min_free_bytes)),
            reserved,
        ),
        None => (
            Room::Unknown(policy.unknown_budget_bytes.saturating_sub(reserved)),
            reserved,
        ),
    })
}

/// Admission of SPEC 7.10, inside the caller's transaction, serialised across replicas.
async fn admit(db: &mut PgConnection, policy: &StoragePolicy, needed: u64) -> Result<(), AppError> {
    cap_repo::lock_admission(&mut *db).await?;
    let (Room::Known(available) | Room::Unknown(available)) = room(db, policy).await?.0;
    if needed > available {
        return Err(AppError::InsufficientStorage { needed, available });
    }
    Ok(())
}

// ---------------------------------------------------------------- uploads

fn clean_file_name(raw: &str) -> Option<String> {
    let name: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_owned();
    (!name.is_empty() && name.chars().count() <= 255).then_some(name)
}

pub async fn start_upload(
    state: &AppState,
    actor: Actor<'_>,
    variant_id: Uuid,
    input: UploadStart,
) -> Result<UploadStarted, AppError> {
    let parts = part_count(input.size_bytes)?;
    let file_name = clean_file_name(&input.file_name).ok_or(AppError::BadRequest("invalid-file-name"))?;
    writable(state)?;

    let mut conn = state.db.acquire().await?;
    let v = repo::variant_for_update(&mut conn, variant_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if repo::has_open_upload(&mut conn, variant_id).await? || repo::has_active_job(&mut conn, variant_id).await? {
        return Err(AppError::Conflict("upload-in-progress"));
    }
    drop(conn);

    let duration = capacity::estimate_duration(input.duration_seconds, v.runtime_minutes);
    let ladder = capacity::ladder_bytes(duration, input.size_bytes);
    let needed = input.size_bytes.saturating_add(ladder);

    // Garage is asked outside the transaction, so the admission lock is never held across a
    // network call. A refused admission aborts the fresh multipart upload again.
    let upload_id = Uuid::new_v4();
    let key = source_key(variant_id, upload_id);
    let s3_id = state
        .storage
        .create_multipart(BucketName::Uploads, &key, "application/octet-stream")
        .await
        .map_err(|e| storage_error(&e, "starting an upload"))?;

    let result = async {
        let mut tx = state.db.begin().await?;
        admit(&mut tx, &state.config.storage, needed).await?;
        // Re-checked under the lock, two admins may race on one variant.
        if repo::has_open_upload(&mut tx, variant_id).await? {
            return Err(AppError::Conflict("upload-in-progress"));
        }
        repo::insert_upload(
            &mut tx,
            repo::NewUpload {
                id: upload_id,
                variant_id,
                object_key: &key,
                s3_upload_id: &s3_id,
                size_bytes: i64_of(input.size_bytes),
                part_size: i32::try_from(PART_SIZE).unwrap_or(i32::MAX),
                part_count: i32::try_from(parts).unwrap_or(i32::MAX),
                original_filename: Some(&file_name),
                estimated_output_bytes: i64_of(ladder),
                reserved_bytes: i64_of(needed),
                created_by: actor.user_id,
            },
        )
        .await?;
        audit(
            &mut tx,
            actor,
            "upload.start",
            "upload",
            upload_id,
            json!({ "variantId": variant_id, "sizeBytes": input.size_bytes, "reservedBytes": needed }),
        )
        .await?;
        tx.commit().await?;
        Ok::<_, AppError>(())
    }
    .await;
    if let Err(e) = result {
        if let Err(abort) = state.storage.abort_multipart(BucketName::Uploads, &key, &s3_id).await {
            // The cleanup task cannot see an upload without a row, the object stays until the
            // bucket lifecycle or encode GC finds it.
            tracing::warn!(error = %abort, "aborting a refused upload failed");
        }
        return Err(e);
    }
    Ok(UploadStarted {
        upload_id,
        part_size: PART_SIZE,
        part_count: parts,
    })
}

async fn open_upload(db: &PgPool, id: Uuid) -> Result<repo::UploadRow, AppError> {
    let mut conn = db.acquire().await?;
    let u = repo::upload(&mut conn, id, false).await?.ok_or(AppError::NotFound)?;
    if u.status != "initiated" || u.expired {
        return Err(AppError::Conflict("upload-not-open"));
    }
    Ok(u)
}

pub async fn part_urls(state: &AppState, id: Uuid, req: PartsRequest) -> Result<PartUrls, AppError> {
    if req.part_numbers.is_empty() || req.part_numbers.len() > MAX_PART_URLS {
        return Err(AppError::BadRequest("invalid-part-numbers"));
    }
    writable(state)?;
    let u = open_upload(&state.db, id).await?;
    let max = u16::try_from(u.part_count).unwrap_or(u16::MAX);
    if req.part_numbers.iter().any(|n| *n == 0 || *n > max) {
        return Err(AppError::BadRequest("invalid-part-numbers"));
    }
    let mut numbers = req.part_numbers;
    numbers.sort_unstable();
    numbers.dedup();
    Ok(PartUrls {
        parts: numbers
            .into_iter()
            .map(|n| PartUrl {
                part_number: n,
                url: state.upload_urls.part(&u.object_key, &u.s3_upload_id, n),
            })
            .collect(),
    })
}

pub async fn upload_state(state: &AppState, id: Uuid) -> Result<UploadState, AppError> {
    let mut conn = state.db.acquire().await?;
    let u = repo::upload(&mut conn, id, false).await?.ok_or(AppError::NotFound)?;
    drop(conn);
    let parts = if u.status == "initiated" && !u.expired {
        state
            .storage
            .list_parts(BucketName::Uploads, &u.object_key, &u.s3_upload_id)
            .await
            .map_err(|e| storage_error(&e, "listing parts"))?
            .into_iter()
            .map(|p| StoredPartDto {
                part_number: p.number,
                etag: p.etag,
                size_bytes: p.size,
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(UploadState {
        upload_id: u.id,
        status: if u.status == "initiated" && u.expired {
            "aborted".into()
        } else {
            u.status
        },
        size_bytes: u64_of(u.size_bytes),
        part_size: u64_of(i64::from(u.part_size)),
        part_count: u32::try_from(u.part_count).unwrap_or(0),
        parts,
    })
}

/// The browser's part list must be exactly `1..=part_count` in order with plausible part tags.
fn check_parts(parts: &[CompletedPart], part_count: i32) -> Result<Vec<String>, AppError> {
    let expected = usize::try_from(part_count).unwrap_or(0);
    if parts.len() != expected {
        return Err(AppError::BadRequest("parts-incomplete"));
    }
    parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if usize::from(p.part_number) != i + 1 || !valid_etag(&p.etag) {
                Err(AppError::BadRequest("invalid-parts"))
            } else {
                Ok(p.etag.clone())
            }
        })
        .collect()
}

pub async fn complete_upload(
    state: &AppState,
    actor: Actor<'_>,
    id: Uuid,
    req: UploadComplete,
) -> Result<Uuid, AppError> {
    writable(state)?;
    let u = open_upload(&state.db, id).await?;
    let etags = check_parts(&req.parts, u.part_count)?;
    state
        .storage
        .complete_multipart(BucketName::Uploads, &u.object_key, &u.s3_upload_id, &etags)
        .await
        .map_err(|e| match e {
            // Garage refuses a list that does not match what it holds, a client error.
            StorageError::Unexpected(400) => AppError::BadRequest("invalid-parts"),
            e => storage_error(&e, "completing an upload"),
        })?;
    let stored = state
        .storage
        .head_object(BucketName::Uploads, &u.object_key)
        .await
        .map_err(|e| storage_error(&e, "checking an upload"))?;
    let mut tx = state.db.begin().await?;
    let locked = repo::upload(&mut tx, id, true).await?.ok_or(AppError::NotFound)?;
    if locked.status != "initiated" {
        return Err(AppError::Conflict("upload-not-open"));
    }
    if stored != Some(u64_of(u.size_bytes)) {
        repo::mark_aborted(&mut tx, id).await?;
        audit(&mut tx, actor, "upload.size_mismatch", "upload", id, json!({})).await?;
        tx.commit().await?;
        if let Err(e) = state.storage.delete_object(BucketName::Uploads, &u.object_key).await {
            tracing::warn!(error = %e, "deleting a mismatched upload failed");
        }
        return Err(AppError::BadRequest("size-mismatch"));
    }
    repo::mark_completed(&mut tx, id).await?;
    if u.purpose == "image_source" {
        let job_id = finish_image(&mut tx, &u).await?;
        audit(&mut tx, actor, "image.upload", "upload", id, json!({ "jobId": job_id })).await?;
        tx.commit().await?;
        return Ok(job_id);
    }
    let variant_id = u.variant_id.ok_or(AppError::NotFound)?;
    repo::set_variant_processing(&mut tx, variant_id).await?;
    let job_id = repo::enqueue_transcode(&mut tx, variant_id, id)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(d) if d.is_unique_violation() => AppError::Conflict("job-active"),
            _ => e.into(),
        })?;
    audit(
        &mut tx,
        actor,
        "upload.complete",
        "upload",
        id,
        json!({ "variantId": variant_id, "jobId": job_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(job_id)
}

/// Queues the image job and points the title or episode at the new image.
async fn finish_image(tx: &mut PgConnection, u: &repo::UploadRow) -> Result<Uuid, AppError> {
    let (Some(image_id), Some(kind)) = (u.image_id, u.image_kind.as_deref()) else {
        return Err(AppError::NotFound);
    };
    image_repo::set_source_bytes(&mut *tx, image_id, u.size_bytes).await?;
    let job_id = image_repo::enqueue(&mut *tx, image_id).await?;
    if !repo::attach_image(&mut *tx, kind, u.image_title_id, u.image_episode_id, image_id).await? {
        return Err(AppError::NotFound);
    }
    Ok(job_id)
}

/// Starts the single-part upload of a manual poster, backdrop or still (SPEC 8.2).
pub async fn start_image_upload(
    state: &AppState,
    actor: Actor<'_>,
    input: ImageUploadStart,
) -> Result<ImageUploadStarted, AppError> {
    match (input.kind.as_str(), input.title_id, input.episode_id) {
        ("poster" | "backdrop", Some(_), None) | ("still", None, Some(_)) => {}
        _ => return Err(AppError::BadRequest("invalid-image-target")),
    }
    if !matches!(input.content_type.as_str(), "image/jpeg" | "image/png" | "image/webp") {
        return Err(AppError::BadRequest("invalid-content-type"));
    }
    if input.size_bytes == 0 || input.size_bytes > MAX_IMAGE_BYTES {
        return Err(AppError::BadRequest("invalid-size"));
    }
    writable(state)?;
    let mut conn = state.db.acquire().await?;
    if !repo::image_target_exists(&mut conn, input.title_id, input.episode_id).await? {
        return Err(AppError::NotFound);
    }
    drop(conn);

    let image_id = Uuid::new_v4();
    let upload_id = Uuid::new_v4();
    let key = crate::images::source_key(image_id);
    let s3_id = state
        .storage
        .create_multipart(BucketName::Uploads, &key, &input.content_type)
        .await
        .map_err(|e| storage_error(&e, "starting an image upload"))?;
    let result = async {
        let mut tx = state.db.begin().await?;
        // Images reserve nothing, they are small, but they still do not start on a full store.
        admit(&mut tx, &state.config.storage, input.size_bytes.saturating_mul(2)).await?;
        image_repo::insert_pending(&mut tx, image_id, &input.kind, &key, None).await?;
        repo::insert_image_upload(
            &mut tx,
            repo::NewImageUpload {
                id: upload_id,
                image_id,
                kind: &input.kind,
                title_id: input.title_id,
                episode_id: input.episode_id,
                object_key: &key,
                s3_upload_id: &s3_id,
                size_bytes: i64_of(input.size_bytes),
                created_by: actor.user_id,
            },
        )
        .await?;
        audit(
            &mut tx,
            actor,
            "image.upload_start",
            "upload",
            upload_id,
            json!({ "kind": input.kind, "titleId": input.title_id, "episodeId": input.episode_id }),
        )
        .await?;
        tx.commit().await?;
        Ok::<_, AppError>(())
    }
    .await;
    if let Err(e) = result {
        if let Err(abort) = state.storage.abort_multipart(BucketName::Uploads, &key, &s3_id).await {
            tracing::warn!(error = %abort, "aborting a refused image upload failed");
        }
        return Err(e);
    }
    Ok(ImageUploadStarted {
        upload_id,
        image_id,
        part_url: state.upload_urls.part(&key, &s3_id, 1),
    })
}

// ---------------------------------------------------------------- subtitles

/// Object key of a staged subtitle in the uploads bucket (SPEC 10.4).
pub fn subtitle_key(id: Uuid) -> String {
    format!("subtitles/{id}.vtt")
}

fn clean_label(raw: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(raw) = raw else { return Ok(None) };
    let label: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_owned();
    match label.chars().count() {
        0 => Ok(None),
        1..=60 => Ok(Some(label)),
        _ => Err(AppError::BadRequest("invalid-label")),
    }
}

/// Stages a WebVTT file in the uploads bucket and queues its promotion to media.
pub async fn create_subtitle(
    state: &AppState,
    actor: Actor<'_>,
    variant_id: Uuid,
    q: SubtitleQuery,
    body: &[u8],
) -> Result<Uuid, AppError> {
    let language = normalize_language(&q.language).ok_or(AppError::BadRequest("invalid-language"))?;
    let kind = q.kind.unwrap_or_else(|| "subtitles".to_owned());
    if !matches!(kind.as_str(), "subtitles" | "captions" | "forced") {
        return Err(AppError::BadRequest("invalid-kind"));
    }
    let label = clean_label(q.label.as_deref())?;
    if body.is_empty() || body.len() > MAX_SUBTITLE_BYTES || !is_webvtt(body) {
        return Err(AppError::BadRequest("invalid-webvtt"));
    }
    writable(state)?;
    let mut conn = state.db.acquire().await?;
    if !repo::variant_exists(&mut conn, variant_id).await? {
        return Err(AppError::NotFound);
    }
    if repo::subtitle_exists(&mut conn, variant_id, &language, &kind).await? {
        return Err(AppError::Conflict("subtitle-exists"));
    }
    drop(conn);
    let id = Uuid::new_v4();
    let key = subtitle_key(id);
    state
        .storage
        .put_object(BucketName::Uploads, &key, body, "text/vtt; charset=utf-8", None)
        .await
        .map_err(|e| storage_error(&e, "staging a subtitle"))?;
    let result = async {
        let mut tx = state.db.begin().await?;
        let job_id = repo::insert_subtitle(
            &mut tx,
            repo::NewSubtitle {
                id,
                variant_id,
                language: &language,
                label: label.as_deref(),
                kind: &kind,
                staged_key: &key,
            },
        )
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(d) if d.is_unique_violation() => AppError::Conflict("subtitle-exists"),
            _ => e.into(),
        })?;
        audit(
            &mut tx,
            actor,
            "subtitle.create",
            "subtitle",
            id,
            json!({ "variantId": variant_id, "language": language, "kind": kind, "jobId": job_id }),
        )
        .await?;
        tx.commit().await?;
        Ok::<_, AppError>(())
    }
    .await;
    if let Err(e) = result {
        if let Err(del) = state.storage.delete_object(BucketName::Uploads, &key).await {
            tracing::warn!(error = %del, "removing a refused subtitle failed");
        }
        return Err(e);
    }
    Ok(id)
}

/// Removes a track. A staged file is deleted here, a promoted one by encode GC, which keeps
/// only subtitle objects that a track references.
pub async fn delete_subtitle(state: &AppState, actor: Actor<'_>, id: Uuid) -> Result<(), AppError> {
    let mut tx = state.db.begin().await?;
    let t = repo::subtitle_for_update(&mut tx, id)
        .await?
        .ok_or(AppError::NotFound)?;
    if t.status != "ready" {
        match state.storage.delete_object(BucketName::Uploads, &t.staged_key).await {
            Ok(()) | Err(StorageError::NotFound) => {}
            Err(e) => return Err(storage_error(&e, "deleting a staged subtitle")),
        }
    }
    repo::delete_subtitle(&mut tx, id).await?;
    audit(&mut tx, actor, "subtitle.delete", "subtitle", id, json!({})).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn abort_upload(state: &AppState, actor: Actor<'_>, id: Uuid) -> Result<(), AppError> {
    let mut conn = state.db.acquire().await?;
    let u = repo::upload(&mut conn, id, false).await?.ok_or(AppError::NotFound)?;
    drop(conn);
    if u.status != "initiated" {
        return Err(AppError::Conflict("upload-not-open"));
    }
    state
        .storage
        .abort_multipart(BucketName::Uploads, &u.object_key, &u.s3_upload_id)
        .await
        .map_err(|e| storage_error(&e, "aborting an upload"))?;
    let mut tx = state.db.begin().await?;
    repo::mark_aborted(&mut tx, id).await?;
    audit(&mut tx, actor, "upload.abort", "upload", id, json!({})).await?;
    tx.commit().await?;
    Ok(())
}

/// A new attempt from the kept source. Reserves the ladder only, the source already counts.
pub async fn retranscode(state: &AppState, actor: Actor<'_>, variant_id: Uuid) -> Result<Uuid, AppError> {
    let mut tx = state.db.begin().await?;
    let v = repo::variant_for_update(&mut tx, variant_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if repo::has_active_job(&mut tx, variant_id).await? {
        return Err(AppError::Conflict("job-active"));
    }
    let upload_id = repo::usable_source(&mut tx, variant_id)
        .await?
        .ok_or(AppError::Conflict("source_unavailable"))?;
    let u = repo::upload(&mut tx, upload_id, false)
        .await?
        .ok_or(AppError::NotFound)?;
    let duration = capacity::estimate_duration(None, v.runtime_minutes);
    let ladder = capacity::ladder_bytes(duration, u64_of(u.size_bytes));
    admit(&mut tx, &state.config.storage, ladder).await?;
    repo::add_reservation(&mut tx, upload_id, i64_of(ladder)).await?;
    repo::set_variant_processing(&mut tx, variant_id).await?;
    let job_id = repo::enqueue_transcode(&mut tx, variant_id, upload_id).await?;
    audit(
        &mut tx,
        actor,
        "variant.retranscode",
        "variant",
        variant_id,
        json!({ "jobId": job_id, "uploadId": upload_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(job_id)
}

// ---------------------------------------------------------------- jobs

const JOB_STATUSES: &[&str] = &["queued", "running", "cancelling", "succeeded", "failed", "cancelled"];
const JOB_KINDS: &[&str] = &["transcode", "image", "subtitle", "encode_gc"];

fn job_dto(r: repo::JobRow) -> AdminJob {
    AdminJob {
        id: r.id,
        kind: r.kind,
        status: r.status,
        progress: r.progress,
        attempts: r.attempts,
        max_attempts: r.max_attempts,
        storage_waits: r.storage_waits,
        blocked_reason: r.blocked_reason,
        failure_code: r.failure_code,
        failure_detail: r.failure_detail,
        variant_id: r.variant_id,
        image_id: r.image_id,
        created_at: r.created_at,
        started_at: r.started_at,
        finished_at: r.finished_at,
        run_after: r.run_after,
    }
}

pub async fn jobs(db: &PgPool, q: JobListQuery) -> Result<AdminJobPage, AppError> {
    if q.status.as_deref().is_some_and(|s| !JOB_STATUSES.contains(&s)) {
        return Err(AppError::BadRequest("invalid-status"));
    }
    if q.kind.as_deref().is_some_and(|k| !JOB_KINDS.contains(&k)) {
        return Err(AppError::BadRequest("invalid-kind"));
    }
    let limit = q.limit.unwrap_or(30).clamp(1, 60);
    let after: Option<(time::OffsetDateTime, Uuid)> = cursor::decode(q.cursor.as_deref())?;
    let mut rows = repo::jobs(db, q.status.as_deref(), q.kind.as_deref(), after, limit + 1).await?;
    let next_cursor = if rows.len() > usize::try_from(limit).unwrap_or(0) {
        rows.truncate(usize::try_from(limit).unwrap_or(0));
        rows.last().map(|r| cursor::encode(&(r.created_at, r.id))).transpose()?
    } else {
        None
    };
    Ok(AdminJobPage {
        items: rows.into_iter().map(job_dto).collect(),
        next_cursor,
    })
}

pub async fn job(db: &PgPool, id: Uuid) -> Result<AdminJob, AppError> {
    repo::job(db, id).await?.map(job_dto).ok_or(AppError::NotFound)
}

/// Queues a failed or cancelled job again. A transcode reserves its ladder estimate again, as a
/// re-transcode does, because its reservation was released when it ended.
pub async fn retry_job(state: &AppState, actor: Actor<'_>, id: Uuid) -> Result<(), AppError> {
    let mut tx = state.db.begin().await?;
    if let Some(t) = repo::transcode_ref(&mut tx, id).await? {
        let v = repo::variant_for_update(&mut tx, t.variant_id)
            .await?
            .ok_or(AppError::NotFound)?;
        let u = repo::upload(&mut tx, t.upload_id, false)
            .await?
            .ok_or(AppError::NotFound)?;
        if u.source_deleted || u.status != "completed" {
            return Err(AppError::Conflict("source_unavailable"));
        }
        let ladder = capacity::ladder_bytes(
            capacity::estimate_duration(None, v.runtime_minutes),
            u64_of(u.size_bytes),
        );
        admit(&mut tx, &state.config.storage, ladder).await?;
        repo::add_reservation(&mut tx, t.upload_id, i64_of(ladder)).await?;
        repo::set_variant_processing(&mut tx, t.variant_id).await?;
    }
    match repo::retry_job(&mut tx, id).await {
        Ok(None) => return Err(AppError::NotFound),
        Ok(Some(false)) => return Err(AppError::Conflict("job-not-retryable")),
        Ok(Some(true)) => {}
        // A newer job for the same variant holds the dedupe key.
        Err(sqlx::Error::Database(d)) if d.is_unique_violation() => return Err(AppError::Conflict("job-active")),
        Err(e) => return Err(e.into()),
    }
    audit(&mut tx, actor, "job.retry", "job", id, json!({})).await?;
    tx.commit().await?;
    Ok(())
}

/// Cancels a job. A queued transcode releases its reservation here, a running one when the
/// worker stops it.
pub async fn cancel_job(db: &PgPool, actor: Actor<'_>, id: Uuid) -> Result<(), AppError> {
    let mut tx = db.begin().await?;
    match repo::cancel_job(&mut tx, id).await? {
        None => return Err(AppError::NotFound),
        Some(None) => return Err(AppError::Conflict("job-not-cancellable")),
        Some(Some(status)) => {
            if status == "cancelled"
                && let Some(t) = repo::transcode_ref(&mut tx, id).await?
            {
                repo::release_transcode(&mut tx, &t).await?;
            }
        }
    }
    audit(&mut tx, actor, "job.cancel", "job", id, json!({})).await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------- storage view

pub async fn overview(db: &PgPool, policy: &StoragePolicy) -> Result<StorageOverview, AppError> {
    let mut conn = db.acquire().await?;
    let nodes = cap_repo::nodes(&mut conn, capacity::FRESH.as_secs_f64()).await?;
    let (room, reserved) = room(&mut conn, policy).await?;
    drop(conn);
    let components = repo::component_health(db).await?;
    let (capacity_known, logical_free_bytes, unknown_reason) = match room {
        Room::Known(_) => {
            let fresh: Vec<(u64, u16)> = nodes
                .iter()
                .map(|n| (u64_of(n.data_avail), u16::try_from(n.replication).unwrap_or(2)))
                .collect();
            (
                true,
                capacity::logical_free(&fresh, policy.metrics_endpoints.len()),
                None,
            )
        }
        Room::Unknown(_) => (
            false,
            None,
            Some(
                if policy.metrics_endpoints.is_empty() {
                    "not_configured"
                } else {
                    "stale"
                }
                .to_owned(),
            ),
        ),
    };
    let meta_low = nodes.iter().any(|n| match (n.meta_avail, n.meta_total) {
        (Some(a), Some(t)) => a < t / 10,
        _ => false,
    });
    let warning = !capacity_known
        || meta_low
        || logical_free_bytes.is_some_and(|f| f.saturating_sub(reserved) < policy.warn_free_bytes);
    let degraded = components.iter().any(|c| c.fresh && !c.storage_writable);
    Ok(StorageOverview {
        capacity_known,
        unknown_reason,
        logical_free_bytes,
        reserved_bytes: reserved,
        warning,
        degraded,
        nodes: nodes
            .into_iter()
            .map(|n| NodeCapacity {
                node: n.node,
                data_avail_bytes: n.data_avail,
                data_total_bytes: n.data_total,
                meta_avail_bytes: n.meta_avail,
                meta_total_bytes: n.meta_total,
                fresh: n.fresh,
            })
            .collect(),
        components: components
            .into_iter()
            .map(|c| ComponentHealthDto {
                component: c.component,
                instance: c.instance,
                storage_writable: c.storage_writable,
                encoder: c.encoder,
                fresh: c.fresh,
            })
            .collect(),
    })
}

pub async fn usage(db: &PgPool, q: CursorQuery) -> Result<TitleUsagePage, AppError> {
    let limit = q.limit.unwrap_or(30).clamp(1, 60);
    let after: Option<(i64, Uuid)> = cursor::decode(q.cursor.as_deref())?;
    let mut rows = repo::usage_by_title(db, after, limit + 1).await?;
    let n = usize::try_from(limit).unwrap_or(0);
    let next_cursor = if rows.len() > n {
        rows.truncate(n);
        rows.last().map(|r| cursor::encode(&(r.bytes, r.id))).transpose()?
    } else {
        None
    };
    Ok(TitleUsagePage {
        items: rows
            .into_iter()
            .map(|r| TitleUsageDto {
                title_id: r.id,
                name: r.name,
                bytes: r.bytes,
            })
            .collect(),
        next_cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(n: u16) -> CompletedPart {
        CompletedPart {
            part_number: n,
            etag: format!("\"{n:032x}\""),
        }
    }

    #[test]
    fn completed_parts_must_be_the_full_ordered_list() {
        assert_eq!(check_parts(&[part(1), part(2)], 2).expect("ok").len(), 2);
        assert!(check_parts(&[part(1)], 2).is_err(), "missing part");
        assert!(check_parts(&[part(2), part(1)], 2).is_err(), "out of order");
        assert!(check_parts(&[part(1), part(1)], 2).is_err(), "duplicate");
    }

    #[test]
    fn file_names_lose_control_characters() {
        assert_eq!(clean_file_name(" movie\u{7}.mkv\n").as_deref(), Some("movie.mkv"));
        assert!(clean_file_name("\u{0}\u{1}").is_none());
    }

    #[test]
    fn source_keys_are_valid_object_keys() {
        assert!(crate::storage::valid_key(&source_key(Uuid::nil(), Uuid::max())));
    }
}
