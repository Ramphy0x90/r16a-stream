//! Catalog rules. Browsing and search for members, editing for admins.

use serde_json::json;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    dto::{
        AdminEpisode, AdminSeason, AdminTitle, AdminTitlePage, AdminTitleRow, BrowseQuery, Credit, CreditInput,
        CreditRole, Episode, EpisodeInput, EpisodePatch, Genre, GenreInput, SeasonInput, SeasonPatch, SeasonSummary,
        TitleCard, TitleDetail, TitleInput, TitleKind, TitlePage, TitlePatch, format_date, parse_date,
    },
    repository::{self as repo, CardRow, Filters},
};
use crate::{
    admin::audit,
    error::AppError,
    http::cursor,
    images::{repository as image_repo, urls::ImageUrls},
    me::dto::is_language_tag,
};

use super::dto::{ImageSet, ImageSource};

/// Images a save brought in, already stored as sources, written with the title (SPEC 8.2).
#[derive(Debug, Default, Clone, Copy)]
pub struct Imported {
    /// `Some(None)` clears, `Some(Some(..))` sets a new image with its stored size.
    pub poster: Option<Option<(Uuid, i64)>>,
    pub backdrop: Option<Option<(Uuid, i64)>>,
}

/// Ready images as presigned sets, in one query for any number of ids (SPEC 4.10).
pub async fn image_sets(
    db: &PgPool,
    urls: &ImageUrls,
    ids: &[Uuid],
) -> Result<std::collections::HashMap<Uuid, ImageSet>, AppError> {
    let mut out = std::collections::HashMap::<Uuid, ImageSet>::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let now = jiff::Timestamp::now();
    for v in image_repo::ready_variants(db, ids).await? {
        let set = out.entry(v.image_id).or_insert_with(|| ImageSet {
            placeholder_hsl: v.placeholder_hsl.clone(),
            sources: Vec::new(),
        });
        set.sources.push(ImageSource {
            format: v.format,
            width: v.width,
            url: urls.url(&v.object_key, now),
        });
    }
    Ok(out)
}

/// Who made a change and under which request, for the audit log.
#[derive(Clone, Copy)]
pub struct Actor<'a> {
    pub user_id: Uuid,
    pub request_id: Option<&'a str>,
}

/// Turns a genre name into its slug, the i18n key `genre.<slug>`. Latin accents fold to their
/// base letter, other characters become separators. `None` when nothing usable is left.
pub fn slugify(name: &str) -> Option<String> {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        let folded = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            'ß' => 's',
            c if c.is_ascii_alphanumeric() => c,
            _ => '-',
        };
        if folded == '-' {
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
        } else {
            out.push(folded);
        }
    }
    let slug: String = out.trim_matches('-').chars().take(40).collect();
    let slug = slug.trim_end_matches('-').to_owned();
    (slug.len() >= 2).then_some(slug)
}

fn card(r: CardRow, images: &std::collections::HashMap<Uuid, ImageSet>) -> TitleCard {
    TitleCard {
        id: r.id,
        kind: TitleKind::parse(&r.kind).unwrap_or(TitleKind::Movie),
        name: r.name,
        year: r.year,
        poster: r.poster_image_id.and_then(|i| images.get(&i).cloned()),
    }
}

