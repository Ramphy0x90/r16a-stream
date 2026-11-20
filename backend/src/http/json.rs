//! JSON body extractor that maps every rejection to a generic problem, so framework error
//! strings, which may echo input, never reach the client.

use axum::{
    extract::{FromRequest, Request, rejection::JsonRejection},
    http::StatusCode,
};
use serde::de::DeserializeOwned;

use crate::error::AppError;

/// Use instead of `axum::Json` for every request body. DTOs pair it with
/// `#[serde(deny_unknown_fields)]`.
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, AppError> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(v)) => Ok(Self(v)),
            Err(JsonRejection::BytesRejection(e)) if e.status() == StatusCode::PAYLOAD_TOO_LARGE => {
                Err(AppError::PayloadTooLarge)
            }
            Err(JsonRejection::MissingJsonContentType(_)) => Err(AppError::BadRequest("unsupported-media-type")),
            Err(_) => Err(AppError::BadRequest("invalid-body")),
        }
    }
}
