//! Request and response shapes of the catalog, public and admin.
//!
//! The TMDB lookup returns a [`TitleInput`], the same shape `POST /admin/titles` takes, so the
//! admin client edits the draft and posts it back without a second mapping.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum TitleKind {
    Movie,
    Show,
}

impl TitleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Show => "show",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "movie" => Some(Self::Movie),
            "show" => Some(Self::Show),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CreditRole {
    Cast,
    Director,
    Creator,
    Writer,
}

impl CreditRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cast => "cast",
            Self::Director => "director",
            Self::Creator => "creator",
            Self::Writer => "writer",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cast" => Some(Self::Cast),
            "director" => Some(Self::Director),
            "creator" => Some(Self::Creator),
            "writer" => Some(Self::Writer),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- admin input

/// A genre that may not exist yet, from TMDB or typed by an admin. Matched by `tmdbId` when set,
/// otherwise by the slug derived from the name.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenreInput {
    pub tmdb_id: Option<i32>,
    #[schema(min_length = 1, max_length = 60)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreditInput {
    #[schema(min_length = 1, max_length = 200)]
    pub person_name: String,
    #[schema(max_length = 200)]
    pub character: Option<String>,
    pub role: CreditRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EpisodeInput {
    #[schema(minimum = 1, maximum = 9999)]
    pub number: i16,
    #[schema(max_length = 300)]
    pub name: Option<String>,
    #[schema(max_length = 5000)]
    pub overview: Option<String>,
    #[schema(minimum = 1, maximum = 1440)]
    pub runtime_minutes: Option<i32>,
    /// ISO date, `YYYY-MM-DD`.
    #[schema(format = Date, max_length = 10)]
    pub air_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SeasonInput {
    #[schema(minimum = 0, maximum = 999)]
    pub number: i16,
    #[schema(max_length = 300)]
    pub name: Option<String>,
    #[schema(max_length = 5000)]
    pub overview: Option<String>,
    #[schema(minimum = 1870, maximum = 2200)]
    pub year: Option<i16>,
    #[serde(default)]
    pub episodes: Vec<EpisodeInput>,
}

/// A whole title as an admin saves it, or as the TMDB lookup proposes it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TitleInput {
    pub kind: TitleKind,
    #[schema(min_length = 1, max_length = 300)]
    pub name: String,
    #[schema(max_length = 300)]
    pub original_name: Option<String>,
    #[schema(max_length = 5000)]
    pub overview: Option<String>,
    #[schema(minimum = 1870, maximum = 2200)]
    pub year: Option<i16>,
    /// Movies only.
    #[schema(minimum = 1, maximum = 1440)]
    pub runtime_minutes: Option<i32>,
    pub tmdb_id: Option<i32>,
    /// Existing genres by id.
    #[serde(default)]
    pub genre_ids: Vec<i16>,
    /// Genres to create when missing.
    #[serde(default)]
    pub new_genres: Vec<GenreInput>,
    #[serde(default)]
    pub credits: Vec<CreditInput>,
    /// Shows only. Create only, later changes go through the season and episode routes.
    #[serde(default)]
    pub seasons: Vec<SeasonInput>,
    /// TMDB file path of the poster, for example `/kU3B75TyRiCgE270EyZnHjfivoq.jpg`. The API
    /// downloads it on save (SPEC 8.2).
    #[schema(max_length = 70)]
    pub poster_path: Option<String>,
    #[schema(max_length = 70)]
    pub backdrop_path: Option<String>,
}

/// Fields of a title that `PATCH /admin/titles/{id}` may change. Left out means unchanged.
/// `genreIds`, `newGenres` and `credits` replace the whole set when present.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TitlePatch {
    #[schema(min_length = 1, max_length = 300)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable, max_length = 300)]
    pub original_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable, max_length = 5000)]
    pub overview: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i16>, nullable)]
    pub year: Option<Option<i16>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i32>, nullable)]
    pub runtime_minutes: Option<Option<i32>>,
    pub genre_ids: Option<Vec<i16>>,
    pub new_genres: Option<Vec<GenreInput>>,
    pub credits: Option<Vec<CreditInput>>,
    /// A TMDB file path replaces the image, null removes it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable, max_length = 70)]
    pub poster_path: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable, max_length = 70)]
    pub backdrop_path: Option<Option<String>>,
}