/// Browse or search published titles (H1 to H5).
pub async fn browse(db: &PgPool, urls: &ImageUrls, q: &BrowseQuery) -> Result<TitlePage, AppError> {
    let limit = cursor::limit(q.limit)?;
    let kind = match q.kind.as_deref() {
        None => None,
        Some(k) => Some(
            TitleKind::parse(k)
                .ok_or(AppError::BadRequest("invalid-type"))?
                .as_str(),
        ),
    };
    if q.genre.as_deref().is_some_and(|g| g.len() > 40) {
        return Err(AppError::BadRequest("invalid-genre"));
    }
    if q.language.as_deref().is_some_and(|l| !is_language_tag(l)) {
        return Err(AppError::BadRequest("invalid-language"));
    }
    let filters = Filters {
        kind,
        genre: q.genre.as_deref(),
        language: q.language.as_deref(),
    };
    let search = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if search.is_some_and(|s| s.chars().count() > 200) {
        return Err(AppError::BadRequest("invalid-query"));
    }
    // One extra row tells whether there is a next page.
    let fetch = limit + 1;
    let (mut rows, keys): (Vec<CardRow>, Vec<String>) = if let Some(text) = search {
        let after = cursor::decode::<(f32, Uuid)>(q.cursor.as_deref())?;
        let rows = repo::search(db, &filters, text, after, fetch).await?;
        let keys = rows
            .iter()
            .map(|(r, rank)| cursor::encode(&(rank, r.id)))
            .collect::<Result<_, _>>()?;
        (rows.into_iter().map(|(r, _)| r).collect(), keys)
    } else {
        match q.sort.as_deref().unwrap_or("recent") {
            "recent" => {
                let after = cursor::decode::<(i128, Uuid)>(q.cursor.as_deref())?
                    .map(|(nanos, id)| {
                        OffsetDateTime::from_unix_timestamp_nanos(nanos)
                            .map(|t| (t, id))
                            .map_err(|_| AppError::BadRequest("invalid-cursor"))
                    })
                    .transpose()?;
                let rows = repo::browse_recent(db, &filters, after, fetch).await?;
                let keys = rows
                    .iter()
                    .map(|(r, at)| cursor::encode(&(at.unix_timestamp_nanos(), r.id)))
                    .collect::<Result<_, _>>()?;
                (rows.into_iter().map(|(r, _)| r).collect(), keys)
            }
            "az" => {
                let after = cursor::decode::<(String, Uuid)>(q.cursor.as_deref())?;
                let rows = repo::browse_az(db, &filters, after, fetch).await?;
                let keys = rows
                    .iter()
                    .map(|(r, name)| cursor::encode(&(name, r.id)))
                    .collect::<Result<_, _>>()?;
                (rows.into_iter().map(|(r, _)| r).collect(), keys)
            }
            _ => return Err(AppError::BadRequest("invalid-sort")),
        }
    };
    let page_len = usize::try_from(limit).unwrap_or(30);
    let next_cursor = if rows.len() > page_len {
        rows.truncate(page_len);
        keys.get(page_len - 1).cloned()
    } else {
        None
    };
    let poster_ids: Vec<Uuid> = rows.iter().filter_map(|r| r.poster_image_id).collect();
    let images = image_sets(db, urls, &poster_ids).await?;
    let items = rows.into_iter().map(|r| card(r, &images)).collect();
    Ok(TitlePage { items, next_cursor })
}

fn genre(g: repo::GenreRow) -> Genre {
    Genre {
        id: g.id,
        slug: g.slug,
        name: g.name,
    }
}

fn credit(c: repo::CreditRow) -> Credit {
    Credit {
        person_name: c.person_name,
        character: c.character,
        role: CreditRole::parse(&c.role).unwrap_or(CreditRole::Cast),
    }
}

/// The title page of a published title (H8), in a fixed number of queries.
pub async fn title_detail(db: &PgPool, urls: &ImageUrls, id: Uuid) -> Result<TitleDetail, AppError> {
    let t = repo::title(db, id, true).await?.ok_or(AppError::NotFound)?;
    let ids: Vec<Uuid> = [t.poster_image_id, t.backdrop_image_id].into_iter().flatten().collect();
    let mut images = image_sets(db, urls, &ids).await?;
    let (genres, credits, seasons, languages) = tokio::try_join!(
        repo::title_genres(db, id),
        repo::title_credits(db, id),
        repo::seasons(db, id),
        repo::title_languages(db, id),
    )?;
    Ok(TitleDetail {
        id: t.id,
        kind: TitleKind::parse(&t.kind).unwrap_or(TitleKind::Movie),
        name: t.name,
        original_name: t.original_name,
        overview: t.overview,
        year: t.year,
        runtime_minutes: t.runtime_minutes,
        genres: genres.into_iter().map(genre).collect(),
        credits: credits.into_iter().map(credit).collect(),
        seasons: seasons
            .into_iter()
            .map(|s| SeasonSummary {
                number: s.number,
                name: s.name,
                year: s.year,
                episode_count: s.episode_count,
            })
            .collect(),
        languages,
        poster: t.poster_image_id.and_then(|i| images.remove(&i)),
        backdrop: t.backdrop_image_id.and_then(|i| images.remove(&i)),
    })
}

