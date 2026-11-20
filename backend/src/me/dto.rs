//! Request and response shapes of `/me`.

use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::AppError;

/// UI languages the clients ship. Content languages are a separate, open set.
pub const UI_LANGUAGES: [&str; 4] = ["en", "it", "es", "de"];
pub const THEMES: [&str; 3] = ["dark", "light", "system"];

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeResponse {
    pub id: Uuid,
    pub display_name: String,
    /// The admin claim of this request's session or token. The admin area in the clients is
    /// a convenience, the API checks admin rights on every admin route.
    pub is_admin: bool,
    pub ui_language: String,
    /// `dark`, `light` or `system`.
    pub theme: String,
    /// BCP 47 tag of the audio language to prefer, null for none.
    pub preferred_audio_language: Option<String>,
}

/// Fields left out stay as they are. `preferredAudioLanguage: null` clears the preference.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    #[schema(max_length = 8)]
    pub ui_language: Option<String>,
    #[schema(max_length = 8)]
    pub theme: Option<String>,
    // Three states are needed, absent, null and a value, which is exactly what this type says.
    #[allow(clippy::option_option)]
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable, max_length = 35)]
    pub preferred_audio_language: Option<Option<String>>,
}

/// Distinguishes a field sent as null from a field not sent at all.
#[allow(clippy::option_option)]
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

impl SettingsPatch {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.ui_language.as_deref().is_some_and(|l| !UI_LANGUAGES.contains(&l)) {
            return Err(AppError::BadRequest("invalid-ui-language"));
        }
        if self.theme.as_deref().is_some_and(|t| !THEMES.contains(&t)) {
            return Err(AppError::BadRequest("invalid-theme"));
        }
        if let Some(Some(tag)) = &self.preferred_audio_language
            && !is_language_tag(tag)
        {
            return Err(AppError::BadRequest("invalid-language"));
        }
        Ok(())
    }
}

/// The subset of BCP 47 the database check accepts, `^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$`.
pub fn is_language_tag(tag: &str) -> bool {
    if tag.len() > 35 {
        return false;
    }
    let mut parts = tag.split('-');
    let primary_ok = parts
        .next()
        .is_some_and(|p| (2..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_lowercase()));
    primary_ok && parts.all(|p| (2..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_tags_follow_the_column_check() {
        for ok in ["en", "ita", "pt-BR", "zh-Hant-TW", "es-419"] {
            assert!(is_language_tag(ok), "{ok}");
        }
        for bad in [
            "",
            "e",
            "EN",
            "english",
            "en-",
            "en-b",
            "en_US",
            "en-US-'",
            &"a".repeat(40),
        ] {
            assert!(!is_language_tag(bad), "{bad}");
        }
    }

    #[test]
    fn null_and_missing_are_different() {
        let cleared: SettingsPatch = serde_json::from_str(r#"{"preferredAudioLanguage":null}"#).expect("json");
        assert_eq!(cleared.preferred_audio_language, Some(None));
        let untouched: SettingsPatch = serde_json::from_str("{}").expect("json");
        assert_eq!(untouched.preferred_audio_language, None);
        assert!(
            serde_json::from_str::<SettingsPatch>(r#"{"isAdmin":true}"#).is_err(),
            "unknown fields refused"
        );
    }
}
