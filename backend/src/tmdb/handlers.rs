//! `GET /admin/tmdb/lookup`.

use axum::{
    Json,
    extract::{Query, State},
};
use serde::Deserialize;
use utoipa::IntoParams;

use super::{draft, reference};
use crate::{auth::Admin, catalog::dto::TitleInput, error::AppError, state::AppState};

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LookupQuery {
    /// A TMDB link (`https://www.themoviedb.org/movie/603`, `/tv/1396-breaking-bad`) or a
    /// numeric id together with `kind`.
    #[serde(rename = "ref")]
    #[param(max_length = 300)]
    pub reference: String,
    /// `movie` or `show`, needed with a numeric id.
    pub kind: Option<String>,
}

/// Prefills a title draft from TMDB. Nothing is saved, the admin edits the draft and posts it
/// to `POST /admin/titles`.
#[utoipa::path(
    get,
    path = "/api/v1/admin/tmdb/lookup",
    tag = "admin",
    params(LookupQuery),
    responses(
        (status = 200, description = "Editable draft", body = TitleInput),
        (status = 400, description = "Not a TMDB link or id"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "Unknown on TMDB, or not an admin"),
        (status = 429, description = "Too many lookups"),
        (status = 503, description = "TMDB is unreachable")
    )
)]
pub async fn lookup(
    State(state): State<AppState>,
    admin: Admin,
    Query(q): Query<LookupQuery>,
) -> Result<Json<TitleInput>, AppError> {
    state.limits.tmdb.check(&admin.0.user_id)?;
    let r = reference::parse(&q.reference, q.kind.as_deref())?;
    Ok(Json(draft::lookup(&state.db, &state.tmdb, r).await?))
}
