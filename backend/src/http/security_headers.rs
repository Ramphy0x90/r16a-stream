//! Response headers set on every API response.
//!
//! The API serves JSON, playlists and health probes only, so the CSP forbids everything and
//! nothing may be cached unless a handler explicitly says otherwise. The API sends no CORS
//! headers, because the web app is served from the same origin.

use axum::{
    Router,
    http::{HeaderName, HeaderValue, header},
};
use tower_http::set_header::SetResponseHeaderLayer;

/// Adds the header layers to `router`. HSTS is added only when the public origin is https,
/// because browsers would otherwise pin a development host to https.
pub fn apply<S: Clone + Send + Sync + 'static>(router: Router<S>, hsts: bool) -> Router<S> {
    let router = router
        // Handlers that may be cached, such as playlists, set Cache-Control themselves, so this
        // layer only fills the gap.
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'; base-uri 'none'"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("cross-origin-resource-policy"),
            HeaderValue::from_static("same-origin"),
        ));
    if hsts {
        router.layer(SetResponseHeaderLayer::overriding(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=63072000; includeSubDomains"),
        ))
    } else {
        router
    }
}
