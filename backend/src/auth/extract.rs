//! Extractors that authenticate a request. Handlers take [`Caller`], [`WebSession`] or
//! [`Admin`], and never look at cookies or headers themselves.

use axum::{
    extract::FromRequestParts,
    http::{Method, header, request::Parts},
};

use super::{Caller, service};
use crate::{error::AppError, state::AppState};

/// Largest bearer token accepted. Authentik's JWTs are around 1 KiB.
const MAX_BEARER_LEN: usize = 8192;

impl FromRequestParts<AppState> for Caller {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        if let Some(c) = parts.extensions.get::<Self>() {
            return Ok(*c);
        }
        let auth = &state.auth;
        let caller = if let Some(value) = parts.headers.get(header::AUTHORIZATION) {
            // A request that names a bearer token is judged by it alone, even if a cookie is
            // also present. Browsers never attach this header on their own, so no CSRF check.
            let token = value
                .to_str()
                .ok()
                .and_then(|v| v.strip_prefix("Bearer "))
                .filter(|t| !t.is_empty() && t.len() <= MAX_BEARER_LEN)
                .ok_or(AppError::Unauthenticated)?;
            service::bearer_caller(auth, &state.db, token).await?
        } else if let Some(cookie) = auth.cookies.read_session(&parts.headers) {
            check_csrf(parts, &auth.public_origin)?;
            service::session_caller(auth, &state.db, &cookie).await?
        } else {
            return Err(AppError::Unauthenticated);
        };
        auth.user_limit.check(&caller.user_id)?;
        parts.extensions.insert(caller);
        Ok(caller)
    }
}

/// State-changing cookie requests need `X-R16a-Request: 1` and an `Origin` equal to the public
/// origin (T-AUTH-5). A cross-site form cannot set the header, and a cross-site script cannot
/// send it without a CORS preflight that the API never answers.
fn check_csrf(parts: &Parts, public_origin: &str) -> Result<(), AppError> {
    if matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(());
    }
    let header_ok = parts.headers.get("x-r16a-request").is_some_and(|v| v == "1");
    let origin_ok = parts
        .headers
        .get(header::ORIGIN)
        .is_some_and(|v| v.as_bytes() == public_origin.as_bytes());
    if header_ok && origin_ok {
        Ok(())
    } else {
        Err(AppError::Forbidden("csrf"))
    }
}

/// A caller signed in through the web BFF. Bearer tokens are refused, logout is a web action.
pub struct WebSession(pub Caller);

impl FromRequestParts<AppState> for WebSession {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        if parts.headers.contains_key(header::AUTHORIZATION) {
            return Err(AppError::Unauthenticated);
        }
        let caller = Caller::from_request_parts(parts, state).await?;
        if caller.session_id.is_none() {
            return Err(AppError::Unauthenticated);
        }
        Ok(Self(caller))
    }
}

/// A caller whose current claims say admin. Anyone else gets 404, so admin routes do not
/// reveal themselves to members (SPEC 6.6). Anonymous callers get 401 like on every route.
pub struct Admin(pub Caller);

impl FromRequestParts<AppState> for Admin {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let caller = Caller::from_request_parts(parts, state).await?;
        if caller.is_admin {
            Ok(Self(caller))
        } else {
            Err(AppError::NotFound)
        }
    }
}
