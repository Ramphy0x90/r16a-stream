//! Admin routes of the catalog. Every one takes [`Admin`] first, so a member gets 404 before any
//! path or body is looked at, and every mutation is audited by the service.

use axum::{
    Extension, Json,
    extract::{Query, State},
    http::StatusCode,
};
use uuid::Uuid;

use super::{
    dto::{
        AdminListQuery, AdminTitle, AdminTitlePage, Created, EpisodeInput, EpisodePatch, Genre, GenreCreate,
        SeasonInput, SeasonPatch, TitleInput, TitlePatch,
    },
    service::{self, Actor},
};
use crate::{
    auth::Admin,
    error::AppError,
    http::{json::ApiJson, path::ApiPath, request_id::RequestId},
    state::AppState,
};

/// Larger body limit for `POST /admin/titles`, which carries a whole show with its episodes.
pub const TITLE_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// Downloads a TMDB image into the uploads bucket before the title is written. Refused with
/// `503 storage_degraded` while the API's write probe fails (SPEC 5.2, 7.8).
async fn import(state: &AppState, path: Option<&str>) -> Result<Option<(Uuid, i64)>, AppError> {
    let Some(path) = path else { return Ok(None) };
    if !state.storage_writable() {
        return Err(AppError::StorageDegraded);
    }
    state.tmdb_images.import(&state.storage, path).await.map(Some)
}

fn actor<'a>(admin: &Admin, rid: Option<&'a Extension<RequestId>>) -> Actor<'a> {
    Actor {
        user_id: admin.0.user_id,
        request_id: rid.map(|Extension(r)| r.0.as_str()),
    }
}

/// Every title, drafts included, last changed first.
#[utoipa::path(
    get,
    path = "/api/v1/admin/titles",
    tag = "admin",
    params(AdminListQuery),
    responses(
        (status = 200, description = "One page of titles", body = AdminTitlePage),
        (status = 400, description = "Invalid filter or cursor"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "Not an admin")
    )
)]
pub async fn list(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<AdminListQuery>,
) -> Result<Json<AdminTitlePage>, AppError> {
    let page = service::admin_list(
        &state.db,
        q.status.as_deref(),
        q.q.as_deref(),
        q.cursor.as_deref(),
        q.limit,
    )
    .await?;
    Ok(Json(page))
}

/// One title with everything an admin edits.
#[utoipa::path(
    get,
    path = "/api/v1/admin/titles/{id}",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Title id")),
    responses(
        (status = 200, description = "The title", body = AdminTitle),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "No such title, or not an admin")
    )
)]
pub async fn get(
    State(state): State<AppState>,
    _admin: Admin,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<AdminTitle>, AppError> {
    Ok(Json(service::admin_title(&state.db, &state.image_urls, id).await?))
}

/// Creates a draft, optionally with seasons and episodes, for example from a TMDB draft.
#[utoipa::path(
    post,
    path = "/api/v1/admin/titles",
    tag = "admin",
    request_body = TitleInput,
    responses(
        (status = 201, description = "Created as a draft", body = Created),
        (status = 400, description = "Invalid field"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "Not an admin"),
        (status = 409, description = "The TMDB id, a season or an episode number exists already")
    )
)]
pub async fn create(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiJson(input): ApiJson<TitleInput>,
) -> Result<(StatusCode, Json<Created>), AppError> {
    input.validate()?;
    let imported = service::Imported {
        poster: import(&state, input.poster_path.as_deref()).await?.map(Some),
        backdrop: import(&state, input.backdrop_path.as_deref()).await?.map(Some),
    };
    let id = service::create_title(&state.db, actor(&admin, rid.as_ref()), &input, imported).await?;
    Ok((StatusCode::CREATED, Json(Created { id })))
}

