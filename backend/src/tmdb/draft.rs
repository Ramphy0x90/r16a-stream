//! Turns TMDB responses into a [`TitleInput`] draft.

use serde_json::Value;
use sqlx::PgPool;

use super::{
    client::TmdbClient,
    reference::{TmdbKind, TmdbRef},
};
use crate::{
    catalog::dto::{
        CreditInput, CreditRole, EpisodeInput, GenreInput, MAX_CREDITS, MAX_EPISODES_PER_SEASON, MAX_GENRES,
        MAX_SEASONS, SeasonInput, TitleInput, TitleKind,
    },
    error::AppError,
};

/// Cast members kept from TMDB. Directors, creators and writers come on top.
const MAX_CAST: usize = 20;

/// Text from TMDB, without control characters except newlines, trimmed and cut to `max`
/// characters. Empty becomes `None`.
fn clean(v: &Value, max: usize) -> Option<String> {
    let s: String = v
        .as_str()?
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect::<String>()
        .trim()
        .chars()
        .take(max)
        .collect();
    (!s.is_empty()).then_some(s)
}

fn int<T: TryFrom<i64>>(v: &Value, lo: i64, hi: i64) -> Option<T> {
    v.as_i64()
        .filter(|n| (lo..=hi).contains(n))
        .and_then(|n| T::try_from(n).ok())
}

/// A TMDB image file path, only when it has the expected plain form.
fn image_path(v: &Value) -> Option<String> {
    v.as_str()
        .filter(|p| crate::images::ingest::valid_tmdb_path(p))
        .map(String::from)
}

/// The year of a TMDB date such as `1995-12-15`.
fn year(v: &Value) -> Option<i16> {
    let s = v.as_str()?;
    s.get(..4)?.parse::<i16>().ok().filter(|y| (1870..=2200).contains(y))
}

/// A TMDB date when it is a valid calendar date.
fn date(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    crate::catalog::dto::parse_date(s)
        .ok()
        .map(crate::catalog::dto::format_date)
}

fn genres(v: &Value) -> Vec<GenreInput> {
    v["genres"].as_array().map_or_else(Vec::new, |a| {
        a.iter()
            .filter_map(|g| {
                Some(GenreInput {
                    tmdb_id: int(&g["id"], 1, i64::from(i32::MAX)),
                    name: clean(&g["name"], 60)?,
                })
            })
            .filter(|g| crate::catalog::service::slugify(&g.name).is_some())
            .take(MAX_GENRES)
            .collect()
    })
}

fn credits(v: &Value, kind: TmdbKind) -> Vec<CreditInput> {
    let mut out = Vec::new();
    if kind == TmdbKind::Tv
        && let Some(creators) = v["created_by"].as_array()
    {
        out.extend(creators.iter().filter_map(|c| {
            Some(CreditInput {
                person_name: clean(&c["name"], 200)?,
                character: None,
                role: CreditRole::Creator,
            })
        }));
    }
    if let Some(crew) = v["credits"]["crew"].as_array() {
        out.extend(crew.iter().filter_map(|c| {
            let role = match c["job"].as_str()? {
                "Director" => CreditRole::Director,
                "Screenplay" | "Writer" => CreditRole::Writer,
                _ => return None,
            };
            Some(CreditInput {
                person_name: clean(&c["name"], 200)?,
                character: None,
                role,
            })
        }));
    }
    if let Some(cast) = v["credits"]["cast"].as_array() {
        out.extend(cast.iter().take(MAX_CAST).filter_map(|c| {
            Some(CreditInput {
                person_name: clean(&c["name"], 200)?,
                character: clean(&c["character"], 200),
                role: CreditRole::Cast,
            })
        }));
    }
    out.truncate(MAX_CREDITS);
    out
}

fn episodes(season: &Value) -> Vec<EpisodeInput> {
    let mut seen = std::collections::HashSet::new();
    season["episodes"].as_array().map_or_else(Vec::new, |a| {
        a.iter()
            .filter_map(|e| {
                let number: i16 = int(&e["episode_number"], 1, 9999)?;
                seen.insert(number).then(|| EpisodeInput {
                    number,
                    name: clean(&e["name"], 300),
                    overview: clean(&e["overview"], 5000),
                    runtime_minutes: int(&e["runtime"], 1, 1440),
                    air_date: date(&e["air_date"]),
                })
            })
            .take(MAX_EPISODES_PER_SEASON)
            .collect()
    })
}

