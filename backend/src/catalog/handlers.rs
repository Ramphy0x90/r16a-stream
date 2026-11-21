//! Catalog reads for every signed-in user. Answers carry weak entity tags (SPEC 5.1).

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::Response,
};
use uuid::Uuid;

use super::{
    dto::{BrowseQuery, Episode, Genre, TitleDetail, TitlePage},
    service,
};
use crate::{
    auth::Caller,
    error::AppError,
    http::{etag::json_with_etag, path::ApiPath},
    state::AppState,
};

/// Home grid, filters and search (H1 to H5).
#[utoipa::path(
    get,
    path = "/api/v1/titles",
    tag = "catalog",
    params(BrowseQuery),
    responses(
        (status = 200, description = "One page of published titles", body = TitlePage),
        (status = 304, description = "Unchanged since the given ETag"),
        (status = 400, description = "Invalid filter, sort or cursor"),
        (status = 401, description = "Not signed in"),
        (status = 429, description = "Too many searches")
    )
)]
pub async fn browse(
    State(state): State<AppState>,
    caller: Caller,
    headers: HeaderMap,
    Query(q): Query<BrowseQuery>,
) -> Result<Response, AppError> {
    if q.q.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        state.limits.search.check(&caller.user_id)?;
    }
    let page = service::browse(&state.db, &state.image_urls, &q).await?;
    json_with_etag(&headers, &page)
}

/// Up to 20 titles added most recently, for the home row.
#[utoipa::path(
    get,
    path = "/api/v1/home/recently-added",
    tag = "catalog",
    responses(
        (status = 200, description = "Newest published titles", body = TitlePage),
        (status = 304, description = "Unchanged since the given ETag"),
        (status = 401, description = "Not signed in")
    )
)]
pub async fn recently_added(
    State(state): State<AppState>,
    _caller: Caller,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let q = BrowseQuery {
        kind: None,
        genre: None,
        language: None,
        q: None,
        sort: None,
        cursor: None,
        limit: Some(20),
    };
    let mut page = service::browse(&state.db, &state.image_urls, &q).await?;
    page.next_cursor = None;
    json_with_etag(&headers, &page)
}

/// The title page (H8). Drafts are 404 here, admins see them through the admin routes.
#[utoipa::path(
    get,
    path = "/api/v1/titles/{id}",
    tag = "catalog",
    params(("id" = Uuid, Path, description = "Title id")),
    responses(
        (status = 200, description = "The title", body = TitleDetail),
        (status = 304, description = "Unchanged since the given ETag"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "No such published title")
    )
)]
pub async fn title(
    State(state): State<AppState>,
    _caller: Caller,
    headers: HeaderMap,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Response, AppError> {
    let t = service::title_detail(&state.db, &state.image_urls, id).await?;
    json_with_etag(&headers, &t)
}

/// Episodes of one season (H9).
#[utoipa::path(
    get,
    path = "/api/v1/titles/{id}/seasons/{number}/episodes",
    tag = "catalog",
    params(
        ("id" = Uuid, Path, description = "Title id"),
        ("number" = i16, Path, description = "Season number, 0 for specials")
    ),
    responses(
        (status = 200, description = "Episodes in order", body = [Episode]),
        (status = 304, description = "Unchanged since the given ETag"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "No such published title or season")
    )
)]
pub async fn episodes(
    State(state): State<AppState>,
    _caller: Caller,
    headers: HeaderMap,
    ApiPath((id, number)): ApiPath<(Uuid, i16)>,
) -> Result<Response, AppError> {
    let list = service::season_episodes(&state.db, &state.image_urls, id, number).await?;
    json_with_etag(&headers, &list)
}

/// All genres, with the slug the clients translate.
#[utoipa::path(
    get,
    path = "/api/v1/genres",
    tag = "catalog",
    responses(
        (status = 200, description = "Genres by name", body = [Genre]),
        (status = 304, description = "Unchanged since the given ETag"),
        (status = 401, description = "Not signed in")
    )
)]
pub async fn genres(State(state): State<AppState>, _caller: Caller, headers: HeaderMap) -> Result<Response, AppError> {
    json_with_etag(&headers, &service::genres(&state.db).await?)
}

/// Content languages that at least one published title can be watched in.
#[utoipa::path(
    get,
    path = "/api/v1/languages",
    tag = "catalog",
    responses(
        (status = 200, description = "BCP 47 tags", body = [String]),
        (status = 304, description = "Unchanged since the given ETag"),
        (status = 401, description = "Not signed in")
    )
)]
pub async fn languages(
    State(state): State<AppState>,
    _caller: Caller,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let list = super::repository::languages(&state.db).await?;
    json_with_etag(&headers, &list)
}
