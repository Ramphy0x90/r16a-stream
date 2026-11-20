//! SQL for admin actions on users.

use sqlx::PgConnection;
use uuid::Uuid;

/// Marks every bearer token issued until now as revoked and deletes all web sessions of the
/// user. Returns the number of web sessions removed, or `None` when there is no such user.
pub async fn revoke_sessions(db: &mut PgConnection, user_id: Uuid) -> Result<Option<u64>, sqlx::Error> {
    let found = sqlx::query_scalar!(
        "update users set sessions_revoked_at = now() where id = $1 returning id",
        user_id
    )
    .fetch_optional(&mut *db)
    .await?;
    if found.is_none() {
        return Ok(None);
    }
    let deleted = sqlx::query!("delete from sessions where user_id = $1", user_id)
        .execute(db)
        .await?
        .rows_affected();
    Ok(Some(deleted))
}