/// Looks the title up on TMDB and returns an editable draft. Writes only the response cache.
pub async fn lookup(db: &PgPool, tmdb: &TmdbClient, r: TmdbRef) -> Result<TitleInput, AppError> {
    let id = r.id.to_string();
    let main = tmdb.get(db, &[r.kind.segment(), &id], Some("credits")).await?;
    let tmdb_id = int(&main["id"], 1, i64::from(i32::MAX)).ok_or(AppError::Unavailable)?;
    let (name_key, original_key, date_key) = match r.kind {
        TmdbKind::Movie => ("title", "original_title", "release_date"),
        TmdbKind::Tv => ("name", "original_name", "first_air_date"),
    };
    let name = clean(&main[name_key], 300).ok_or(AppError::Unavailable)?;
    let original_name = clean(&main[original_key], 300).filter(|o| *o != name);
    let mut seasons = Vec::new();
    if r.kind == TmdbKind::Tv {
        let numbers: Vec<i16> = main["seasons"].as_array().map_or_else(Vec::new, |a| {
            a.iter().filter_map(|s| int(&s["season_number"], 0, 999)).collect()
        });
        let mut seen = std::collections::HashSet::new();
        for n in numbers.into_iter().filter(|n| seen.insert(*n)).take(MAX_SEASONS) {
            let s = tmdb.get(db, &["tv", &id, "season", &n.to_string()], None).await?;
            seasons.push(SeasonInput {
                number: n,
                name: clean(&s["name"], 300),
                overview: clean(&s["overview"], 5000),
                year: year(&s["air_date"]),
                episodes: episodes(&s),
            });
        }
    }
    Ok(TitleInput {
        kind: match r.kind {
            TmdbKind::Movie => TitleKind::Movie,
            TmdbKind::Tv => TitleKind::Show,
        },
        name,
        original_name,
        overview: clean(&main["overview"], 5000),
        year: year(&main[date_key]),
        runtime_minutes: if r.kind == TmdbKind::Movie {
            int(&main["runtime"], 1, 1440)
        } else {
            None
        },
        tmdb_id: Some(tmdb_id),
        genre_ids: Vec::new(),
        new_genres: genres(&main),
        credits: credits(&main, r.kind),
        seasons,
        poster_path: image_path(&main["poster_path"]),
        backdrop_path: image_path(&main["backdrop_path"]),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn hostile_tmdb_text_is_cleaned_and_bounded() {
        assert_eq!(clean(&json!("  a\u{0}b\nc  "), 10).as_deref(), Some("ab\nc"));
        assert_eq!(clean(&json!(""), 10), None);
        assert_eq!(clean(&json!(42), 10), None);
        assert_eq!(clean(&json!("x".repeat(50)), 10).map(|s| s.len()), Some(10));
        assert_eq!(year(&json!("1995-12-15")), Some(1995));
        assert_eq!(year(&json!("0001-01-01")), None);
        assert_eq!(date(&json!("2008-01-20")).as_deref(), Some("2008-01-20"));
        assert_eq!(date(&json!("2008-13-40")), None);
        assert_eq!(int::<i32>(&json!(-5), 1, 10), None);
    }

    #[test]
    fn credits_keep_directors_writers_and_capped_cast() {
        let v = json!({
            "credits": {
                "crew": [{"name": "Michael Mann", "job": "Director"}, {"name": "X", "job": "Grip"}],
                "cast": (0..30).map(|i| json!({"name": format!("A{i}"), "character": "C"})).collect::<Vec<_>>()
            }
        });
        let c = credits(&v, TmdbKind::Movie);
        assert_eq!(c[0].role, CreditRole::Director);
        assert_eq!(c.iter().filter(|c| c.role == CreditRole::Cast).count(), MAX_CAST);
        assert!(c.iter().all(|c| c.person_name != "X"));
    }
}