pub async fn season_episodes(
    db: &PgPool,
    urls: &ImageUrls,
    title_id: Uuid,
    number: i16,
) -> Result<Vec<Episode>, AppError> {
    let rows = repo::published_season_episodes(db, title_id, number)
        .await?
        .ok_or(AppError::NotFound)?;
    let ids: Vec<Uuid> = rows.iter().filter_map(|e| e.still_image_id).collect();
    let mut stills = image_sets(db, urls, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|e| Episode {
            id: e.id,
            number: e.number,
            name: e.name,
            overview: e.overview,
            runtime_minutes: e.runtime_minutes,
            air_date: e.air_date.map(format_date),
            still: e.still_image_id.and_then(|i| stills.remove(&i)),
            languages: e.languages,
        })
        .collect())
}

pub async fn genres(db: &PgPool) -> Result<Vec<Genre>, AppError> {
    Ok(repo::genres(db).await?.into_iter().map(genre).collect())
}

// ---------------------------------------------------------------- admin

pub async fn admin_list(
    db: &PgPool,
    status: Option<&str>,
    q: Option<&str>,
    raw_cursor: Option<&str>,
    raw_limit: Option<i64>,
) -> Result<AdminTitlePage, AppError> {
    let limit = cursor::limit(raw_limit)?;
    if status.is_some_and(|s| s != "draft" && s != "published") {
        return Err(AppError::BadRequest("invalid-status"));
    }
    let q = q.map(str::trim).filter(|s| !s.is_empty());
    if q.is_some_and(|s| s.chars().count() > 200) {
        return Err(AppError::BadRequest("invalid-query"));
    }
    let after = cursor::decode::<(i128, Uuid)>(raw_cursor)?
        .map(|(nanos, id)| {
            OffsetDateTime::from_unix_timestamp_nanos(nanos)
                .map(|t| (t, id))
                .map_err(|_| AppError::BadRequest("invalid-cursor"))
        })
        .transpose()?;
    let mut rows = repo::admin_list(db, status, q, after, limit + 1).await?;
    let page_len = usize::try_from(limit).unwrap_or(30);
    let next_cursor = if rows.len() > page_len {
        rows.truncate(page_len);
        rows.last()
            .map(|r| cursor::encode(&(r.updated_at.unix_timestamp_nanos(), r.id)))
            .transpose()?
    } else {
        None
    };
    Ok(AdminTitlePage {
        items: rows
            .into_iter()
            .map(|r| AdminTitleRow {
                id: r.id,
                kind: TitleKind::parse(&r.kind).unwrap_or(TitleKind::Movie),
                name: r.name,
                year: r.year,
                status: r.status,
                updated_at: r.updated_at,
            })
            .collect(),
        next_cursor,
    })
}

