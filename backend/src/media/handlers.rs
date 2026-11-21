//! Admin routes for variants, uploads, jobs and storage. Every one takes [`Admin`] first, so
//! a member gets 404 before anything else is looked at. Upload part URLs are presigned and are
//! never logged, the access log carries only the route template.

use axum::{
    Extension, Json,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
};
use uuid::Uuid;

use super::{
    dto::{
        AdminJob, AdminJobPage, CursorQuery, ImageUploadStart, ImageUploadStarted, JobListQuery, JobRef,
        MAX_SUBTITLE_BYTES, PartUrls, PartsRequest, StorageOverview, SubtitleQuery, TitleStorage, TitleUsagePage,
        UploadComplete, UploadStart, UploadStarted, UploadState, VariantCreate,
    },
    service,
};
use crate::{
    auth::Admin,
    catalog::{dto::Created, service::Actor},
    error::AppError,
    http::{json::ApiJson, path::ApiPath, request_id::RequestId},
    state::AppState,
};

/// `POST /admin/uploads/{id}/complete` carries up to 1229 part tags (SPEC 5.2).
pub const COMPLETE_BODY_LIMIT: usize = 256 * 1024;

fn actor<'a>(admin: &Admin, rid: Option<&'a Extension<RequestId>>) -> Actor<'a> {
    Actor {
        user_id: admin.0.user_id,
        request_id: rid.map(|Extension(r)| r.0.as_str()),
    }
}

/// A language for a movie or an episode.
#[utoipa::path(
    post,
    path = "/api/v1/admin/variants",
    tag = "admin",
    request_body = VariantCreate,
    responses(
        (status = 201, description = "Created, awaiting its upload", body = Created),
        (status = 400, description = "Invalid language, or episode missing for a show"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such title or episode, or not an admin"),
        (status = 409, description = "This language exists already")
    )
)]
pub async fn create_variant(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiJson(input): ApiJson<VariantCreate>,
) -> Result<(StatusCode, Json<Created>), AppError> {
    let id = service::create_variant(&state.db, actor(&admin, rid.as_ref()), input).await?;
    Ok((StatusCode::CREATED, Json(Created { id })))
}

/// Removes a variant and its sources. Encodes go with encode GC.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/variants/{id}",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Variant id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such variant, or not an admin"),
        (status = 409, description = "A job runs for this variant"),
        (status = 503, description = "Storage degraded, writes paused")
    )
)]
pub async fn delete_variant(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    service::delete_variant(&state, actor(&admin, rid.as_ref()), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Variants of one title with their encode, source and latest job.
#[utoipa::path(
    get,
    path = "/api/v1/admin/titles/{id}/storage",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Title id")),
    responses(
        (status = 200, description = "Per-variant storage", body = TitleStorage),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "No such title, or not an admin")
    )
)]
pub async fn title_storage(
    State(state): State<AppState>,
    _admin: Admin,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<TitleStorage>, AppError> {
    let out = service::title_storage(&state.db, id).await?;
    Ok(Json(out))
}

/// Starts a multipart upload of a source after the space check (SPEC 7.10).
#[utoipa::path(
    post,
    path = "/api/v1/admin/variants/{id}/uploads",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Variant id")),
    request_body = UploadStart,
    responses(
        (status = 201, description = "Started", body = UploadStarted),
        (status = 400, description = "Invalid size or file name"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such variant, or not an admin"),
        (status = 409, description = "An upload or job is in progress for this variant"),
        (status = 507, description = "Not enough space, with neededBytes and availableBytes"),
        (status = 503, description = "Storage degraded, writes paused")
    )
)]
pub async fn start_upload(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<UploadStart>,
) -> Result<(StatusCode, Json<UploadStarted>), AppError> {
    let out = service::start_upload(&state, actor(&admin, rid.as_ref()), id, input).await?;
    Ok((StatusCode::CREATED, Json(out)))
}

/// Presigned part URLs on the app's origin, one hour, at most 50 per call.
#[utoipa::path(
    post,
    path = "/api/v1/admin/uploads/{id}/parts",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Upload id")),
    request_body = PartsRequest,
    responses(
        (status = 200, description = "Part URLs", body = PartUrls),
        (status = 400, description = "Invalid part numbers"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such upload, or not an admin"),
        (status = 409, description = "The upload is no longer open"),
        (status = 503, description = "Storage degraded, writes paused")
    )
)]
pub async fn part_urls(
    State(state): State<AppState>,
    _admin: Admin,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<PartsRequest>,
) -> Result<([(header::HeaderName, &'static str); 1], Json<PartUrls>), AppError> {
    let out = service::part_urls(&state, id, input).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(out)))
}

