//! Router assembly and the middleware stack of the API.

use std::time::Duration;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{StatusCode, header},
    middleware,
    response::IntoResponse,
    routing::{delete, get, patch, post},
};
use tower::ServiceBuilder;
use tower_http::{
    catch_panic::CatchPanicLayer, compression::CompressionLayer, limit::RequestBodyLimitLayer,
    sensitive_headers::SetSensitiveRequestHeadersLayer, timeout::TimeoutLayer,
};

use crate::{admin, auth, catalog, error, health, http, me, media, state::AppState, tmdb};

/// Default limit for request bodies. Routes that need more set their own with
/// `DefaultBodyLimit`, up to [`MAX_BODY_LIMIT`].
pub const BODY_LIMIT: usize = 16 * 1024;

/// Hard ceiling for any request body, enforced before a handler runs. The largest route limit.
pub const MAX_BODY_LIMIT: usize = 2 * 1024 * 1024;

pub fn router(state: AppState) -> Router {
    router_with(state, Router::new())
}

/// Variant, upload, subtitle, job and storage administration (SPEC 5.2, Phase 4).
fn media_routes() -> Router<AppState> {
    Router::new()
        .route("/admin/variants", post(media::handlers::create_variant))
        .route("/admin/variants/{id}", delete(media::handlers::delete_variant))
        .route("/admin/variants/{id}/uploads", post(media::handlers::start_upload))
        .route("/admin/variants/{id}/retranscode", post(media::handlers::retranscode))
        .route("/admin/titles/{id}/storage", get(media::handlers::title_storage))
        .route(
            "/admin/uploads/{id}",
            get(media::handlers::get_upload).delete(media::handlers::abort_upload),
        )
        .route("/admin/uploads/{id}/parts", post(media::handlers::part_urls))
        .route(
            "/admin/uploads/{id}/complete",
            post(media::handlers::complete_upload).layer(DefaultBodyLimit::max(media::handlers::COMPLETE_BODY_LIMIT)),
        )
        .route("/admin/jobs", get(media::handlers::list_jobs))
        .route("/admin/jobs/{id}", get(media::handlers::get_job))
        .route("/admin/jobs/{id}/retry", post(media::handlers::retry_job))
        .route("/admin/jobs/{id}/cancel", post(media::handlers::cancel_job))
        .route("/admin/images", post(media::handlers::start_image_upload))
        .route(
            "/admin/variants/{id}/subtitles",
            post(media::handlers::create_subtitle).layer(DefaultBodyLimit::max(media::handlers::SUBTITLE_BODY_LIMIT)),
        )
        .route("/admin/subtitles/{id}", delete(media::handlers::delete_subtitle))
        .route("/admin/storage", get(media::handlers::storage))
        .route("/admin/storage/titles", get(media::handlers::storage_titles))
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
        .route("/admin/users/{id}/revoke-sessions", post(admin::users::revoke_sessions))
        .route("/genres", get(catalog::handlers::genres))
        .route("/languages", get(catalog::handlers::languages))
        .route("/titles", get(catalog::handlers::browse))
        .route("/home/recently-added", get(catalog::handlers::recently_added))
        .route("/titles/{id}", get(catalog::handlers::title))
        .route(
            "/titles/{id}/seasons/{number}/episodes",
            get(catalog::handlers::episodes),
        )
        .route("/admin/tmdb/lookup", get(tmdb::handlers::lookup))
        .route("/admin/genres", post(catalog::admin_handlers::create_genre))
        .route(
            "/admin/titles",
            get(catalog::admin_handlers::list)
                .post(catalog::admin_handlers::create)
                .layer(
                    // A whole show with every episode, as the TMDB draft carries it.
                    DefaultBodyLimit::max(catalog::admin_handlers::TITLE_BODY_LIMIT),
                ),
        )
        .route(
            "/admin/titles/{id}",
            get(catalog::admin_handlers::get)
                .patch(catalog::admin_handlers::update)
                .delete(catalog::admin_handlers::delete),
        )
        .route("/admin/titles/{id}/publish", post(catalog::admin_handlers::publish))
        .route("/admin/titles/{id}/unpublish", post(catalog::admin_handlers::unpublish))
        .route(
            "/admin/titles/{id}/seasons",
            post(catalog::admin_handlers::create_season)
                .layer(DefaultBodyLimit::max(catalog::admin_handlers::TITLE_BODY_LIMIT)),
        )
        .route(
            "/admin/seasons/{id}",
            patch(catalog::admin_handlers::update_season).delete(catalog::admin_handlers::delete_season),
        )
        .route(
            "/admin/seasons/{id}/episodes",
            post(catalog::admin_handlers::create_episode),
        )
        .route(
            "/admin/episodes/{id}",
            patch(catalog::admin_handlers::update_episode).delete(catalog::admin_handlers::delete_episode),
        )
        .merge(media_routes());
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
                // The default for extractors, routes raise it with their own DefaultBodyLimit. The
                // hard ceiling refuses anything larger than the largest route limit up front.
                .layer(DefaultBodyLimit::max(BODY_LIMIT))
                .layer(RequestBodyLimitLayer::new(MAX_BODY_LIMIT)),
        );
    // Outside the problem layer, which rebuilds error responses, so every response gets them.
    http::security_headers::apply(router, hsts)
        .layer(CompressionLayer::new().br(true).gzip(true))
        .with_state(state)
}