/// Fields of a season or episode patch. Left out means unchanged, null clears.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SeasonPatch {
    #[schema(minimum = 0, maximum = 999)]
    pub number: Option<i16>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable, max_length = 300)]
    pub name: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable, max_length = 5000)]
    pub overview: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i16>, nullable)]
    pub year: Option<Option<i16>>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EpisodePatch {
    #[schema(minimum = 1, maximum = 9999)]
    pub number: Option<i16>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable, max_length = 300)]
    pub name: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable, max_length = 5000)]
    pub overview: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i32>, nullable)]
    pub runtime_minutes: Option<Option<i32>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable, format = Date)]
    pub air_date: Option<Option<String>>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenreCreate {
    #[schema(min_length = 1, max_length = 60)]
    pub name: String,
}

/// Distinguishes a field sent as null from a field not sent at all.
#[allow(clippy::option_option)]
fn present<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    pub id: Uuid,
}

// ---------------------------------------------------------------- validation

/// Upper bounds on what one request may carry, far above any real show.
pub const MAX_SEASONS: usize = 100;
pub const MAX_EPISODES_PER_SEASON: usize = 500;
pub const MAX_CREDITS: usize = 50;
pub const MAX_GENRES: usize = 20;

fn text(v: &str, min: usize, max: usize, slug: &'static str) -> Result<(), AppError> {
    let n = v.chars().count();
    if n < min || n > max || v.chars().any(|c| c.is_control() && c != '\n') {
        return Err(AppError::BadRequest(slug));
    }
    Ok(())
}

fn opt_text(v: Option<&str>, max: usize, slug: &'static str) -> Result<(), AppError> {
    v.map_or(Ok(()), |s| text(s, 0, max, slug))
}

fn range<T: PartialOrd + Copy>(v: Option<T>, lo: T, hi: T, slug: &'static str) -> Result<(), AppError> {
    match v {
        Some(x) if x < lo || x > hi => Err(AppError::BadRequest(slug)),
        _ => Ok(()),
    }
}

pub fn validate_genres(ids: &[i16], new: &[GenreInput]) -> Result<(), AppError> {
    if ids.len() + new.len() > MAX_GENRES {
        return Err(AppError::BadRequest("too-many-genres"));
    }
    for g in new {
        text(&g.name, 1, 60, "invalid-genre")?;
        if crate::catalog::service::slugify(&g.name).is_none() {
            return Err(AppError::BadRequest("invalid-genre"));
        }
    }
    Ok(())
}

pub fn validate_credits(credits: &[CreditInput]) -> Result<(), AppError> {
    if credits.len() > MAX_CREDITS {
        return Err(AppError::BadRequest("too-many-credits"));
    }
    for c in credits {
        text(&c.person_name, 1, 200, "invalid-credit")?;
        opt_text(c.character.as_deref(), 200, "invalid-credit")?;
    }
    Ok(())
}

/// Parses an ISO calendar date, `YYYY-MM-DD`.
pub fn parse_date(s: &str) -> Result<time::Date, AppError> {
    time::Date::parse(s, &time::format_description::well_known::Iso8601::DATE)
        .ok()
        .filter(|d| (1870..=2200).contains(&d.year()))
        .ok_or(AppError::BadRequest("invalid-date"))
}

