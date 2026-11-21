//! SQL for the catalog. Every user-facing query carries `status = 'published'` (SPEC 6.6).

use sqlx::{PgConnection, PgPool};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

// ---------------------------------------------------------------- public reads

pub struct CardRow {
    pub id: Uuid,
    pub kind: String,
    pub name: String,
    pub year: Option<i16>,
    pub poster_image_id: Option<Uuid>,
}

pub struct Filters<'a> {
    pub kind: Option<&'a str>,
    pub genre: Option<&'a str>,
    pub language: Option<&'a str>,
}

/// H1 to H3, newest first, keyset on `(listed_at, id)`.
pub async fn browse_recent(
    db: &PgPool,
    f: &Filters<'_>,
    after: Option<(OffsetDateTime, Uuid)>,
    limit: i64,
) -> Result<Vec<(CardRow, OffsetDateTime)>, sqlx::Error> {
    let (after_at, after_id) = after.map_or((None, None), |(a, i)| (Some(a), Some(i)));
    let rows = sqlx::query!(
        r#"select t.id, t.kind, t.name, t.year, t.poster_image_id, t.listed_at as "listed_at!"
           from titles t
           where t.status = 'published'
             and ($1::text is null or t.kind = $1)
             and ($2::text is null or exists (
                   select 1 from title_genres tg join genres g on g.id = tg.genre_id
                   where tg.title_id = t.id and g.slug = $2))
             and ($3::text is null or exists (
                   select 1 from title_languages tl where tl.title_id = t.id and tl.language = $3))
             and ($4::timestamptz is null or (t.listed_at, t.id) < ($4, $5::uuid))
           order by t.listed_at desc, t.id desc
           limit $6"#,
        f.kind,
        f.genre,
        f.language,
        after_at,
        after_id,
        limit,
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                CardRow {
                    id: r.id,
                    kind: r.kind,
                    name: r.name,
                    year: r.year,
                    poster_image_id: r.poster_image_id,
                },
                r.listed_at,
            )
        })
        .collect())
}

/// H4, A to Z, keyset on `(sort_name, id)`.
pub async fn browse_az(
    db: &PgPool,
    f: &Filters<'_>,
    after: Option<(String, Uuid)>,
    limit: i64,
) -> Result<Vec<(CardRow, String)>, sqlx::Error> {
    let (after_name, after_id) = after.map_or((None, None), |(a, i)| (Some(a), Some(i)));
    let rows = sqlx::query!(
        r#"select t.id, t.kind, t.name, t.year, t.poster_image_id, t.sort_name
           from titles t
           where t.status = 'published'
             and ($1::text is null or t.kind = $1)
             and ($2::text is null or exists (
                   select 1 from title_genres tg join genres g on g.id = tg.genre_id
                   where tg.title_id = t.id and g.slug = $2))
             and ($3::text is null or exists (
                   select 1 from title_languages tl where tl.title_id = t.id and tl.language = $3))
             and ($4::text is null or (t.sort_name, t.id) > ($4, $5::uuid))
           order by t.sort_name, t.id
           limit $6"#,
        f.kind,
        f.genre,
        f.language,
        after_name,
        after_id,
        limit,
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                CardRow {
                    id: r.id,
                    kind: r.kind,
                    name: r.name,
                    year: r.year,
                    poster_image_id: r.poster_image_id,
                },
                r.sort_name,
            )
        })
        .collect())
}

