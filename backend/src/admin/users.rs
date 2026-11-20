//! Admin actions on users (SPEC 6.8).

use axum::{Extension, extract::State, http::StatusCode};
use uuid::Uuid;

use super::{audit, repository};
use crate::{
    auth::Admin,
    error::AppError,
    http::{path::ApiPath, request_id::RequestId},
    state::AppState,
};

/// Ends every web session of a user now and refuses their mobile tokens issued until now.
///
/// It is not a ban. A user still in `r16a-stream-users` can sign in again, removing them from
/// the group in Authentik is the ban (OPERATING 8).
#[utoipa::path(
    post,
    path = "/api/v1/admin/users/{id}/revoke-sessions",
    tag = "admin",
    params(("id" = Uuid, Path, description = "User id")),
    responses(
        (status = 204, description = "Sessions revoked"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "CSRF header or Origin missing"),
        (status = 404, description = "No such user, or the caller is not an admin")
    )
)]
pub async fn revoke_sessions(
    State(state): State<AppState>,
    Admin(admin): Admin,
    ApiPath(user_id): ApiPath<Uuid>,
    request_id: Option<Extension<RequestId>>,
) -> Result<StatusCode, AppError> {
    let mut tx = state.db.begin().await?;
    let deleted = repository::revoke_sessions(&mut tx, user_id)
        .await?
        .ok_or(AppError::NotFound)?;
    audit::record(
        &mut tx,
        audit::Entry {
            actor_id: admin.user_id,
            action: "user.revoke_sessions",
            target_kind: "user",
            target_id: Some(user_id.to_string()),
            details: serde_json::json!({ "webSessions": deleted }),
            request_id: request_id.as_ref().map(|Extension(r)| r.0.as_str()),
        },
    )
    .await?;
    tx.commit().await?;
    tracing::info!(actor = %admin.user_id, target = %user_id, "sessions revoked");
    Ok(StatusCode::NO_CONTENT)
}