/// An upload with the parts Garage holds, for resuming.
#[utoipa::path(
    get,
    path = "/api/v1/admin/uploads/{id}",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Upload id")),
    responses(
        (status = 200, description = "Upload state", body = UploadState),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "No such upload, or not an admin"),
        (status = 503, description = "Storage degraded, writes paused")
    )
)]
pub async fn get_upload(
    State(state): State<AppState>,
    _admin: Admin,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<UploadState>, AppError> {
    let out = service::upload_state(&state, id).await?;
    Ok(Json(out))
}

/// Completes the upload, checks its size and queues the transcode.
#[utoipa::path(
    post,
    path = "/api/v1/admin/uploads/{id}/complete",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Upload id")),
    request_body = UploadComplete,
    responses(
        (status = 202, description = "Transcode queued", body = JobRef),
        (status = 400, description = "Parts missing or wrong, or the stored size differs"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such upload, or not an admin"),
        (status = 409, description = "The upload is no longer open, or a job runs"),
        (status = 503, description = "Storage degraded, writes paused")
    )
)]
pub async fn complete_upload(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<UploadComplete>,
) -> Result<(StatusCode, Json<JobRef>), AppError> {
    let job_id = service::complete_upload(&state, actor(&admin, rid.as_ref()), id, input).await?;
    Ok((StatusCode::ACCEPTED, Json(JobRef { job_id })))
}

/// Aborts an open upload and releases its reservation.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/uploads/{id}",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Upload id")),
    responses(
        (status = 204, description = "Aborted"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such upload, or not an admin"),
        (status = 409, description = "The upload is not open"),
        (status = 503, description = "Storage degraded, writes paused")
    )
)]
pub async fn abort_upload(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    service::abort_upload(&state, actor(&admin, rid.as_ref()), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A new attempt from the kept source.
#[utoipa::path(
    post,
    path = "/api/v1/admin/variants/{id}/retranscode",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Variant id")),
    responses(
        (status = 202, description = "Transcode queued", body = JobRef),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such variant, or not an admin"),
        (status = 409, description = "A job runs, or source_unavailable once retention deleted the source"),
        (status = 507, description = "Not enough space for the new encode")
    )
)]
pub async fn retranscode(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<(StatusCode, Json<JobRef>), AppError> {
    let job_id = service::retranscode(&state, actor(&admin, rid.as_ref()), id).await?;
    Ok((StatusCode::ACCEPTED, Json(JobRef { job_id })))
}

/// The job queue, newest first.
#[utoipa::path(
    get,
    path = "/api/v1/admin/jobs",
    tag = "admin",
    params(JobListQuery),
    responses(
        (status = 200, description = "One page of jobs", body = AdminJobPage),
        (status = 400, description = "Invalid filter or cursor"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "Not an admin")
    )
)]
pub async fn list_jobs(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<JobListQuery>,
) -> Result<Json<AdminJobPage>, AppError> {
    let out = service::jobs(&state.db, q).await?;
    Ok(Json(out))
}