/// H5. Full-text match on the `simple` configuration, or trigram similarity on the name for
/// typos and partial words. Ranked, keyset on `(rank, id)`. The rank is computed the same way
/// on every page, so comparing it with the cursor is exact.
pub async fn search(
    db: &PgPool,
    f: &Filters<'_>,
    q: &str,
    after: Option<(f32, Uuid)>,
    limit: i64,
) -> Result<Vec<(CardRow, f32)>, sqlx::Error> {
    let (after_rank, after_id) = after.map_or((None, None), |(a, i)| (Some(a), Some(i)));
    let rows = sqlx::query!(
        r#"with ranked as (
             select t.id, t.kind, t.name, t.year, t.poster_image_id,
                    (ts_rank(t.search_doc, websearch_to_tsquery('simple', f_unaccent($1)))
                      + similarity(f_unaccent(lower(t.name)), f_unaccent(lower($1))))::float4 as rank
             from titles t
             where t.status = 'published'
               -- Written inline, not through a CTE, so each side matches its GIN index and the
               -- planner combines them with a BitmapOr (measured in docs/perf/queries.md).
               and (t.search_doc @@ websearch_to_tsquery('simple', f_unaccent($1))
                    or f_unaccent(lower(t.name)) % f_unaccent(lower($1)))
               and ($2::text is null or t.kind = $2)
               and ($3::text is null or exists (
                     select 1 from title_genres tg join genres g on g.id = tg.genre_id
                     where tg.title_id = t.id and g.slug = $3))
               and ($4::text is null or exists (
                     select 1 from title_languages tl where tl.title_id = t.id and tl.language = $4))
           )
           select id, kind, name, year, poster_image_id, rank as "rank!" from ranked
           where ($5::float4 is null or (rank, id) < ($5, $6::uuid))
           order by rank desc, id desc
           limit $7"#,
        q,
        f.kind,
        f.genre,
        f.language,
        after_rank,
        after_id,
        limit,
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                CardRow {
                    id: r.id,
                    kind: r.kind,
                    name: r.name,
                    year: r.year,
                    poster_image_id: r.poster_image_id,
                },
                r.rank,
            )
        })
        .collect())
}

pub struct TitleRow {
    pub id: Uuid,
    pub kind: String,
    pub name: String,
    pub original_name: Option<String>,
    pub overview: Option<String>,
    pub year: Option<i16>,
    pub runtime_minutes: Option<i32>,
    pub tmdb_id: Option<i32>,
    pub status: String,
    pub poster_image_id: Option<Uuid>,
    pub backdrop_image_id: Option<Uuid>,
}

/// One title. `published_only` is true for every user-facing read.
pub async fn title(db: &PgPool, id: Uuid, published_only: bool) -> Result<Option<TitleRow>, sqlx::Error> {
    sqlx::query_as!(
        TitleRow,
        "select id, kind, name, original_name, overview, year, runtime_minutes, tmdb_id, status,
                poster_image_id, backdrop_image_id
         from titles where id = $1 and (not $2 or status = 'published')",
        id,
        published_only,
    )
    .fetch_optional(db)
    .await
}

pub struct GenreRow {
    pub id: i16,
    pub slug: String,
    pub name: String,
}

pub async fn title_genres(db: &PgPool, title_id: Uuid) -> Result<Vec<GenreRow>, sqlx::Error> {
    sqlx::query_as!(
        GenreRow,
        "select g.id, g.slug, g.name from title_genres tg join genres g on g.id = tg.genre_id
         where tg.title_id = $1 order by g.name",
        title_id
    )
    .fetch_all(db)
    .await
}

pub struct CreditRow {
    pub person_name: String,
    pub character: Option<String>,
    pub role: String,
}

pub async fn title_credits(db: &PgPool, title_id: Uuid) -> Result<Vec<CreditRow>, sqlx::Error> {
    sqlx::query_as!(
        CreditRow,
        "select person_name, character, role from title_credits where title_id = $1
         order by case role when 'director' then 0 when 'creator' then 1 when 'writer' then 2 else 3 end, ord",
        title_id
    )
    .fetch_all(db)
    .await
}

pub struct SeasonRow {
    pub id: Uuid,
    pub number: i16,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub year: Option<i16>,
    pub episode_count: i64,
}

pub async fn seasons(db: &PgPool, title_id: Uuid) -> Result<Vec<SeasonRow>, sqlx::Error> {
    sqlx::query_as!(
        SeasonRow,
        r#"select s.id, s.number, s.name, s.overview, s.year,
                  (select count(*) from episodes e where e.season_id = s.id) as "episode_count!"
           from seasons s where s.title_id = $1 order by s.number"#,
        title_id
    )
    .fetch_all(db)
    .await
}

pub async fn title_languages(db: &PgPool, title_id: Uuid) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "select language from title_languages where title_id = $1 order by language",
        title_id
    )
    .fetch_all(db)
    .await
}

pub struct EpisodeRow {
    pub id: Uuid,
    pub season_id: Uuid,
    pub number: i16,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub air_date: Option<Date>,
    pub still_image_id: Option<Uuid>,
    /// Content languages with a ready variant.
    pub languages: Vec<String>,
}

