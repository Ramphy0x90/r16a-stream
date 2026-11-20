//! Login, session and bearer rules (SPEC 6.3 to 6.5).

use std::net::IpAddr;

use subtle::ConstantTimeEq;
use url::Url;
use uuid::Uuid;

use super::{
    AuthState, Caller,
    crypto::{self, sha256},
    oidc::{Claims, OidcError, Provider},
    repository as repo,
};
use crate::error::AppError;

/// The access token lifetime. A web session re-reads its claims once this much time passed.
const CLAIMS_MAX_AGE_SECS: f64 = 300.0;
/// While Authentik is unreachable a session keeps working this long past its refresh time,
/// then fails closed.
const REFRESH_GRACE_SECS: f64 = 3600.0;
const SCOPES: &str = "openid profile offline_access r16a-stream";
/// The scope that makes an Authentik JWT an r16a-stream access token. ID tokens carry no
/// `scope` claim, so they are refused as bearer tokens.
const REQUIRED_SCOPE: &str = "r16a-stream";

/// Why a login did not finish. Shown to the user only as `failed` or `denied`.
#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("login flow invalid, {0}")]
    Flow(&'static str),
    #[error("identity provider, {0}")]
    Provider(#[from] OidcError),
    #[error("not a member of r16a-stream-users or disabled")]
    Denied,
    #[error(transparent)]
    App(#[from] AppError),
}

impl From<sqlx::Error> for LoginError {
    fn from(e: sqlx::Error) -> Self {
        Self::App(e.into())
    }
}

pub struct LoginStart {
    pub authorize_url: Url,
    pub flow_cookie: String,
}

/// Starts the authorization code flow with PKCE, a nonce and a browser-bound state.
pub async fn start_login(auth: &AuthState, db: &sqlx::PgPool, return_to: &str) -> Result<LoginStart, AppError> {
    let discovery = auth.providers.web.discovery().await.map_err(|e| provider_down(&e))?;
    let state = crypto::random_token()?;
    let nonce = crypto::random_token()?;
    let verifier = crypto::random_token()?;
    let flow_cookie = crypto::random_token()?;

    let state_hash = sha256(state.as_bytes());
    let (pkce_verifier_enc, key_id) = auth.keys.seal(verifier.as_bytes(), &crypto::flow_aad(&state_hash))?;
    repo::insert_flow(
        db,
        repo::NewFlow {
            state_hash: &state_hash,
            browser_binding: &sha256(flow_cookie.as_bytes()),
            pkce_verifier_enc: &pkce_verifier_enc,
            key_id,
            nonce_hash: &sha256(nonce.as_bytes()),
            return_to,
        },
    )
    .await?;

    let mut authorize_url = discovery.authorization_endpoint;
    authorize_url
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", auth.providers.web.client_id())
        .append_pair("redirect_uri", &auth.redirect_uri)
        .append_pair("scope", SCOPES)
        .append_pair("state", &state)
        .append_pair("nonce", &nonce)
        .append_pair("code_challenge", &crypto::pkce_challenge(&verifier))
        .append_pair("code_challenge_method", "S256");
    Ok(LoginStart {
        authorize_url,
        flow_cookie,
    })
}

pub struct LoginDone {
    pub session_cookie: String,
    pub return_to: String,
}

/// Finishes the login. Every check runs before anything is written, and the flow row is
/// consumed first, so a failed attempt cannot be retried with the same state.
pub async fn finish_login(
    auth: &AuthState,
    db: &sqlx::PgPool,
    flow_cookie: Option<&str>,
    state: &str,
    code: &str,
    user_agent: Option<&str>,
) -> Result<LoginDone, LoginError> {
    if state.len() > 128 || code.len() > 2048 {
        return Err(LoginError::Flow("parameter length"));
    }
    let state_hash = sha256(state.as_bytes());
    let flow = repo::take_flow(db, &state_hash)
        .await?
        .ok_or(LoginError::Flow("unknown or expired state"))?;
    let cookie = flow_cookie.ok_or(LoginError::Flow("no flow cookie"))?;
    if !bool::from(sha256(cookie.as_bytes()).ct_eq(flow.browser_binding.as_slice())) {
        return Err(LoginError::Flow("flow cookie does not match"));
    }
    let verifier = auth
        .keys
        .open(&flow.pkce_verifier_enc, &crypto::flow_aad(&state_hash), flow.key_id)
        .and_then(|v| String::from_utf8(v).ok())
        .ok_or(LoginError::Flow("verifier does not open"))?;

    let web = &auth.providers.web;
    let tokens = web.exchange_code(code, &verifier, &auth.redirect_uri).await?;
    let id_token = tokens
        .id_token
        .as_deref()
        .ok_or(OidcError::Protocol("no id token".into()))?;
    let refresh_token = tokens.refresh_token.as_deref().ok_or(OidcError::Protocol(
        "no refresh token, is offline_access granted".into(),
    ))?;
    let claims = verify_id_token(web, id_token).await?;
    let nonce = claims.nonce.as_deref().ok_or(LoginError::Flow("no nonce"))?;
    if !bool::from(sha256(nonce.as_bytes()).ct_eq(flow.nonce_hash.as_slice())) {
        return Err(LoginError::Flow("nonce does not match"));
    }
    if !claims.r16a_stream_user {
        revoke_quietly(web, refresh_token).await;
        return Err(LoginError::Denied);
    }

    let session_cookie = crypto::random_token()?;
    let session_id = Uuid::new_v4();
    let (refresh_token_enc, key_id) = auth
        .keys
        .seal(refresh_token.as_bytes(), &crypto::session_aad(session_id))?;
    let user_agent = user_agent.map(|ua| truncate(ua, 256));

    let mut tx = db.begin().await?;
    let user = repo::upsert_user(
        &mut tx,
        &auth.account_issuer,
        &claims.sub,
        &display_name(&claims),
        claims.r16a_stream_admin,
    )
    .await?;
    if user.disabled {
        tx.rollback().await?;
        revoke_quietly(web, refresh_token).await;
        return Err(LoginError::Denied);
    }
    repo::insert_session(
        &mut tx,
        repo::NewSession {
            id: session_id,
            user_id: user.id,
            token_hash: &sha256(session_cookie.as_bytes()),
            is_admin: claims.r16a_stream_admin,
            refresh_token_enc: &refresh_token_enc,
            key_id,
            user_agent: user_agent.as_deref(),
        },
    )
    .await?;
    tx.commit().await?;
    tracing::info!(user_id = %user.id, "web login");
    Ok(LoginDone {
        session_cookie,
        return_to: flow.return_to,
    })
}

/// ID token rules beyond the signature, issuer, audience and times checked by the provider.
async fn verify_id_token(web: &Provider, token: &str) -> Result<Claims, OidcError> {
    let claims = web.verify(token).await?;
    // A token minted for another client of the same provider must not open a session here.
    if claims.azp.as_deref().is_some_and(|azp| azp != web.client_id()) {
        return Err(OidcError::Rejected("authorized party".into()));
    }
    Ok(claims)
}

/// Resolves a session cookie to a caller, refreshing the claims when they are older than the
/// access token lifetime.
pub async fn session_caller(auth: &AuthState, db: &sqlx::PgPool, cookie: &str) -> Result<Caller, AppError> {
    let hash = sha256(cookie.as_bytes());
    let session = repo::find_session(db, &hash).await?.ok_or(AppError::Unauthenticated)?;
    let is_admin = if session.claims_age_secs >= CLAIMS_MAX_AGE_SECS {
        refresh_claims(auth, db, session.id).await?
    } else {
        session.is_admin
    };
    if session.needs_touch {
        repo::touch_session(db, session.id).await?;
    }
    Ok(Caller {
        user_id: session.user_id,
        is_admin,
        session_id: Some(session.id),
    })
}

/// Single-flight claim refresh under the session row lock (T-AUTH-14). Returns the admin flag
/// to use for this request.
async fn refresh_claims(auth: &AuthState, db: &sqlx::PgPool, session_id: Uuid) -> Result<bool, AppError> {
    let mut tx = db.begin().await?;
    let locked = repo::lock_session(&mut tx, session_id)
        .await?
        .ok_or(AppError::Unauthenticated)?;
    if locked.claims_age_secs < CLAIMS_MAX_AGE_SECS {
        // Another request or replica refreshed while this one waited for the lock.
        tx.commit().await?;
        return Ok(locked.is_admin);
    }
    let Some(refresh_token) = auth
        .keys
        .open(
            &locked.refresh_token_enc,
            &crypto::session_aad(session_id),
            locked.key_id,
        )
        .and_then(|v| String::from_utf8(v).ok())
    else {
        tracing::warn!(%session_id, "sealed refresh token does not open, ending the session");
        repo::delete_session(&mut tx, session_id).await?;
        tx.commit().await?;
        return Err(AppError::Unauthenticated);
    };

    let web = &auth.providers.web;
    let outcome = match web.refresh(&refresh_token).await {
        Ok(tokens) => match (tokens.id_token, tokens.refresh_token) {
            (Some(id), Some(new_refresh)) => match verify_id_token(web, &id).await {
                Ok(claims) if claims.sub != locked.subject => Err(OidcError::Rejected("subject changed".into())),
                Ok(claims) => Ok((claims, new_refresh)),
                Err(e) => Err(e),
            },
            _ => Err(OidcError::Protocol("refresh response lacks a token".into())),
        },
        Err(e) => Err(e),
    };

    match outcome {
        Ok((claims, new_refresh)) if claims.r16a_stream_user => {
            let (enc, key_id) = auth
                .keys
                .seal(new_refresh.as_bytes(), &crypto::session_aad(session_id))?;
            repo::store_refreshed(&mut tx, session_id, claims.r16a_stream_admin, &enc, key_id).await?;
            repo::set_user_admin_seen(&mut tx, locked.user_id, claims.r16a_stream_admin).await?;
            tx.commit().await?;
            Ok(claims.r16a_stream_admin)
        }
        Ok((_, new_refresh)) => {
            // Removed from the users group or deactivated in Authentik (C3).
            tracing::info!(%session_id, "access claim gone on refresh, ending the session");
            repo::delete_session(&mut tx, session_id).await?;
            tx.commit().await?;
            revoke_quietly(web, &new_refresh).await;
            Err(AppError::Unauthenticated)
        }
        Err(OidcError::Unavailable(reason)) => {
            tx.commit().await?;
            if locked.claims_age_secs > CLAIMS_MAX_AGE_SECS + REFRESH_GRACE_SECS {
                tracing::warn!(%session_id, %reason, "claim refresh impossible past the grace period");
                Err(AppError::Unavailable)
            } else {
                tracing::warn!(%session_id, %reason, "claim refresh postponed, Authentik unreachable");
                Ok(locked.is_admin)
            }
        }
        Err(e) => {
            tracing::info!(%session_id, error = %e, "claim refresh refused, ending the session");
            repo::delete_session(&mut tx, session_id).await?;
            tx.commit().await?;
            Err(AppError::Unauthenticated)
        }
    }
}

/// Resolves a mobile access token to a caller, provisioning the user on first sight.
pub async fn bearer_caller(auth: &AuthState, db: &sqlx::PgPool, token: &str) -> Result<Caller, AppError> {
    let mobile = &auth.providers.mobile;
    let claims = match mobile.verify(token).await {
        Ok(c) => c,
        Err(OidcError::Unavailable(reason)) => {
            tracing::warn!(%reason, "cannot verify bearer tokens, Authentik unreachable");
            return Err(AppError::Unavailable);
        }
        Err(e) => {
            tracing::debug!(error = %e, "bearer token refused");
            return Err(AppError::Unauthenticated);
        }
    };
    let scoped = claims
        .scope
        .as_deref()
        .is_some_and(|s| s.split_ascii_whitespace().any(|x| x == REQUIRED_SCOPE));
    let own_client = claims.azp.as_deref().is_none_or(|azp| azp == mobile.client_id());
    if !scoped || !own_client || !claims.r16a_stream_user {
        tracing::debug!(
            scoped,
            own_client,
            member = claims.r16a_stream_user,
            "bearer token refused"
        );
        return Err(AppError::Unauthenticated);
    }

    let name = display_name(&claims);
    let found = repo::find_bearer_user(db, &auth.account_issuer, &claims.sub, &name, claims.r16a_stream_admin).await?;
    let (id, disabled, revoked_at) = match found {
        Some(u) if !u.stale => (u.id, u.disabled, u.sessions_revoked_at),
        other => {
            let mut conn = db.acquire().await?;
            let u = repo::upsert_user(
                &mut conn,
                &auth.account_issuer,
                &claims.sub,
                &name,
                claims.r16a_stream_admin,
            )
            .await?;
            (u.id, u.disabled, other.and_then(|o| o.sessions_revoked_at))
        }
    };
    if disabled {
        return Err(AppError::Unauthenticated);
    }
    // Tokens issued in the same second as a revocation are refused too, the comparison fails
    // closed (SPEC 6.8).
    if revoked_at.is_some_and(|r| claims.iat <= r.unix_timestamp()) {
        return Err(AppError::Unauthenticated);
    }
    Ok(Caller {
        user_id: id,
        is_admin: claims.r16a_stream_admin,
        session_id: None,
    })
}

/// Ends the caller's web session and revokes its refresh token at Authentik. Returns the
/// end-session URL the browser visits next, if the provider has one.
pub async fn logout(
    auth: &AuthState,
    db: &sqlx::PgPool,
    session_id: Uuid,
    user_id: Uuid,
) -> Result<Option<String>, AppError> {
    if let Some(ended) = repo::end_session(db, session_id, user_id).await?
        && let Some(token) = auth
            .keys
            .open(&ended.refresh_token_enc, &crypto::session_aad(ended.id), ended.key_id)
            .and_then(|v| String::from_utf8(v).ok())
    {
        revoke_quietly(&auth.providers.web, &token).await;
    }
    // No parameters, Authentik 2026.2.1 ignores them and the provider's own logout flow
    // redirects back to the app (SPEC 6.1.1).
    Ok(auth
        .providers
        .web
        .discovery()
        .await
        .ok()
        .and_then(|d| d.end_session_endpoint)
        .map(String::from))
}

async fn revoke_quietly(provider: &Provider, refresh_token: &str) {
    if let Err(e) = provider.revoke(refresh_token).await {
        tracing::warn!(error = %e, "refresh token revocation failed");
    }
}

fn provider_down(e: &OidcError) -> AppError {
    tracing::warn!(error = %e, "identity provider discovery failed");
    AppError::Unavailable
}

/// Rate limit key for the login routes.
pub fn check_login_rate(auth: &AuthState, ip: IpAddr) -> Result<(), AppError> {
    auth.login_limit.check(&ip)
}

/// `name`, else `preferred_username`, cleaned of control characters and cut to the column limit.
pub fn display_name(c: &Claims) -> String {
    let pick = [c.name.as_deref(), c.preferred_username.as_deref()]
        .into_iter()
        .flatten()
        .map(|s| s.chars().filter(|ch| !ch.is_control()).collect::<String>())
        .map(|s| s.trim().to_owned())
        .find(|s| !s.is_empty());
    truncate(pick.as_deref().unwrap_or("user"), 100)
}

fn truncate(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(name: Option<&str>, username: Option<&str>) -> Claims {
        Claims {
            sub: "s".into(),
            iat: 0,
            nonce: None,
            azp: None,
            scope: None,
            name: name.map(Into::into),
            preferred_username: username.map(Into::into),
            r16a_stream_user: true,
            r16a_stream_admin: false,
        }
    }

    #[test]
    fn display_name_prefers_name_and_cleans_it() {
        assert_eq!(display_name(&claims(Some(" Ada\u{0007} "), Some("ada"))), "Ada");
        assert_eq!(display_name(&claims(Some("  "), Some("ada"))), "ada");
        assert_eq!(display_name(&claims(None, None)), "user");
        assert_eq!(display_name(&claims(Some(&"é".repeat(150)), None)).chars().count(), 100);
    }
}