/// One job with its progress and failure reason.
#[utoipa::path(
    get,
    path = "/api/v1/admin/jobs/{id}",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Job id")),
    responses(
        (status = 200, description = "The job", body = AdminJob),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "No such job, or not an admin")
    )
)]
pub async fn get_job(
    State(state): State<AppState>,
    _admin: Admin,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<AdminJob>, AppError> {
    let out = service::job(&state.db, id).await?;
    Ok(Json(out))
}

/// Queues a failed or cancelled job again with fresh attempts.
#[utoipa::path(
    post,
    path = "/api/v1/admin/jobs/{id}/retry",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Job id")),
    responses(
        (status = 204, description = "Queued"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such job, or not an admin"),
        (status = 409, description = "Not failed or cancelled, or a newer job runs")
    )
)]
pub async fn retry_job(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    service::retry_job(&state, actor(&admin, rid.as_ref()), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Cancels a queued job, or asks a running one to stop.
#[utoipa::path(
    post,
    path = "/api/v1/admin/jobs/{id}/cancel",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Job id")),
    responses(
        (status = 204, description = "Cancelled or cancelling"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such job, or not an admin"),
        (status = 409, description = "The job has ended")
    )
)]
pub async fn cancel_job(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    service::cancel_job(&state.db, actor(&admin, rid.as_ref()), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Capacity, reservations, warning and write health (SPEC 7.10).
#[utoipa::path(
    get,
    path = "/api/v1/admin/storage",
    tag = "admin",
    responses(
        (status = 200, description = "Storage overview", body = StorageOverview),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "Not an admin")
    )
)]
pub async fn storage(State(state): State<AppState>, _admin: Admin) -> Result<Json<StorageOverview>, AppError> {
    let out = service::overview(&state.db, &state.config.storage).await?;
    Ok(Json(out))
}

/// Bytes per title, largest first.
#[utoipa::path(
    get,
    path = "/api/v1/admin/storage/titles",
    tag = "admin",
    params(CursorQuery),
    responses(
        (status = 200, description = "One page of titles", body = TitleUsagePage),
        (status = 400, description = "Invalid cursor"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "Not an admin")
    )
)]
pub async fn storage_titles(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<CursorQuery>,
) -> Result<Json<TitleUsagePage>, AppError> {
    let out = service::usage(&state.db, q).await?;
    Ok(Json(out))
}

/// Body limit of `POST /admin/variants/{id}/subtitles`, the file plus nothing.
pub const SUBTITLE_BODY_LIMIT: usize = MAX_SUBTITLE_BYTES;

/// Starts a single-part upload of a manual poster or backdrop for a title, or a still for an
/// episode. Completing it through `/admin/uploads/{id}/complete` attaches the image.
#[utoipa::path(
    post,
    path = "/api/v1/admin/images",
    tag = "admin",
    request_body = ImageUploadStart,
    responses(
        (status = 201, description = "Started, with the part URL", body = ImageUploadStarted),
        (status = 400, description = "Invalid kind, target, type or size"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such title or episode, or not an admin"),
        (status = 507, description = "Not enough space"),
        (status = 503, description = "Storage degraded, writes paused")
    )
)]
pub async fn start_image_upload(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiJson(input): ApiJson<ImageUploadStart>,
) -> Result<
    (
        StatusCode,
        [(header::HeaderName, &'static str); 1],
        Json<ImageUploadStarted>,
    ),
    AppError,
> {
    let out = service::start_image_upload(&state, actor(&admin, rid.as_ref()), input).await?;
    Ok((StatusCode::CREATED, [(header::CACHE_CONTROL, "no-store")], Json(out)))
}

/// Adds a WebVTT track to a variant. The worker promotes it to the media bucket.
#[utoipa::path(
    post,
    path = "/api/v1/admin/variants/{id}/subtitles",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Variant id"), SubtitleQuery),
    request_body(content = String, content_type = "text/vtt", description = "The WebVTT file, at most 2 MiB"),
    responses(
        (status = 201, description = "Staged, promotion queued", body = Created),
        (status = 400, description = "Not WebVTT, or an invalid language, label or kind"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such variant, or not an admin"),
        (status = 409, description = "A track with this language and kind exists"),
        (status = 413, description = "Larger than 2 MiB"),
        (status = 503, description = "Storage degraded, writes paused")
    )
)]
pub async fn create_subtitle(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
    Query(q): Query<SubtitleQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Created>), AppError> {
    let vtt = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|t| t.trim().eq_ignore_ascii_case("text/vtt"))
        });
    if !vtt {
        return Err(AppError::BadRequest("invalid-content-type"));
    }
    let id = service::create_subtitle(&state, actor(&admin, rid.as_ref()), id, q, &body).await?;
    Ok((StatusCode::CREATED, Json(Created { id })))
}

/// Removes a subtitle track.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/subtitles/{id}",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Subtitle id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such track, or not an admin"),
        (status = 503, description = "Storage degraded, writes paused")
    )
)]
pub async fn delete_subtitle(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    service::delete_subtitle(&state, actor(&admin, rid.as_ref()), id).await?;
    Ok(StatusCode::NO_CONTENT)
}
