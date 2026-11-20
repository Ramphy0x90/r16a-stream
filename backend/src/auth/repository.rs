//! SQL for login flows, users and web sessions.

use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use super::cookies::{FLOW_SECS, SESSION_ABSOLUTE_SECS, SESSION_IDLE_SECS};

pub struct NewFlow<'a> {
    pub state_hash: &'a [u8],
    pub browser_binding: &'a [u8],
    pub pkce_verifier_enc: &'a [u8],
    pub key_id: i16,
    pub nonce_hash: &'a [u8],
    pub return_to: &'a str,
}

pub struct Flow {
    pub browser_binding: Vec<u8>,
    pub pkce_verifier_enc: Vec<u8>,
    pub key_id: i16,
    pub nonce_hash: Vec<u8>,
    pub return_to: String,
}

pub async fn insert_flow(db: &PgPool, f: NewFlow<'_>) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "insert into oidc_flows (state_hash, browser_binding, pkce_verifier_enc, key_id, nonce_hash, return_to,
                                 expires_at)
         values ($1, $2, $3, $4, $5, $6, now() + make_interval(secs => $7))",
        f.state_hash,
        f.browser_binding,
        f.pkce_verifier_enc,
        f.key_id,
        f.nonce_hash,
        f.return_to,
        f64::from(FLOW_SECS),
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Takes a flow exactly once. The row is deleted whether or not it is still valid, so a state
/// value can never be replayed.
pub async fn take_flow(db: &PgPool, state_hash: &[u8]) -> Result<Option<Flow>, sqlx::Error> {
    let row = sqlx::query!(
        "delete from oidc_flows where state_hash = $1
         returning browser_binding, pkce_verifier_enc, key_id, nonce_hash, return_to, expires_at > now() as \"live!\"",
        state_hash,
    )
    .fetch_optional(db)
    .await?;
    Ok(row.filter(|r| r.live).map(|r| Flow {
        browser_binding: r.browser_binding,
        pkce_verifier_enc: r.pkce_verifier_enc,
        key_id: r.key_id,
        nonce_hash: r.nonce_hash,
        return_to: r.return_to,
    }))
}

pub struct ProvisionedUser {
    pub id: Uuid,
    pub disabled: bool,
}

/// Creates the user on first sight and refreshes the display name and last admin claim.
pub async fn upsert_user(
    db: &mut PgConnection,
    issuer: &str,
    subject: &str,
    display_name: &str,
    is_admin: bool,
) -> Result<ProvisionedUser, sqlx::Error> {
    let r = sqlx::query!(
        "insert into users (oidc_issuer, oidc_subject, display_name, is_admin)
         values ($1, $2, $3, $4)
         on conflict (oidc_issuer, oidc_subject) do update
           set display_name = excluded.display_name, is_admin = excluded.is_admin, last_seen_at = now()
         returning id, disabled_at is not null as \"disabled!\"",
        issuer,
        subject,
        display_name,
        is_admin,
    )
    .fetch_one(db)
    .await?;
    Ok(ProvisionedUser {
        id: r.id,
        disabled: r.disabled,
    })
}

pub struct BearerUser {
    pub id: Uuid,
    pub disabled: bool,
    pub sessions_revoked_at: Option<OffsetDateTime>,
    /// Whether name or admin flag changed or the row was not touched for a minute.
    pub stale: bool,
}

pub async fn find_bearer_user(
    db: &PgPool,
    issuer: &str,
    subject: &str,
    display_name: &str,
    is_admin: bool,
) -> Result<Option<BearerUser>, sqlx::Error> {
    let r = sqlx::query!(
        "select id, disabled_at is not null as \"disabled!\", sessions_revoked_at,
                (display_name <> $3 or is_admin <> $4 or last_seen_at < now() - interval '1 minute') as \"stale!\"
         from users where oidc_issuer = $1 and oidc_subject = $2",
        issuer,
        subject,
        display_name,
        is_admin,
    )
    .fetch_optional(db)
    .await?;
    Ok(r.map(|r| BearerUser {
        id: r.id,
        disabled: r.disabled,
        sessions_revoked_at: r.sessions_revoked_at,
        stale: r.stale,
    }))
}

pub struct NewSession<'a> {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: &'a [u8],
    pub is_admin: bool,
    pub refresh_token_enc: &'a [u8],
    pub key_id: i16,
    pub user_agent: Option<&'a str>,
}

pub async fn insert_session(db: &mut PgConnection, s: NewSession<'_>) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "insert into sessions (id, user_id, token_hash, is_admin, refresh_token_enc, key_id, idle_expires_at,
                               absolute_expires_at, user_agent)
         values ($1, $2, $3, $4, $5, $6, now() + make_interval(secs => $7), now() + make_interval(secs => $8), $9)",
        s.id,
        s.user_id,
        s.token_hash,
        s.is_admin,
        s.refresh_token_enc,
        s.key_id,
        f64::from(SESSION_IDLE_SECS),
        f64::from(SESSION_ABSOLUTE_SECS),
        s.user_agent,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub struct ActiveSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub is_admin: bool,
    pub claims_age_secs: f64,
    pub needs_touch: bool,
}