/// Changes fields of a title. The kind never changes.
#[utoipa::path(
    patch,
    path = "/api/v1/admin/titles/{id}",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Title id")),
    request_body = TitlePatch,
    responses(
        (status = 204, description = "Saved"),
        (status = 400, description = "Invalid field"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such title, or not an admin")
    )
)]
pub async fn update(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(patch): ApiJson<TitlePatch>,
) -> Result<StatusCode, AppError> {
    patch.validate()?;
    let slot = |p: &Option<Option<String>>| p.as_ref().map(|inner| inner.as_deref().map(String::from));
    let imported = service::Imported {
        poster: match slot(&patch.poster_path) {
            None => None,
            Some(None) => Some(None),
            Some(Some(tmdb_path)) => Some(import(&state, Some(&tmdb_path)).await?),
        },
        backdrop: match slot(&patch.backdrop_path) {
            None => None,
            Some(None) => Some(None),
            Some(Some(tmdb_path)) => Some(import(&state, Some(&tmdb_path)).await?),
        },
    };
    service::update_title(&state.db, actor(&admin, rid.as_ref()), id, &patch, imported).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes a title with its seasons, episodes, genres and credits.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/titles/{id}",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Title id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such title, or not an admin")
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    service::delete_title(&state.db, actor(&admin, rid.as_ref()), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Makes a title visible to members. The first publish sets its place in Recently added.
#[utoipa::path(
    post,
    path = "/api/v1/admin/titles/{id}/publish",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Title id")),
    responses(
        (status = 204, description = "Published"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such title, or not an admin")
    )
)]
pub async fn publish(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    service::set_published(&state.db, actor(&admin, rid.as_ref()), id, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Hides a title from members again.
#[utoipa::path(
    post,
    path = "/api/v1/admin/titles/{id}/unpublish",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Title id")),
    responses(
        (status = 204, description = "Back to draft"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such title, or not an admin")
    )
)]
pub async fn unpublish(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    service::set_published(&state.db, actor(&admin, rid.as_ref()), id, false).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Adds a season, optionally with its episodes, to a show.
#[utoipa::path(
    post,
    path = "/api/v1/admin/titles/{id}/seasons",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Title id of a show")),
    request_body = SeasonInput,
    responses(
        (status = 201, description = "Created", body = Created),
        (status = 400, description = "Invalid field"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such title, or not an admin"),
        (status = 409, description = "The title is a movie, or the number exists already")
    )
)]
pub async fn create_season(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<SeasonInput>,
) -> Result<(StatusCode, Json<Created>), AppError> {
    let season = service::create_season(&state.db, actor(&admin, rid.as_ref()), id, &input).await?;
    Ok((StatusCode::CREATED, Json(Created { id: season })))
}

#[utoipa::path(
    patch,
    path = "/api/v1/admin/seasons/{id}",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Season id")),
    request_body = SeasonPatch,
    responses(
        (status = 204, description = "Saved"),
        (status = 400, description = "Invalid field"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such season, or not an admin"),
        (status = 409, description = "The number exists already")
    )
)]
pub async fn update_season(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(patch): ApiJson<SeasonPatch>,
) -> Result<StatusCode, AppError> {
    service::update_season(&state.db, actor(&admin, rid.as_ref()), id, &patch).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/seasons/{id}",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Season id")),
    responses(
        (status = 204, description = "Deleted with its episodes"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such season, or not an admin")
    )
)]
pub async fn delete_season(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    service::delete_season(&state.db, actor(&admin, rid.as_ref()), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/seasons/{id}/episodes",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Season id")),
    request_body = EpisodeInput,
    responses(
        (status = 201, description = "Created", body = Created),
        (status = 400, description = "Invalid field"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such season, or not an admin"),
        (status = 409, description = "The number exists already")
    )
)]
pub async fn create_episode(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<EpisodeInput>,
) -> Result<(StatusCode, Json<Created>), AppError> {
    let episode = service::create_episode(&state.db, actor(&admin, rid.as_ref()), id, &input).await?;
    Ok((StatusCode::CREATED, Json(Created { id: episode })))
}

#[utoipa::path(
    patch,
    path = "/api/v1/admin/episodes/{id}",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Episode id")),
    request_body = EpisodePatch,
    responses(
        (status = 204, description = "Saved"),
        (status = 400, description = "Invalid field"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such episode, or not an admin"),
        (status = 409, description = "The number exists already")
    )
)]
pub async fn update_episode(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(patch): ApiJson<EpisodePatch>,
) -> Result<StatusCode, AppError> {
    service::update_episode(&state.db, actor(&admin, rid.as_ref()), id, &patch).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/episodes/{id}",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Episode id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such episode, or not an admin")
    )
)]
pub async fn delete_episode(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, AppError> {
    service::delete_episode(&state.db, actor(&admin, rid.as_ref()), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Adds a genre for titles entered by hand. TMDB genres are created when a title is saved.
#[utoipa::path(
    post,
    path = "/api/v1/admin/genres",
    tag = "admin",
    request_body = GenreCreate,
    responses(
        (status = 201, description = "Created", body = Genre),
        (status = 400, description = "Invalid name"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "Not an admin"),
        (status = 409, description = "A genre with this slug exists")
    )
)]
pub async fn create_genre(
    State(state): State<AppState>,
    admin: Admin,
    rid: Option<Extension<RequestId>>,
    ApiJson(input): ApiJson<GenreCreate>,
) -> Result<(StatusCode, Json<Genre>), AppError> {
    let g = service::create_genre(&state.db, actor(&admin, rid.as_ref()), &input.name).await?;
    Ok((StatusCode::CREATED, Json(g)))
}