pub fn format_date(d: time::Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

pub fn validate_episode(e: &EpisodeInput) -> Result<(), AppError> {
    range(Some(e.number), 1, 9999, "invalid-episode-number")?;
    if let Some(d) = &e.air_date {
        parse_date(d)?;
    }
    opt_text(e.name.as_deref(), 300, "invalid-name")?;
    opt_text(e.overview.as_deref(), 5000, "invalid-overview")?;
    range(e.runtime_minutes, 1, 1440, "invalid-runtime")
}

pub fn validate_season(s: &SeasonInput) -> Result<(), AppError> {
    range(Some(s.number), 0, 999, "invalid-season-number")?;
    opt_text(s.name.as_deref(), 300, "invalid-name")?;
    opt_text(s.overview.as_deref(), 5000, "invalid-overview")?;
    range(s.year, 1870, 2200, "invalid-year")?;
    if s.episodes.len() > MAX_EPISODES_PER_SEASON {
        return Err(AppError::BadRequest("too-many-episodes"));
    }
    let mut seen = std::collections::HashSet::new();
    for e in &s.episodes {
        validate_episode(e)?;
        if !seen.insert(e.number) {
            return Err(AppError::Conflict("duplicate-episode-number"));
        }
    }
    Ok(())
}

impl TitleInput {
    pub fn validate(&self) -> Result<(), AppError> {
        text(&self.name, 1, 300, "invalid-name")?;
        opt_text(self.original_name.as_deref(), 300, "invalid-name")?;
        opt_text(self.overview.as_deref(), 5000, "invalid-overview")?;
        range(self.year, 1870, 2200, "invalid-year")?;
        range(self.runtime_minutes, 1, 1440, "invalid-runtime")?;
        range(self.tmdb_id, 1, i32::MAX, "invalid-tmdb-id")?;
        if self.kind == TitleKind::Show && self.runtime_minutes.is_some() {
            return Err(AppError::BadRequest("runtime-is-per-episode"));
        }
        if self.kind == TitleKind::Movie && !self.seasons.is_empty() {
            return Err(AppError::BadRequest("movies-have-no-seasons"));
        }
        validate_genres(&self.genre_ids, &self.new_genres)?;
        validate_credits(&self.credits)?;
        validate_image_path(self.poster_path.as_deref())?;
        validate_image_path(self.backdrop_path.as_deref())?;
        if self.seasons.len() > MAX_SEASONS {
            return Err(AppError::BadRequest("too-many-seasons"));
        }
        let mut seen = std::collections::HashSet::new();
        for s in &self.seasons {
            validate_season(s)?;
            if !seen.insert(s.number) {
                return Err(AppError::Conflict("duplicate-season-number"));
            }
        }
        Ok(())
    }
}

impl TitlePatch {
    pub fn validate(&self) -> Result<(), AppError> {
        if let Some(n) = &self.name {
            text(n, 1, 300, "invalid-name")?;
        }
        opt_text(self.original_name.clone().flatten().as_deref(), 300, "invalid-name")?;
        opt_text(self.overview.clone().flatten().as_deref(), 5000, "invalid-overview")?;
        range(self.year.flatten(), 1870, 2200, "invalid-year")?;
        range(self.runtime_minutes.flatten(), 1, 1440, "invalid-runtime")?;
        validate_genres(
            self.genre_ids.as_deref().unwrap_or_default(),
            self.new_genres.as_deref().unwrap_or_default(),
        )?;
        validate_credits(self.credits.as_deref().unwrap_or_default())?;
        validate_image_path(self.poster_path.clone().flatten().as_deref())?;
        validate_image_path(self.backdrop_path.clone().flatten().as_deref())
    }
}

fn validate_image_path(p: Option<&str>) -> Result<(), AppError> {
    match p {
        Some(path) if !crate::images::ingest::valid_tmdb_path(path) => Err(AppError::BadRequest("invalid-image-path")),
        _ => Ok(()),
    }
}

impl SeasonPatch {
    pub fn validate(&self) -> Result<(), AppError> {
        range(self.number, 0, 999, "invalid-season-number")?;
        opt_text(self.name.clone().flatten().as_deref(), 300, "invalid-name")?;
        opt_text(self.overview.clone().flatten().as_deref(), 5000, "invalid-overview")?;
        range(self.year.flatten(), 1870, 2200, "invalid-year")
    }
}

impl EpisodePatch {
    pub fn validate(&self) -> Result<(), AppError> {
        range(self.number, 1, 9999, "invalid-episode-number")?;
        if let Some(Some(d)) = &self.air_date {
            parse_date(d)?;
        }
        opt_text(self.name.clone().flatten().as_deref(), 300, "invalid-name")?;
        opt_text(self.overview.clone().flatten().as_deref(), 5000, "invalid-overview")?;
        range(self.runtime_minutes.flatten(), 1, 1440, "invalid-runtime")
    }
}

// ---------------------------------------------------------------- public output

/// Responsive image, presigned per UTC day (SPEC 8.3). Null until images arrive in Phase 3b.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageSet {
    /// HSL triple for the skeleton, for example `160 84% 39%`.
    pub placeholder_hsl: Option<String>,
    pub sources: Vec<ImageSource>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageSource {
    /// `avif` or `webp`.
    pub format: String,
    pub width: i16,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Genre {
    pub id: i16,
    /// Also the translation key `genre.<slug>` in the clients.
    pub slug: String,
    /// Fallback when the client has no translation.
    pub name: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TitleCard {
    pub id: Uuid,
    pub kind: TitleKind,
    pub name: String,
    pub year: Option<i16>,
    /// Left out when there is no ready image.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub poster: Option<ImageSet>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TitlePage {
    pub items: Vec<TitleCard>,
    /// Pass back as `cursor` for the next page. Null on the last page.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Credit {
    pub person_name: String,
    pub character: Option<String>,
    pub role: CreditRole,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeasonSummary {
    pub number: i16,
    pub name: Option<String>,
    pub year: Option<i16>,
    pub episode_count: i64,
}

/// The title page (H8).
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TitleDetail {
    pub id: Uuid,
    pub kind: TitleKind,
    pub name: String,
    pub original_name: Option<String>,
    pub overview: Option<String>,
    pub year: Option<i16>,
    pub runtime_minutes: Option<i32>,
    pub genres: Vec<Genre>,
    pub credits: Vec<Credit>,
    pub seasons: Vec<SeasonSummary>,
    /// Content languages with at least one ready variant.
    pub languages: Vec<String>,
    /// Left out when there is no ready image.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub poster: Option<ImageSet>,
    /// Left out when there is no ready image.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub backdrop: Option<ImageSet>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Episode {
    pub id: Uuid,
    pub number: i16,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<i32>,
    #[schema(format = Date)]
    pub air_date: Option<String>,
    /// Content languages with a ready variant. Filled once variants exist (Phase 4).
    pub languages: Vec<String>,
    /// Left out when there is no ready image.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub still: Option<ImageSet>,
}

/// Browse filters, all in the URL so web pages can be shared and restored (SPEC 5.1).
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct BrowseQuery {
    /// `movie` or `show`.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Genre slug.
    #[param(max_length = 40)]
    pub genre: Option<String>,
    /// Content language, BCP 47.
    #[param(max_length = 35)]
    pub language: Option<String>,
    /// Search words. Present means relevance order, `sort` is ignored.
    #[param(max_length = 200)]
    pub q: Option<String>,
    /// `recent` (default) or `az`.
    pub sort: Option<String>,
    #[param(max_length = 512)]
    pub cursor: Option<String>,
    #[param(minimum = 1, maximum = 60)]
    pub limit: Option<i64>,
}

// ---------------------------------------------------------------- admin output

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminTitleRow {
    pub id: Uuid,
    pub kind: TitleKind,
    pub name: String,
    pub year: Option<i16>,
    /// `draft` or `published`.
    pub status: String,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminTitlePage {
    pub items: Vec<AdminTitleRow>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminEpisode {
    pub id: Uuid,
    pub number: i16,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<i32>,
    #[schema(format = Date)]
    pub air_date: Option<String>,
    /// Left out when there is no ready image.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub still: Option<ImageSet>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminSeason {
    pub id: Uuid,
    pub number: i16,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub year: Option<i16>,
    pub episodes: Vec<AdminEpisode>,
}

/// Everything an admin edits on one title, drafts included.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminTitle {
    pub id: Uuid,
    pub kind: TitleKind,
    pub name: String,
    pub original_name: Option<String>,
    pub overview: Option<String>,
    pub year: Option<i16>,
    pub runtime_minutes: Option<i32>,
    pub tmdb_id: Option<i32>,
    pub status: String,
    pub genres: Vec<Genre>,
    pub credits: Vec<Credit>,
    pub seasons: Vec<AdminSeason>,
    /// Left out when there is no ready image.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub poster: Option<ImageSet>,
    /// Left out when there is no ready image.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub backdrop: Option<ImageSet>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct AdminListQuery {
    /// `draft` or `published`, both when left out.
    pub status: Option<String>,
    /// Name search.
    #[param(max_length = 200)]
    pub q: Option<String>,
    #[param(max_length = 512)]
    pub cursor: Option<String>,
    #[param(minimum = 1, maximum = 60)]
    pub limit: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie() -> TitleInput {
        TitleInput {
            kind: TitleKind::Movie,
            name: "Heat".into(),
            original_name: None,
            overview: None,
            year: Some(1995),
            runtime_minutes: Some(170),
            tmdb_id: Some(949),
            genre_ids: vec![],
            new_genres: vec![],
            credits: vec![],
            seasons: vec![],
            poster_path: None,
            backdrop_path: None,
        }
    }

    #[test]
    fn title_input_rules() {
        movie().validate().expect("valid");
        let mut m = movie();
        m.name = "a\u{0}b".into();
        assert!(m.validate().is_err(), "control characters");
        let mut m = movie();
        m.year = Some(1500);
        assert!(m.validate().is_err());
        let mut show = movie();
        show.kind = TitleKind::Show;
        assert!(show.validate().is_err(), "shows have no title runtime");
        let mut m = movie();
        m.seasons.push(SeasonInput {
            number: 1,
            name: None,
            overview: None,
            year: None,
            episodes: vec![],
        });
        assert!(m.validate().is_err(), "movies have no seasons");
        let mut m = movie();
        m.poster_path = Some("https://evil.example/x.jpg".into());
        assert!(m.validate().is_err(), "only TMDB file paths");
    }

    #[test]
    fn null_and_missing_differ_in_patches() {
        let p: TitlePatch = serde_json::from_str(r#"{"overview":null}"#).expect("json");
        assert_eq!(p.overview, Some(None));
        let p: TitlePatch = serde_json::from_str("{}").expect("json");
        assert_eq!(p.overview, None);
        assert!(serde_json::from_str::<TitlePatch>(r#"{"status":"published"}"#).is_err());
    }
}