/// A live session of an enabled user, by cookie hash.
pub async fn find_session(db: &PgPool, token_hash: &[u8]) -> Result<Option<ActiveSession>, sqlx::Error> {
    let r = sqlx::query!(
        "select s.id, s.user_id, s.is_admin,
                extract(epoch from now() - s.claims_refreshed_at)::float8 as \"claims_age_secs!\",
                (s.last_seen_at < now() - interval '1 minute') as \"needs_touch!\"
         from sessions s join users u on u.id = s.user_id
         where s.token_hash = $1 and s.idle_expires_at > now() and s.absolute_expires_at > now()
           and u.disabled_at is null",
        token_hash,
    )
    .fetch_optional(db)
    .await?;
    Ok(r.map(|r| ActiveSession {
        id: r.id,
        user_id: r.user_id,
        is_admin: r.is_admin,
        claims_age_secs: r.claims_age_secs,
        needs_touch: r.needs_touch,
    }))
}

/// Slides the idle expiry, never past the absolute one.
pub async fn touch_session(db: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update sessions set last_seen_at = now(),
                idle_expires_at = least(now() + make_interval(secs => $2), absolute_expires_at)
         where id = $1",
        id,
        f64::from(SESSION_IDLE_SECS),
    )
    .execute(db)
    .await?;
    Ok(())
}

/// What a request needs to refresh a session after it won the refresh lease.
pub struct ClaimedRefresh {
    pub is_admin: bool,
    pub refresh_token_enc: Vec<u8>,
    pub key_id: i16,
    pub claims_age_secs: f64,
    pub user_id: Uuid,
    pub subject: String,
}

/// Takes the refresh lease of a session whose claims are stale, in one statement. Returns
/// `None` when the claims are fresh again or another request or replica holds the lease, so
/// exactly one refresh runs per session (T-AUTH-14). No lock outlives the statement.
pub async fn claim_refresh(
    db: &PgPool,
    id: Uuid,
    max_age_secs: f64,
    lease_secs: f64,
) -> Result<Option<ClaimedRefresh>, sqlx::Error> {
    let r = sqlx::query!(
        "update sessions s set refresh_lease_until = now() + make_interval(secs => $3)
         from users u
         where s.id = $1 and u.id = s.user_id
           and s.claims_refreshed_at < now() - make_interval(secs => $2)
           and (s.refresh_lease_until is null or s.refresh_lease_until < now())
         returning s.is_admin, s.refresh_token_enc, s.key_id, s.user_id, u.oidc_subject,
                   extract(epoch from now() - s.claims_refreshed_at)::float8 as \"claims_age_secs!\"",
        id,
        max_age_secs,
        lease_secs,
    )
    .fetch_optional(db)
    .await?;
    Ok(r.map(|r| ClaimedRefresh {
        is_admin: r.is_admin,
        refresh_token_enc: r.refresh_token_enc,
        key_id: r.key_id,
        claims_age_secs: r.claims_age_secs,
        user_id: r.user_id,
        subject: r.oidc_subject,
    }))
}

/// Stores the rotated refresh token and the new admin flag and releases the lease. Returns
/// false when the session was deleted meanwhile, by logout or revocation, in which case the
/// caller revokes the new token instead of keeping it.
pub async fn store_refreshed(
    db: &mut PgConnection,
    id: Uuid,
    is_admin: bool,
    refresh_token_enc: &[u8],
    key_id: i16,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "update sessions set is_admin = $2, refresh_token_enc = $3, key_id = $4, claims_refreshed_at = now(),
                refresh_lease_until = null
         where id = $1",
        id,
        is_admin,
        refresh_token_enc,
        key_id,
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// Keeps a rotated refresh token whose claims could not be read, without marking the claims
/// fresh, and releases the lease only after its normal expiry.
pub async fn store_rotated_only(
    db: &PgPool,
    id: Uuid,
    refresh_token_enc: &[u8],
    key_id: i16,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update sessions set refresh_token_enc = $2, key_id = $3 where id = $1",
        id,
        refresh_token_enc,
        key_id,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn set_user_admin_seen(db: &mut PgConnection, user_id: Uuid, is_admin: bool) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update users set is_admin = $2, last_seen_at = now() where id = $1",
        user_id,
        is_admin
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_session(db: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!("delete from sessions where id = $1", id)
        .execute(db)
        .await?;
    Ok(())
}

pub struct EndedSession {
    pub id: Uuid,
    pub refresh_token_enc: Vec<u8>,
    pub key_id: i16,
}

/// Deletes the caller's own session and hands back the sealed refresh token for revocation.
pub async fn end_session(db: &PgPool, id: Uuid, user_id: Uuid) -> Result<Option<EndedSession>, sqlx::Error> {
    let r = sqlx::query!(
        "delete from sessions where id = $1 and user_id = $2 returning id, refresh_token_enc, key_id",
        id,
        user_id,
    )
    .fetch_optional(db)
    .await?;
    Ok(r.map(|r| EndedSession {
        id: r.id,
        refresh_token_enc: r.refresh_token_enc,
        key_id: r.key_id,
    }))
}

/// Login flows not yet expired. Bounded, because every `GET /auth/login` writes one.
pub async fn count_live_flows(db: &PgPool) -> Result<i64, sqlx::Error> {
    let n = sqlx::query_scalar!("select count(*) as \"n!\" from oidc_flows where expires_at > now()")
        .fetch_one(db)
        .await?;
    Ok(n)
}

/// Removes expired sessions and flows. Returns how many sessions went.
pub async fn purge_expired(db: &PgPool) -> Result<u64, sqlx::Error> {
    sqlx::query!("delete from oidc_flows where expires_at < now()")
        .execute(db)
        .await?;
    let r = sqlx::query!("delete from sessions where idle_expires_at < now() or absolute_expires_at < now()")
        .execute(db)
        .await?;
    Ok(r.rows_affected())
}
