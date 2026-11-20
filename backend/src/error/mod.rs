//! Application errors and their RFC 9457 problem rendering.
//!
//! Handlers return [`AppError`]. Converting it into a response only attaches a [`Problem`]
//! extension, and [`problem_layer`] renders the body once the request id is known. Internal
//! causes are logged there and never sent to the client.

use axum::{
    extract::Request,
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::http::request_id::RequestId;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("bad request")]
    BadRequest(&'static str),
    #[error("unauthenticated")]
    Unauthenticated,
    /// Refused for a reason the client can fix, such as a missing CSRF header. Never used to
    /// say that a resource exists but belongs to someone else, that is `NotFound`.
    #[error("forbidden")]
    Forbidden(&'static str),
    #[error("rate limited")]
    RateLimited { retry_after_secs: u64 },
    #[error("not found")]
    NotFound,
    #[error("conflict")]
    Conflict(&'static str),
    #[error("payload too large")]
    PayloadTooLarge,
    #[error("storage degraded")]
    StorageDegraded,
    #[error("unavailable")]
    Unavailable,
    #[error("internal error")]
    Internal(String),
}

impl AppError {
    pub fn internal(e: impl std::fmt::Display) -> Self {
        Self::Internal(e.to_string())
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        // sqlx errors carry SQL text and server messages but never bound parameter values.
        Self::Internal(format!("database error {e}"))
    }
}

/// Everything the problem middleware needs to render a response body.
#[derive(Debug, Clone)]
pub struct Problem {
    pub status: StatusCode,
    pub slug: &'static str,
    pub title: &'static str,
    pub internal: Option<String>,
}

impl AppError {
    fn problem(&self) -> Problem {
        let (status, slug, title) = match self {
            Self::BadRequest(slug) => (StatusCode::BAD_REQUEST, *slug, "The request was not valid."),
            Self::Unauthenticated => (StatusCode::UNAUTHORIZED, "unauthenticated", "Please sign in."),
            Self::Forbidden(slug) => (StatusCode::FORBIDDEN, *slug, "The request was refused."),
            Self::RateLimited { .. } => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate-limited",
                "Too many requests, please slow down.",
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "not-found", "The item was not found."),
            Self::Conflict(slug) => (
                StatusCode::CONFLICT,
                *slug,
                "The request conflicts with the current state.",
            ),
            Self::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload-too-large",
                "The request body is too large.",
            ),
            Self::StorageDegraded => (
                StatusCode::SERVICE_UNAVAILABLE,
                "storage_degraded",
                "Storage is degraded, uploads are paused.",
            ),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "The service is temporarily unavailable.",
            ),
            Self::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Something went wrong on our side.",
            ),
        };
        Problem {
            status,
            slug,
            title,
            internal: match self {
                Self::Internal(cause) => Some(cause.clone()),
                _ => None,
            },
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let problem = self.problem();
        let mut res = problem.status.into_response();
        if let Self::RateLimited { retry_after_secs } = self {
            res.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(retry_after_secs.max(1)));
        }
        res.extensions_mut().insert(problem);
        res
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProblemBody<'a> {
    #[serde(rename = "type")]
    kind: String,
    title: &'a str,
    status: u16,
    request_id: &'a str,
}

/// Renders problem bodies and turns bare framework error statuses into problems too, so every
/// error response has the same shape and carries the request id.
pub async fn problem_layer(req: Request, next: Next) -> Response {
    let request_id = req
        .extensions()
        .get::<RequestId>()
        .map_or_else(|| String::from("unknown"), |r| r.0.clone());
    let res = next.run(req).await;

    let problem = match res.extensions().get::<Problem>() {
        Some(p) => p.clone(),
        None if res.status().is_client_error() || res.status().is_server_error() => {
            // Bodies produced by axum or tower-http for rejected requests may echo input, so they
            // are replaced. Only the status survives.
            if res
                .headers()
                .get(header::CONTENT_TYPE)
                .is_some_and(|v| v == "application/problem+json")
            {
                return res;
            }
            fallback_problem(res.status())
        }
        None => return res,
    };

    if let Some(cause) = &problem.internal {
        tracing::error!(request_id = %request_id, cause = %cause, "internal error");
    }

    let body = ProblemBody {
        kind: format!("urn:r16a-stream:problem:{}", problem.slug),
        title: problem.title,
        status: problem.status.as_u16(),
        request_id: &request_id,
    };
    let mut out = (problem.status, axum::Json(body)).into_response();
    out.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/problem+json"),
    );
    // Keep headers that matter to clients, such as Retry-After, from the original response.
    for name in [header::RETRY_AFTER, header::ALLOW] {
        if let Some(v) = res.headers().get(&name) {
            out.headers_mut().insert(name, v.clone());
        }
    }
    out
}

fn fallback_problem(status: StatusCode) -> Problem {
    let (slug, title) = match status {
        StatusCode::NOT_FOUND => ("not-found", "The item was not found."),
        StatusCode::METHOD_NOT_ALLOWED => ("method-not-allowed", "This method is not allowed here."),
        StatusCode::PAYLOAD_TOO_LARGE => ("payload-too-large", "The request body is too large."),
        StatusCode::REQUEST_TIMEOUT => ("timeout", "The request took too long."),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ("unsupported-media-type", "The content type is not supported."),
        s if s.is_server_error() => ("internal", "Something went wrong on our side."),
        _ => ("bad-request", "The request was not valid."),
    };
    Problem {
        status,
        slug,
        title,
        internal: None,
    }
}
