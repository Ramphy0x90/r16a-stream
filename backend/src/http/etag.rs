//! Weak entity tags for catalog reads (SPEC 5.1).
//!
//! The tag is a hash of the serialised body, so it changes exactly when the answer changes.
//! Image URLs in the body are presigned per UTC day (SPEC 8.3), so tags change at most once a
//! day for unchanged data. Responses are `private, no-cache`, the browser revalidates every
//! time and gets 304 when nothing changed.

use axum::{
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::AppError;

/// Serialises `body`, answers 304 when the client already holds it.
pub fn json_with_etag<T: Serialize>(request_headers: &HeaderMap, body: &T) -> Result<Response, AppError> {
    let bytes = serde_json::to_vec(body).map_err(AppError::internal)?;
    let digest = Sha256::digest(&bytes);
    let tag = format!("W/\"{}\"", URL_SAFE_NO_PAD.encode(&digest[..16]));
    let matches = request_headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|candidate| {
            let c = candidate.trim();
            c == "*" || c == tag || c.strip_prefix("W/") == tag.strip_prefix("W/")
        });
    let mut res = if matches {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (
            [(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))],
            bytes,
        )
            .into_response()
    };
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&tag) {
        h.insert(header::ETAG, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_body_same_tag_and_304_on_match() {
        let first = json_with_etag(&HeaderMap::new(), &serde_json::json!({"a": 1})).expect("ok");
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(first.headers()[header::CACHE_CONTROL], "private, no-cache");
        let tag = first.headers()[header::ETAG].clone();
        let mut h = HeaderMap::new();
        h.insert(header::IF_NONE_MATCH, tag.clone());
        let again = json_with_etag(&h, &serde_json::json!({"a": 1})).expect("ok");
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
        let changed = json_with_etag(&h, &serde_json::json!({"a": 2})).expect("ok");
        assert_eq!(changed.status(), StatusCode::OK);
        assert_ne!(changed.headers()[header::ETAG], tag);
    }
}
