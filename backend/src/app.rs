//! Router assembly and the middleware stack of the API.

use std::time::Duration;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{StatusCode, header},
    middleware,
    response::IntoResponse,
    routing::{get, patch, post},
};
use tower::ServiceBuilder;
use tower_http::{
    catch_panic::CatchPanicLayer, compression::CompressionLayer, limit::RequestBodyLimitLayer,
    sensitive_headers::SetSensitiveRequestHeadersLayer, timeout::TimeoutLayer,
};

use crate::{admin, auth, error, health, http, me, state::AppState};

/// Default limit for request bodies. Routes that need more set their own.
pub const BODY_LIMIT: usize = 16 * 1024;

pub fn router(state: AppState) -> Router {
    router_with(state, Router::new())
}

/// Builds the app with extra routes mounted inside the full middleware stack. Tests use it to
/// prove that panics and errors in handlers still pass through every layer.
pub fn router_with(state: AppState, extra: Router<AppState>) -> Router {
    let api = Router::new()
        .route("/status", get(health::status))
        .route("/auth/login", get(auth::handlers::login))
        .route("/auth/callback", get(auth::handlers::callback))
        .route("/auth/logout", post(auth::handlers::logout))
        .route("/me", get(me::handlers::get_me))
        .route("/me/settings", patch(me::handlers::patch_settings))
        .route("/admin/users/{id}/revoke-sessions", post(admin::users::revoke_sessions));
    let timeout: Duration = state.config.request_timeout;
    let hsts = state.config.public_url.scheme() == "https";

    let router = Router::new()
        .nest("/api/v1", api)
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::api_readyz))
        .merge(extra)
        .fallback(|| async { StatusCode::NOT_FOUND })
        .layer(
            ServiceBuilder::new()
                // Outermost first. Sensitive headers are marked before anything can log them.
                .layer(SetSensitiveRequestHeadersLayer::new([
                    header::AUTHORIZATION,
                    header::COOKIE,
                ]))
                .layer(middleware::from_fn(http::request_id::request_id_layer))
                .layer(http::trace::layer())
                .layer(middleware::from_fn(error::problem_layer))
                // Inside the problem layer, so a panic still becomes a problem document.
                .layer(CatchPanicLayer::custom(|_| {
                    error::AppError::internal("handler panicked").into_response()
                }))
                .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, timeout))
                .layer(DefaultBodyLimit::max(BODY_LIMIT))
                .layer(RequestBodyLimitLayer::new(BODY_LIMIT)),
        );
    // Outside the problem layer, which rebuilds error responses, so every response gets them.
    http::security_headers::apply(router, hsts)
        .layer(CompressionLayer::new().br(true).gzip(true))
        .with_state(state)
}