/// H9. `None` when the title is not published or the season does not exist.
pub async fn published_season_episodes(
    db: &PgPool,
    title_id: Uuid,
    number: i16,
) -> Result<Option<Vec<EpisodeRow>>, sqlx::Error> {
    let season = sqlx::query_scalar!(
        "select s.id from seasons s join titles t on t.id = s.title_id
         where s.title_id = $1 and s.number = $2 and t.status = 'published'",
        title_id,
        number
    )
    .fetch_optional(db)
    .await?;
    let Some(season_id) = season else {
        return Ok(None);
    };
    let rows = sqlx::query_as!(
        EpisodeRow,
        r#"select e.id, e.season_id, e.number, e.name, e.overview, e.runtime_minutes, e.air_date, e.still_image_id,
                  array(select v.language from media_variants v
                        where v.episode_id = e.id and v.status = 'ready' order by v.language) as "languages!"
           from episodes e where e.season_id = $1 order by e.number"#,
        season_id
    )
    .fetch_all(db)
    .await?;
    Ok(Some(rows))
}

/// Every episode of a title, for the admin editor.
pub async fn all_episodes(db: &PgPool, title_id: Uuid) -> Result<Vec<EpisodeRow>, sqlx::Error> {
    sqlx::query_as!(
        EpisodeRow,
        r#"select e.id, e.season_id, e.number, e.name, e.overview, e.runtime_minutes, e.air_date, e.still_image_id,
                  array(select v.language from media_variants v
                        where v.episode_id = e.id and v.status = 'ready' order by v.language) as "languages!"
           from episodes e where e.title_id = $1 order by e.number"#,
        title_id
    )
    .fetch_all(db)
    .await
}

pub async fn genres(db: &PgPool) -> Result<Vec<GenreRow>, sqlx::Error> {
    sqlx::query_as!(GenreRow, "select id, slug, name from genres order by name")
        .fetch_all(db)
        .await
}

/// Content languages that at least one published title can be watched in.
pub async fn languages(db: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "select distinct tl.language from title_languages tl join titles t on t.id = tl.title_id
         where t.status = 'published' order by tl.language"
    )
    .fetch_all(db)
    .await
}

// ---------------------------------------------------------------- admin reads

pub struct AdminRow {
    pub id: Uuid,
    pub kind: String,
    pub name: String,
    pub year: Option<i16>,
    pub status: String,
    pub updated_at: OffsetDateTime,
}

/// All titles, drafts included, last changed first. `q` matches anywhere in the name, case and
/// accent insensitive. `strpos` instead of `like`, so `%` and `_` typed by an admin are plain
/// characters.
pub async fn admin_list(
    db: &PgPool,
    status: Option<&str>,
    q: Option<&str>,
    after: Option<(OffsetDateTime, Uuid)>,
    limit: i64,
) -> Result<Vec<AdminRow>, sqlx::Error> {
    let (after_at, after_id) = after.map_or((None, None), |(a, i)| (Some(a), Some(i)));
    sqlx::query_as!(
        AdminRow,
        "select id, kind, name, year, status, updated_at from titles
         where ($1::text is null or status = $1)
           and ($2::text is null or strpos(f_unaccent(lower(name)), f_unaccent(lower($2))) > 0)
           and ($3::timestamptz is null or (updated_at, id) < ($3, $4::uuid))
         order by updated_at desc, id desc
         limit $5",
        status,
        q,
        after_at,
        after_id,
        limit,
    )
    .fetch_all(db)
    .await
}

// ---------------------------------------------------------------- admin writes

pub struct NewTitle<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    pub original_name: Option<&'a str>,
    pub overview: Option<&'a str>,
    pub year: Option<i16>,
    pub runtime_minutes: Option<i32>,
    pub tmdb_id: Option<i32>,
}

pub async fn insert_title(db: &mut PgConnection, t: NewTitle<'_>) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar!(
        "insert into titles (kind, name, sort_name, original_name, overview, year, runtime_minutes, tmdb_id)
         values ($1, $2, lower(f_unaccent($2)), $3, $4, $5, $6, $7) returning id",
        t.kind,
        t.name,
        t.original_name,
        t.overview,
        t.year,
        t.runtime_minutes,
        t.tmdb_id,
    )
    .fetch_one(db)
    .await
}

pub struct TitleUpdate<'a> {
    pub name: Option<&'a str>,
    pub original_name: Option<Option<&'a str>>,
    pub overview: Option<Option<&'a str>>,
    pub year: Option<Option<i16>>,
    pub runtime_minutes: Option<Option<i32>>,
}