pub async fn admin_title(db: &PgPool, urls: &ImageUrls, id: Uuid) -> Result<AdminTitle, AppError> {
    let t = repo::title(db, id, false).await?.ok_or(AppError::NotFound)?;
    let ids: Vec<Uuid> = [t.poster_image_id, t.backdrop_image_id].into_iter().flatten().collect();
    let mut images = image_sets(db, urls, &ids).await?;
    let (genres, credits, seasons, episodes) = tokio::try_join!(
        repo::title_genres(db, id),
        repo::title_credits(db, id),
        repo::seasons(db, id),
        repo::all_episodes(db, id),
    )?;
    let still_ids: Vec<Uuid> = episodes.iter().filter_map(|e| e.still_image_id).collect();
    let stills = image_sets(db, urls, &still_ids).await?;
    let seasons = seasons
        .into_iter()
        .map(|s| AdminSeason {
            id: s.id,
            number: s.number,
            name: s.name,
            overview: s.overview,
            year: s.year,
            episodes: episodes
                .iter()
                .filter(|e| e.season_id == s.id)
                .map(|e| AdminEpisode {
                    id: e.id,
                    number: e.number,
                    name: e.name.clone(),
                    overview: e.overview.clone(),
                    runtime_minutes: e.runtime_minutes,
                    air_date: e.air_date.map(format_date),
                    still: e.still_image_id.and_then(|i| stills.get(&i).cloned()),
                })
                .collect(),
        })
        .collect();
    Ok(AdminTitle {
        id: t.id,
        kind: TitleKind::parse(&t.kind).unwrap_or(TitleKind::Movie),
        name: t.name,
        original_name: t.original_name,
        overview: t.overview,
        year: t.year,
        runtime_minutes: t.runtime_minutes,
        tmdb_id: t.tmdb_id,
        status: t.status,
        genres: genres.into_iter().map(genre).collect(),
        credits: credits.into_iter().map(credit).collect(),
        seasons,
        poster: t.poster_image_id.and_then(|i| images.remove(&i)),
        backdrop: t.backdrop_image_id.and_then(|i| images.remove(&i)),
    })
}

/// Maps a unique violation to a 409 with a slug that names what collided.
fn conflict(e: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(d) = &e
        && d.code().as_deref() == Some("23505")
    {
        return AppError::Conflict(match d.constraint() {
            Some("titles_kind_tmdb_id_key") => "tmdb-id-exists",
            Some("seasons_title_id_number_key") => "season-number-exists",
            Some("episodes_season_id_number_key") => "episode-number-exists",
            Some("genres_slug_key" | "genres_tmdb_id_key") => "genre-exists",
            _ => "already-exists",
        });
    }
    e.into()
}

