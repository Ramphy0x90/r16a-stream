//! Assigns every request a random id that ties log lines to problem responses.

use axum::{extract::Request, http::HeaderValue, middleware::Next, response::Response};

#[derive(Debug, Clone)]
pub struct RequestId(pub String);

/// Always generates a fresh id. Ids sent by clients are ignored, so log lines cannot be
/// spoofed or correlated across users by a caller.
pub async fn request_id_layer(mut req: Request, next: Next) -> Response {
    let id = uuid::Uuid::new_v4().simple().to_string();
    req.extensions_mut().insert(RequestId(id.clone()));
    let mut res = next.run(req).await;
    if let Ok(v) = HeaderValue::from_str(&id) {
        res.headers_mut().insert("x-request-id", v);
    }
    res
}