/// Applies a patch. Returns the kind, or `None` when the title does not exist.
pub async fn update_title(db: &mut PgConnection, id: Uuid, u: &TitleUpdate<'_>) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "update titles set
           name = coalesce($2, name),
           sort_name = lower(f_unaccent(coalesce($2, name))),
           original_name = case when $3 then $4 else original_name end,
           overview = case when $5 then $6 else overview end,
           year = case when $7 then $8 else year end,
           runtime_minutes = case when $9 then $10 else runtime_minutes end,
           updated_at = now()
         where id = $1 returning kind",
        id,
        u.name,
        u.original_name.is_some(),
        u.original_name.flatten(),
        u.overview.is_some(),
        u.overview.flatten(),
        u.year.is_some(),
        u.year.flatten(),
        u.runtime_minutes.is_some(),
        u.runtime_minutes.flatten(),
    )
    .fetch_optional(db)
    .await
}

pub async fn title_kind_for_update(db: &mut PgConnection, id: Uuid) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!("select kind from titles where id = $1 for update", id)
        .fetch_optional(db)
        .await
}

/// Sets or clears the poster and the backdrop. `None` leaves one unchanged.
pub async fn set_title_images(
    db: &mut PgConnection,
    id: Uuid,
    poster: Option<Option<Uuid>>,
    backdrop: Option<Option<Uuid>>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update titles set
           poster_image_id = case when $2 then $3 else poster_image_id end,
           backdrop_image_id = case when $4 then $5 else backdrop_image_id end
         where id = $1",
        id,
        poster.is_some(),
        poster.flatten(),
        backdrop.is_some(),
        backdrop.flatten(),
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn touch_title(db: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!("update titles set updated_at = now() where id = $1", id)
        .execute(db)
        .await?;
    Ok(())
}

/// Publishes. `listed_at` is set once, so republishing does not jump the title to the front.
pub async fn publish(db: &mut PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "update titles set status = 'published', listed_at = coalesce(listed_at, now()), updated_at = now()
         where id = $1",
        id
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn unpublish(db: &mut PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "update titles set status = 'draft', updated_at = now() where id = $1",
        id
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn delete_title(db: &mut PgConnection, id: Uuid) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!("delete from titles where id = $1", id).execute(db).await?;
    Ok(r.rows_affected() == 1)
}

/// How many of `ids` exist, to refuse unknown genre ids.
pub async fn count_genres(db: &mut PgConnection, ids: &[i16]) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(r#"select count(*) as "n!" from genres where id = any($1)"#, ids)
        .fetch_one(db)
        .await
}

pub async fn genre_by_tmdb(db: &mut PgConnection, tmdb_id: i32) -> Result<Option<i16>, sqlx::Error> {
    sqlx::query_scalar!("select id from genres where tmdb_id = $1", tmdb_id)
        .fetch_optional(db)
        .await
}

/// Finds a genre by slug, and links it to a TMDB id when it has none yet.
pub async fn genre_by_slug(
    db: &mut PgConnection,
    slug: &str,
    tmdb_id: Option<i32>,
) -> Result<Option<i16>, sqlx::Error> {
    sqlx::query_scalar!(
        "update genres set tmdb_id = coalesce(tmdb_id, $2) where slug = $1 returning id",
        slug,
        tmdb_id
    )
    .fetch_optional(db)
    .await
}

pub async fn insert_genre(
    db: &mut PgConnection,
    slug: &str,
    name: &str,
    tmdb_id: Option<i32>,
) -> Result<i16, sqlx::Error> {
    sqlx::query_scalar!(
        "insert into genres (slug, name, tmdb_id) values ($1, $2, $3) returning id",
        slug,
        name,
        tmdb_id
    )
    .fetch_one(db)
    .await
}

pub async fn set_title_genres(db: &mut PgConnection, title_id: Uuid, ids: &[i16]) -> Result<(), sqlx::Error> {
    sqlx::query!("delete from title_genres where title_id = $1", title_id)
        .execute(&mut *db)
        .await?;
    sqlx::query!(
        "insert into title_genres (title_id, genre_id) select $1, unnest($2::smallint[]) on conflict do nothing",
        title_id,
        ids
    )
    .execute(db)
    .await?;
    Ok(())
}

pub struct CreditBatch {
    pub ord: Vec<i16>,
    pub person: Vec<String>,
    pub character: Vec<Option<String>>,
    pub role: Vec<String>,
}

pub async fn set_title_credits(db: &mut PgConnection, title_id: Uuid, c: &CreditBatch) -> Result<(), sqlx::Error> {
    sqlx::query!("delete from title_credits where title_id = $1", title_id)
        .execute(&mut *db)
        .await?;
    sqlx::query!(
        "insert into title_credits (title_id, ord, person_name, character, role)
         select $1, * from unnest($2::smallint[], $3::text[], $4::text[], $5::text[])",
        title_id,
        &c.ord,
        &c.person,
        &c.character as &[Option<String>],
        &c.role,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub struct NewSeason<'a> {
    pub title_id: Uuid,
    pub number: i16,
    pub name: Option<&'a str>,
    pub overview: Option<&'a str>,
    pub year: Option<i16>,
}

pub async fn insert_season(db: &mut PgConnection, s: NewSeason<'_>) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar!(
        "insert into seasons (title_id, number, name, overview, year) values ($1, $2, $3, $4, $5) returning id",
        s.title_id,
        s.number,
        s.name,
        s.overview,
        s.year,
    )
    .fetch_one(db)
    .await
}

pub struct EpisodeBatch {
    pub number: Vec<i16>,
    pub name: Vec<Option<String>>,
    pub overview: Vec<Option<String>>,
    pub runtime: Vec<Option<i32>>,
    pub air_date: Vec<Option<Date>>,
}

/// Inserts a season's episodes in one statement and returns their ids in input order.
pub async fn insert_episodes(
    db: &mut PgConnection,
    season_id: Uuid,
    title_id: Uuid,
    e: &EpisodeBatch,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "insert into episodes (season_id, title_id, number, name, overview, runtime_minutes, air_date)
         select $1, $2, * from unnest($3::smallint[], $4::text[], $5::text[], $6::int[], $7::date[])
         returning id",
        season_id,
        title_id,
        &e.number,
        &e.name as &[Option<String>],
        &e.overview as &[Option<String>],
        &e.runtime as &[Option<i32>],
        &e.air_date as &[Option<Date>],
    )
    .fetch_all(db)
    .await
}

pub struct SeasonUpdate<'a> {
    pub number: Option<i16>,
    pub name: Option<Option<&'a str>>,
    pub overview: Option<Option<&'a str>>,
    pub year: Option<Option<i16>>,
}

