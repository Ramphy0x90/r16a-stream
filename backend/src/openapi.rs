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
        crate::catalog::handlers::browse,
        crate::catalog::handlers::recently_added,
        crate::catalog::handlers::title,
        crate::catalog::handlers::episodes,
        crate::catalog::handlers::genres,
        crate::catalog::handlers::languages,
        crate::tmdb::handlers::lookup,
        crate::catalog::admin_handlers::list,
        crate::catalog::admin_handlers::get,
        crate::catalog::admin_handlers::create,
        crate::catalog::admin_handlers::update,
        crate::catalog::admin_handlers::delete,
        crate::catalog::admin_handlers::publish,
        crate::catalog::admin_handlers::unpublish,
        crate::catalog::admin_handlers::create_season,
        crate::catalog::admin_handlers::update_season,
        crate::catalog::admin_handlers::delete_season,
        crate::catalog::admin_handlers::create_episode,
        crate::catalog::admin_handlers::update_episode,
        crate::catalog::admin_handlers::delete_episode,
        crate::catalog::admin_handlers::create_genre,
    ),
    components(schemas(
        crate::health::StatusResponse,
        crate::auth::handlers::LogoutResponse,
        crate::me::dto::MeResponse,
        crate::me::dto::SettingsPatch,
        crate::catalog::dto::TitleKind,
        crate::catalog::dto::CreditRole,
        crate::catalog::dto::GenreInput,
        crate::catalog::dto::CreditInput,
        crate::catalog::dto::EpisodeInput,
        crate::catalog::dto::SeasonInput,
        crate::catalog::dto::TitleInput,
        crate::catalog::dto::TitlePatch,
        crate::catalog::dto::SeasonPatch,
        crate::catalog::dto::EpisodePatch,
        crate::catalog::dto::GenreCreate,
        crate::catalog::dto::Created,
        crate::catalog::dto::ImageSet,
        crate::catalog::dto::ImageSource,
        crate::catalog::dto::Genre,
        crate::catalog::dto::TitleCard,
        crate::catalog::dto::TitlePage,
        crate::catalog::dto::Credit,
        crate::catalog::dto::SeasonSummary,
        crate::catalog::dto::TitleDetail,
        crate::catalog::dto::Episode,
        crate::catalog::dto::AdminTitleRow,
        crate::catalog::dto::AdminTitlePage,
        crate::catalog::dto::AdminEpisode,
        crate::catalog::dto::AdminSeason,
        crate::catalog::dto::AdminTitle,
    )),
    tags(
        (name = "status", description = "Reachability"),
        (name = "auth", description = "Web login through the backend (BFF), SPEC 6.3"),
        (name = "me", description = "The signed-in user"),
        (name = "catalog", description = "Published titles, for every signed-in user"),
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
