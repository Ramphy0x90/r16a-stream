//! HTTP side of the web login (SPEC 6.3).

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{WebSession, cookies::sanitize_return_to, service};
use crate::{
    error::AppError,
    http::{client_ip, request_id::RequestId},
    state::AppState,
};

/// Where a failed login lands. The web app shows a neutral message there and a new sign-in
/// button. The reason stays in the logs.
const SIGNIN_FAILED: &str = "/signin?error=failed";
const SIGNIN_DENIED: &str = "/signin?error=denied";

/// A login that failed because Authentik is set up wrong for this app. Everyone sees the same
/// neutral message with a reference, which is the request id. The log line under that id names
/// the missing setting, so an admin finds it without the page telling anonymous visitors how
/// the identity provider is configured.
fn setup_failure(request_id: Option<&RequestId>, reason: &str, cookies: &[HeaderValue]) -> Response {
    let reference = request_id.map_or("unknown", |r| r.0.as_str());
    tracing::error!(reference, reason, "login impossible, Authentik setup incomplete");
    redirect(&format!("/signin?error=setup&ref={reference}"), cookies)
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct LoginQuery {
    /// Same-origin path to return to after login. Anything else is replaced by `/`.
    #[param(max_length = 512)]
    pub return_to: Option<String>,
}

/// Starts the web login and redirects to Authentik.
#[utoipa::path(
    get,
    path = "/api/v1/auth/login",
    tag = "auth",
    params(LoginQuery),
    responses(
        (status = 302, description = "Redirect to the Authentik authorize endpoint, sets the flow cookie"),
        (status = 429, description = "Too many login attempts from this address"),
        (status = 503, description = "Authentik is unreachable")
    )
)]
pub async fn login(
    State(state): State<AppState>,
    parts: Parts,
    Query(q): Query<LoginQuery>,
) -> Result<Response, AppError> {
    let auth = &state.auth;
    service::check_login_rate(auth, client_ip::client_ip(&parts, &auth.trusted_proxies))?;
    let return_to = sanitize_return_to(q.return_to.as_deref());
    match service::start_login(auth, &state.db, &return_to).await {
        Ok(start) => Ok(redirect(
            start.authorize_url.as_str(),
            &[auth.cookies.flow(&start.flow_cookie)],
        )),
        Err(service::LoginError::Setup(reason)) => Ok(setup_failure(parts.extensions.get(), &reason, &[])),
        Err(service::LoginError::App(e)) => Err(e),
        Err(e) => Err(AppError::internal(e)),
    }
}

/// Query of the callback. Unknown parameters such as `iss` are ignored.
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// The code and state are one-time credentials, so they never print.
impl std::fmt::Debug for CallbackQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CallbackQuery([redacted])")
    }
}

/// Finishes the web login, sets the session cookie and returns to the app.
#[utoipa::path(
    get,
    path = "/api/v1/auth/callback",
    tag = "auth",
    params(CallbackQuery),
    responses(
        (status = 302, description = "Signed in, redirect to the saved path. On failure, redirect to the sign-in page with an error"),
        (status = 429, description = "Too many login attempts from this address")
    )
)]
pub async fn callback(
    State(state): State<AppState>,
    parts: Parts,
    Query(q): Query<CallbackQuery>,
) -> Result<Response, AppError> {
    let auth = &state.auth;
    service::check_login_rate(auth, client_ip::client_ip(&parts, &auth.trusted_proxies))?;
    let clear_flow = auth.cookies.clear_flow();
    let (Some(code), Some(flow_state), None) = (q.code.as_deref(), q.state.as_deref(), q.error.as_deref()) else {
        tracing::info!(provider_error = q.error.is_some(), "login callback without a code");
        return Ok(redirect(SIGNIN_FAILED, &[clear_flow]));
    };
    let flow_cookie = auth.cookies.read_flow(&parts.headers);
    let user_agent = parts.headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok());
    match service::finish_login(auth, &state.db, flow_cookie.as_deref(), flow_state, code, user_agent).await {
        Ok(done) => Ok(redirect(
            &done.return_to,
            &[clear_flow, auth.cookies.session(&done.session_cookie)],
        )),
        Err(service::LoginError::Denied) => {
            tracing::info!("login denied, r16a_stream_user is false or the user is disabled");
            Ok(redirect(SIGNIN_DENIED, &[clear_flow]))
        }
        Err(service::LoginError::Setup(reason)) => Ok(setup_failure(parts.extensions.get(), &reason, &[clear_flow])),
        Err(service::LoginError::App(e)) => Err(e),
        Err(e) => {
            tracing::warn!(error = %e, "login failed");
            Ok(redirect(SIGNIN_FAILED, &[clear_flow]))
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogoutResponse {
    /// Authentik's end-session endpoint. The browser navigates there so the Authentik session
    /// ends too, and the provider's logout flow sends it back to the app. Null when Authentik
    /// is unreachable, the local session is gone either way.
    pub end_session_url: Option<String>,
}

/// Ends the web session, revokes its refresh token and clears the cookie.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "auth",
    responses(
        (status = 200, description = "Signed out", body = LogoutResponse),
        (status = 401, description = "No web session"),
        (status = 403, description = "CSRF header or Origin missing")
    )
)]
pub async fn logout(State(state): State<AppState>, WebSession(caller): WebSession) -> Result<Response, AppError> {
    let Some(session_id) = caller.session_id else {
        return Err(AppError::Unauthenticated);
    };
    let end_session_url = service::logout(&state.auth, &state.db, session_id, caller.user_id).await?;
    let mut res = Json(LogoutResponse { end_session_url }).into_response();
    res.headers_mut()
        .append(header::SET_COOKIE, state.auth.cookies.clear_session());
    no_store(res.headers_mut());
    Ok(res)
}

fn redirect(location: &str, cookies: &[HeaderValue]) -> Response {
    let mut res = StatusCode::FOUND.into_response();
    let headers = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(location) {
        headers.insert(header::LOCATION, v);
    }
    for c in cookies {
        headers.append(header::SET_COOKIE, c.clone());
    }
    no_store(headers);
    res
}

/// Login responses carry one-time values and must never be cached or replayed from a cache.
fn no_store(headers: &mut HeaderMap) {
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
}
