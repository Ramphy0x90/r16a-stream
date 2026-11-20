//! SQL for the caller's own row. Every query is keyed by the caller's id.

use sqlx::PgPool;
use uuid::Uuid;

pub struct MeRow {
    pub id: Uuid,
    pub display_name: String,
    pub ui_language: String,
    pub theme: String,
    pub preferred_audio_language: Option<String>,
}

pub async fn get(db: &PgPool, caller: Uuid) -> Result<Option<MeRow>, sqlx::Error> {
    sqlx::query_as!(
        MeRow,
        "select id, display_name, ui_language, theme, preferred_audio_language from users where id = $1",
        caller
    )
    .fetch_optional(db)
    .await
}

pub async fn update_settings(
    db: &PgPool,
    caller: Uuid,
    ui_language: Option<&str>,
    theme: Option<&str>,
    audio: Option<Option<&str>>,
) -> Result<Option<MeRow>, sqlx::Error> {
    sqlx::query_as!(
        MeRow,
        "update users set
           ui_language = coalesce($2, ui_language),
           theme = coalesce($3, theme),
           preferred_audio_language = case when $4 then $5 else preferred_audio_language end
         where id = $1
         returning id, display_name, ui_language, theme, preferred_audio_language",
        caller,
        ui_language,
        theme,
        audio.is_some(),
        audio.flatten(),
    )
    .fetch_optional(db)
    .await
}
