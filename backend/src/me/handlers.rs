//! HTTP side of `/me`.

use axum::{Json, extract::State};

use super::{
    dto::{MeResponse, SettingsPatch},
    repository::{self, MeRow},
};
use crate::{auth::Caller, error::AppError, http::json::ApiJson, state::AppState};

fn to_response(row: MeRow, is_admin: bool) -> MeResponse {
    MeResponse {
        id: row.id,
        display_name: row.display_name,
        is_admin,
        ui_language: row.ui_language,
        theme: row.theme,
        preferred_audio_language: row.preferred_audio_language,
    }
}

/// The signed-in user. Provisions the user on a first mobile call.
#[utoipa::path(
    get,
    path = "/api/v1/me",
    tag = "me",
    responses(
        (status = 200, description = "The caller", body = MeResponse),
        (status = 401, description = "Not signed in")
    )
)]
pub async fn get_me(State(state): State<AppState>, caller: Caller) -> Result<Json<MeResponse>, AppError> {
    let row = repository::get(&state.db, caller.user_id)
        .await?
        .ok_or(AppError::Unauthenticated)?;
    Ok(Json(to_response(row, caller.is_admin)))
}

/// Changes theme, UI language or preferred audio language.
#[utoipa::path(
    patch,
    path = "/api/v1/me/settings",
    tag = "me",
    request_body = SettingsPatch,
    responses(
        (status = 200, description = "Updated settings", body = MeResponse),
        (status = 400, description = "Invalid value"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing")
    )
)]
pub async fn patch_settings(
    State(state): State<AppState>,
    caller: Caller,
    ApiJson(patch): ApiJson<SettingsPatch>,
) -> Result<Json<MeResponse>, AppError> {
    patch.validate()?;
    let audio = patch.preferred_audio_language.as_ref().map(Option::as_deref);
    let row = repository::update_settings(
        &state.db,
        caller.user_id,
        patch.ui_language.as_deref(),
        patch.theme.as_deref(),
        audio,
    )
    .await?
    .ok_or(AppError::Unauthenticated)?;
    Ok(Json(to_response(row, caller.is_admin)))
}
