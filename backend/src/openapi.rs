//! The `OpenAPI` document, generated from the handlers and committed as `docs/openapi.json`.
//!
//! Web types and mobile models are generated from that file, and CI fails when it drifts from
//! this code. Refresh it with `cargo run --bin api -- openapi > ../docs/openapi.json`.

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(title = "r16a-stream API", version = "1", description = "Private streaming API. See docs/SPEC.md section 5."),
    paths(
        crate::health::status,
        crate::auth::handlers::login,
        crate::auth::handlers::callback,
        crate::auth::handlers::logout,
        crate::me::handlers::get_me,
        crate::me::handlers::patch_settings,
        crate::admin::users::revoke_sessions,
    ),
    components(schemas(
        crate::health::StatusResponse,
        crate::auth::handlers::LogoutResponse,
        crate::me::dto::MeResponse,
        crate::me::dto::SettingsPatch,
    )),
    tags(
        (name = "status", description = "Reachability"),
        (name = "auth", description = "Web login through the backend (BFF), SPEC 6.3"),
        (name = "me", description = "The signed-in user"),
        (name = "admin", description = "Admin only, 404 for everyone else")
    )
)]
pub struct ApiDoc;

pub fn document() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}

/// The committed form, pretty-printed with a trailing newline so diffs stay small.
pub fn to_json() -> Result<String, serde_json::Error> {
    let mut out = serde_json::to_string_pretty(&document())?;
    out.push('\n');
    Ok(out)
}
