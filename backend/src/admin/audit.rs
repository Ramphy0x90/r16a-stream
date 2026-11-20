//! Writes to the append-only audit log.

use sqlx::PgConnection;
use uuid::Uuid;

pub struct Entry<'a> {
    pub actor_id: Uuid,
    /// Dotted verb such as `user.revoke_sessions`.
    pub action: &'a str,
    pub target_kind: &'a str,
    pub target_id: Option<String>,
    /// Field names and ids only, never secrets or free text.
    pub details: serde_json::Value,
    pub request_id: Option<&'a str>,
}

pub async fn record(db: &mut PgConnection, e: Entry<'_>) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "insert into audit_log (actor_id, action, target_kind, target_id, details, request_id)
         values ($1, $2, $3, $4, $5, $6)",
        e.actor_id,
        e.action,
        e.target_kind,
        e.target_id,
        e.details,
        e.request_id,
    )
    .execute(db)
    .await?;
    Ok(())
}