/// Resolves the genre ids of a title, creating genres that do not exist yet.
async fn resolve_genres(db: &mut PgConnection, ids: &[i16], new: &[GenreInput]) -> Result<Vec<i16>, AppError> {
    let mut out: Vec<i16> = ids.to_vec();
    out.sort_unstable();
    out.dedup();
    if repo::count_genres(db, &out).await? != i64::try_from(out.len()).unwrap_or(i64::MAX) {
        return Err(AppError::BadRequest("unknown-genre"));
    }
    for g in new {
        let name = g.name.trim();
        let slug = slugify(name).ok_or(AppError::BadRequest("invalid-genre"))?;
        let found = match g.tmdb_id {
            Some(t) => repo::genre_by_tmdb(db, t).await?,
            None => None,
        };
        let id = match found {
            Some(id) => id,
            None => match repo::genre_by_slug(db, &slug, g.tmdb_id).await? {
                Some(id) => id,
                None => repo::insert_genre(db, &slug, name, g.tmdb_id).await.map_err(conflict)?,
            },
        };
        out.push(id);
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

fn credit_batch(credits: &[CreditInput]) -> repo::CreditBatch {
    let mut b = repo::CreditBatch {
        ord: Vec::new(),
        person: Vec::new(),
        character: Vec::new(),
        role: Vec::new(),
    };
    let mut counters = std::collections::HashMap::<&str, i16>::new();
    for c in credits {
        let role = c.role.as_str();
        let ord = counters.entry(role).or_insert(0);
        b.ord.push(*ord);
        *ord += 1;
        b.person.push(c.person_name.trim().to_owned());
        b.character.push(
            c.character
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from),
        );
        b.role.push(role.to_owned());
    }
    b
}

fn episode_batch(episodes: &[EpisodeInput]) -> Result<repo::EpisodeBatch, AppError> {
    let mut b = repo::EpisodeBatch {
        number: Vec::new(),
        name: Vec::new(),
        overview: Vec::new(),
        runtime: Vec::new(),
        air_date: Vec::new(),
    };
    for e in episodes {
        b.number.push(e.number);
        b.name.push(e.name.clone());
        b.overview.push(e.overview.clone());
        b.runtime.push(e.runtime_minutes);
        b.air_date.push(e.air_date.as_deref().map(parse_date).transpose()?);
    }
    Ok(b)
}

async fn insert_season_with_episodes(db: &mut PgConnection, title_id: Uuid, s: &SeasonInput) -> Result<Uuid, AppError> {
    let season_id = repo::insert_season(
        db,
        repo::NewSeason {
            title_id,
            number: s.number,
            name: s.name.as_deref(),
            overview: s.overview.as_deref(),
            year: s.year,
        },
    )
    .await
    .map_err(conflict)?;
    if !s.episodes.is_empty() {
        repo::insert_episodes(db, season_id, title_id, &episode_batch(&s.episodes)?)
            .await
            .map_err(conflict)?;
    }
    Ok(season_id)
}

async fn audit(
    db: &mut PgConnection,
    actor: Actor<'_>,
    action: &str,
    kind: &str,
    id: Uuid,
    details: serde_json::Value,
) -> Result<(), AppError> {
    audit::record(
        db,
        audit::Entry {
            actor_id: actor.user_id,
            action,
            target_kind: kind,
            target_id: Some(id.to_string()),
            details,
            request_id: actor.request_id,
        },
    )
    .await?;
    Ok(())
}

/// Writes imported images with their jobs and points the title at them. A replaced image row
/// stays until encode GC removes images no title references (Phase 4).
async fn store_images(db: &mut PgConnection, title_id: Uuid, imported: Imported) -> Result<(), AppError> {
    for (slot, kind) in [(imported.poster, "poster"), (imported.backdrop, "backdrop")] {
        if let Some(Some((image_id, bytes))) = slot {
            image_repo::insert_with_job(db, image_id, kind, &crate::images::source_key(image_id), bytes).await?;
        }
    }
    let pick = |slot: Option<Option<(Uuid, i64)>>| slot.map(|s| s.map(|(id, _)| id));
    repo::set_title_images(db, title_id, pick(imported.poster), pick(imported.backdrop)).await?;
    Ok(())
}

/// Creates a draft with its genres, credits, seasons and episodes, in one transaction.
pub async fn create_title(
    db: &PgPool,
    actor: Actor<'_>,
    input: &TitleInput,
    imported: Imported,
) -> Result<Uuid, AppError> {
    input.validate()?;
    let mut tx = db.begin().await?;
    let id = repo::insert_title(
        &mut tx,
        repo::NewTitle {
            kind: input.kind.as_str(),
            name: input.name.trim(),
            original_name: input.original_name.as_deref(),
            overview: input.overview.as_deref(),
            year: input.year,
            runtime_minutes: input.runtime_minutes,
            tmdb_id: input.tmdb_id,
        },
    )
    .await
    .map_err(conflict)?;
    let genre_ids = resolve_genres(&mut tx, &input.genre_ids, &input.new_genres).await?;
    repo::set_title_genres(&mut tx, id, &genre_ids).await?;
    repo::set_title_credits(&mut tx, id, &credit_batch(&input.credits)).await?;
    for s in &input.seasons {
        insert_season_with_episodes(&mut tx, id, s).await?;
    }
    store_images(&mut tx, id, imported).await?;
    audit(
        &mut tx,
        actor,
        "title.create",
        "title",
        id,
        json!({ "kind": input.kind.as_str(), "seasons": input.seasons.len(), "tmdbId": input.tmdb_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn update_title(
    db: &PgPool,
    actor: Actor<'_>,
    id: Uuid,
    p: &TitlePatch,
    imported: Imported,
) -> Result<(), AppError> {
    p.validate()?;
    let mut tx = db.begin().await?;
    let kind = repo::update_title(
        &mut tx,
        id,
        &repo::TitleUpdate {
            name: p.name.as_deref().map(str::trim),
            original_name: p.original_name.as_ref().map(Option::as_deref),
            overview: p.overview.as_ref().map(Option::as_deref),
            year: p.year,
            runtime_minutes: p.runtime_minutes,
        },
    )
    .await?
    .ok_or(AppError::NotFound)?;
    if kind == "show" && p.runtime_minutes.flatten().is_some() {
        return Err(AppError::BadRequest("runtime-is-per-episode"));
    }
    if p.genre_ids.is_some() || p.new_genres.is_some() {
        let ids = resolve_genres(
            &mut tx,
            p.genre_ids.as_deref().unwrap_or_default(),
            p.new_genres.as_deref().unwrap_or_default(),
        )
        .await?;
        repo::set_title_genres(&mut tx, id, &ids).await?;
    }
    if let Some(credits) = &p.credits {
        repo::set_title_credits(&mut tx, id, &credit_batch(credits)).await?;
    }
    store_images(&mut tx, id, imported).await?;
    let mut fields = Vec::new();
    for (name, set) in [
        ("name", p.name.is_some()),
        ("originalName", p.original_name.is_some()),
        ("overview", p.overview.is_some()),
        ("year", p.year.is_some()),
        ("runtimeMinutes", p.runtime_minutes.is_some()),
        ("genres", p.genre_ids.is_some() || p.new_genres.is_some()),
        ("credits", p.credits.is_some()),
        ("poster", p.poster_path.is_some()),
        ("backdrop", p.backdrop_path.is_some()),
    ] {
        if set {
            fields.push(name);
        }
    }
    audit(&mut tx, actor, "title.update", "title", id, json!({ "fields": fields })).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_published(db: &PgPool, actor: Actor<'_>, id: Uuid, published: bool) -> Result<(), AppError> {
    let mut tx = db.begin().await?;
    let found = if published {
        repo::publish(&mut tx, id).await?
    } else {
        repo::unpublish(&mut tx, id).await?
    };
    if !found {
        return Err(AppError::NotFound);
    }
    let action = if published { "title.publish" } else { "title.unpublish" };
    audit(&mut tx, actor, action, "title", id, json!({})).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn delete_title(db: &PgPool, actor: Actor<'_>, id: Uuid) -> Result<(), AppError> {
    let mut tx = db.begin().await?;
    if !repo::delete_title(&mut tx, id).await? {
        return Err(AppError::NotFound);
    }
    audit(&mut tx, actor, "title.delete", "title", id, json!({})).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn create_season(db: &PgPool, actor: Actor<'_>, title_id: Uuid, s: &SeasonInput) -> Result<Uuid, AppError> {
    super::dto::validate_season(s)?;
    let mut tx = db.begin().await?;
    match repo::title_kind_for_update(&mut tx, title_id).await?.as_deref() {
        None => return Err(AppError::NotFound),
        Some("show") => {}
        Some(_) => return Err(AppError::Conflict("not-a-show")),
    }
    let season_id = insert_season_with_episodes(&mut tx, title_id, s).await?;
    repo::touch_title(&mut tx, title_id).await?;
    audit(
        &mut tx,
        actor,
        "season.create",
        "season",
        season_id,
        json!({ "titleId": title_id, "number": s.number, "episodes": s.episodes.len() }),
    )
    .await?;
    tx.commit().await?;
    Ok(season_id)
}

pub async fn update_season(db: &PgPool, actor: Actor<'_>, id: Uuid, p: &SeasonPatch) -> Result<(), AppError> {
    p.validate()?;
    let mut tx = db.begin().await?;
    let title_id = repo::update_season(
        &mut tx,
        id,
        &repo::SeasonUpdate {
            number: p.number,
            name: p.name.as_ref().map(Option::as_deref),
            overview: p.overview.as_ref().map(Option::as_deref),
            year: p.year,
        },
    )
    .await
    .map_err(conflict)?
    .ok_or(AppError::NotFound)?;
    repo::touch_title(&mut tx, title_id).await?;
    audit(
        &mut tx,
        actor,
        "season.update",
        "season",
        id,
        json!({ "titleId": title_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn delete_season(db: &PgPool, actor: Actor<'_>, id: Uuid) -> Result<(), AppError> {
    let mut tx = db.begin().await?;
    let title_id = repo::delete_season(&mut tx, id).await?.ok_or(AppError::NotFound)?;
    repo::touch_title(&mut tx, title_id).await?;
    audit(
        &mut tx,
        actor,
        "season.delete",
        "season",
        id,
        json!({ "titleId": title_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn create_episode(
    db: &PgPool,
    actor: Actor<'_>,
    season_id: Uuid,
    e: &EpisodeInput,
) -> Result<Uuid, AppError> {
    super::dto::validate_episode(e)?;
    let mut tx = db.begin().await?;
    let title_id = repo::season_title(&mut tx, season_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let id = repo::insert_episodes(&mut tx, season_id, title_id, &episode_batch(std::slice::from_ref(e))?)
        .await
        .map_err(conflict)?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::internal("episode insert returned no id"))?;
    repo::touch_title(&mut tx, title_id).await?;
    audit(
        &mut tx,
        actor,
        "episode.create",
        "episode",
        id,
        json!({ "titleId": title_id, "number": e.number }),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn update_episode(db: &PgPool, actor: Actor<'_>, id: Uuid, p: &EpisodePatch) -> Result<(), AppError> {
    p.validate()?;
    let air_date = match &p.air_date {
        None => None,
        Some(None) => Some(None),
        Some(Some(d)) => Some(Some(parse_date(d)?)),
    };
    let mut tx = db.begin().await?;
    let title_id = repo::update_episode(
        &mut tx,
        id,
        &repo::EpisodeUpdate {
            number: p.number,
            name: p.name.as_ref().map(Option::as_deref),
            overview: p.overview.as_ref().map(Option::as_deref),
            runtime_minutes: p.runtime_minutes,
            air_date,
        },
    )
    .await
    .map_err(conflict)?
    .ok_or(AppError::NotFound)?;
    repo::touch_title(&mut tx, title_id).await?;
    audit(
        &mut tx,
        actor,
        "episode.update",
        "episode",
        id,
        json!({ "titleId": title_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn delete_episode(db: &PgPool, actor: Actor<'_>, id: Uuid) -> Result<(), AppError> {
    let mut tx = db.begin().await?;
    let title_id = repo::delete_episode(&mut tx, id).await?.ok_or(AppError::NotFound)?;
    repo::touch_title(&mut tx, title_id).await?;
    audit(
        &mut tx,
        actor,
        "episode.delete",
        "episode",
        id,
        json!({ "titleId": title_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn create_genre(db: &PgPool, actor: Actor<'_>, name: &str) -> Result<Genre, AppError> {
    super::dto::validate_genres(
        &[],
        &[GenreInput {
            tmdb_id: None,
            name: name.to_owned(),
        }],
    )?;
    let name = name.trim();
    let slug = slugify(name).ok_or(AppError::BadRequest("invalid-genre"))?;
    let mut tx = db.begin().await?;
    let id = repo::insert_genre(&mut tx, &slug, name, None).await.map_err(conflict)?;
    audit::record(
        &mut tx,
        audit::Entry {
            actor_id: actor.user_id,
            action: "genre.create",
            target_kind: "genre",
            target_id: Some(id.to_string()),
            details: json!({ "slug": slug }),
            request_id: actor.request_id,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Genre {
        id,
        slug,
        name: name.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_fold_accents_and_separators() {
        assert_eq!(slugify("Science Fiction").as_deref(), Some("science-fiction"));
        assert_eq!(slugify("Ciencia ficción").as_deref(), Some("ciencia-ficcion"));
        assert_eq!(slugify("Action & Adventure").as_deref(), Some("action-adventure"));
        assert_eq!(slugify("  War & Politics  ").as_deref(), Some("war-politics"));
        assert_eq!(slugify("!"), None);
        assert_eq!(slugify("日本"), None);
        assert!(slugify(&"x".repeat(80)).is_some_and(|s| s.len() == 40));
    }
}