/// Returns the title id, or `None` when the season does not exist.
pub async fn update_season(db: &mut PgConnection, id: Uuid, u: &SeasonUpdate<'_>) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "update seasons set
           number = coalesce($2, number),
           name = case when $3 then $4 else name end,
           overview = case when $5 then $6 else overview end,
           year = case when $7 then $8 else year end,
           updated_at = now()
         where id = $1 returning title_id",
        id,
        u.number,
        u.name.is_some(),
        u.name.flatten(),
        u.overview.is_some(),
        u.overview.flatten(),
        u.year.is_some(),
        u.year.flatten(),
    )
    .fetch_optional(db)
    .await
}

pub async fn delete_season(db: &mut PgConnection, id: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!("delete from seasons where id = $1 returning title_id", id)
        .fetch_optional(db)
        .await
}

pub async fn season_title(db: &mut PgConnection, id: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!("select title_id from seasons where id = $1", id)
        .fetch_optional(db)
        .await
}

pub struct EpisodeUpdate<'a> {
    pub number: Option<i16>,
    pub name: Option<Option<&'a str>>,
    pub overview: Option<Option<&'a str>>,
    pub runtime_minutes: Option<Option<i32>>,
    pub air_date: Option<Option<Date>>,
}

pub async fn update_episode(
    db: &mut PgConnection,
    id: Uuid,
    u: &EpisodeUpdate<'_>,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "update episodes set
           number = coalesce($2, number),
           name = case when $3 then $4 else name end,
           overview = case when $5 then $6 else overview end,
           runtime_minutes = case when $7 then $8 else runtime_minutes end,
           air_date = case when $9 then $10 else air_date end,
           updated_at = now()
         where id = $1 returning title_id",
        id,
        u.number,
        u.name.is_some(),
        u.name.flatten(),
        u.overview.is_some(),
        u.overview.flatten(),
        u.runtime_minutes.is_some(),
        u.runtime_minutes.flatten(),
        u.air_date.is_some(),
        u.air_date.flatten(),
    )
    .fetch_optional(db)
    .await
}

pub async fn delete_episode(db: &mut PgConnection, id: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!("delete from episodes where id = $1 returning title_id", id)
        .fetch_optional(db)
        .await
}
