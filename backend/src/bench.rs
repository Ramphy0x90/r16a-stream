//! `api seed --bench`, the hot query check of SPEC 4.10.
//!
//! Loads the scale of SPEC 1 (10 000 titles, 100 000 episodes) into the development database,
//! runs `EXPLAIN (ANALYZE, BUFFERS)` for the catalog queries, prints a Markdown report for
//! `docs/perf/queries.md`, and removes everything it inserted. Refused outside development and
//! test, it writes bulk data.
//!
//! The SQL below mirrors `catalog/repository.rs`. A change there must be repeated here, which
//! the report's query names make easy to spot.

use std::fmt::Write;

use sqlx::{PgPool, Row};

/// Marker on every seeded title, so cleanup removes exactly the seeded rows.
const MARK: &str = "bench-seed";

/// Inserts the data set in a handful of set-based statements.
pub async fn seed(db: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query(
        "insert into genres (slug, name)
         select 'bench-genre-' || g, 'Bench genre ' || g from generate_series(1, 20) g
         on conflict (slug) do nothing",
    )
    .execute(&mut *tx)
    .await?;
    // Words for search, so the text index has realistic content.
    sqlx::query(
        "insert into titles (kind, name, sort_name, original_name, overview, year, runtime_minutes, status, listed_at)
         select case when n % 2 = 0 then 'movie' else 'show' end,
                w.a || ' ' || w.b || ' ' || n,
                lower(w.a || ' ' || w.b || ' ' || n),
                'bench-seed',
                'A story about ' || w.b || ' and ' || w.a || ' in the city of ' || w.c || ', tag' || n || '.',
                1950 + n % 75,
                case when n % 2 = 0 then 90 + n % 60 end,
                case when n % 10 = 0 then 'draft' else 'published' end,
                case when n % 10 = 0 then null else now() - make_interval(mins => n) end
         from generate_series(1, 10000) n,
              lateral (select (array['Breaking','Silent','Golden','Last','Hidden','Broken','Northern','Wild'])[1 + n % 8] as a,
                              (array['River','Empire','Garden','Signal','Harbour','Winter','Machine','Crown'])[1 + (n / 8) % 8] as b,
                              (array['Rome','Lisbon','Oslo','Kyoto','Lima','Accra'])[1 + n % 6] as c) w",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "insert into title_genres (title_id, genre_id)
         select t.id, g.id from titles t
         join lateral (select id from genres where slug like 'bench-genre-%'
                       order by md5(t.id::text || id::text) limit 2) g on true
         where t.original_name = 'bench-seed'",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "insert into title_languages (title_id, language)
         select t.id, l from titles t, unnest(array['en','it','de']) l
         where t.original_name = 'bench-seed' and t.status = 'published' and (l = 'en' or md5(t.id::text || l) < '6')",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "insert into seasons (title_id, number)
         select t.id, s from titles t, generate_series(1, 4) s
         where t.original_name = 'bench-seed' and t.kind = 'show'",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "insert into episodes (season_id, title_id, number, name, runtime_minutes)
         select s.id, s.title_id, e, 'Episode ' || e, 45
         from seasons s join titles t on t.id = s.title_id, generate_series(1, 5) e
         where t.original_name = 'bench-seed'",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    // VACUUM, not only ANALYZE. A bulk insert leaves new GIN entries in the pending list, which
    // makes the planner avoid the search indexes until the list is merged. Production inserts
    // titles one at a time and autovacuum merges it, so the bench measures that state.
    for table in [
        "titles",
        "title_genres",
        "title_languages",
        "seasons",
        "episodes",
        "genres",
    ] {
        let sql = format!("vacuum (analyze) {table}");
        // The table names are the constants above.
        sqlx::query(sqlx::AssertSqlSafe(sql)).execute(db).await?;
    }
    Ok(())
}

pub async fn cleanup(db: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("delete from titles where original_name = $1")
        .bind(MARK)
        .execute(db)
        .await?;
    sqlx::query("delete from genres where slug like 'bench-genre-%'")
        .execute(db)
        .await?;
    Ok(())
}

struct Case {
    id: &'static str,
    what: &'static str,
    sql: &'static str,
    /// Bound in order. Text, the query casts.
    params: &'static [&'static str],
}

const CASES: &[Case] = &[
    Case {
        id: "H1",
        what: "home grid, recently added, first page",
        sql: "select t.id, t.kind, t.name, t.year, t.poster_image_id, t.listed_at from titles t
              where t.status = 'published' and ($1::text is null or t.kind = $1)
              order by t.listed_at desc, t.id desc limit 31",
        params: &[""],
    },
    Case {
        id: "H1",
        what: "home grid, shows only, page after a cursor deep in the list",
        sql: "select t.id, t.kind, t.name, t.year, t.poster_image_id from titles t
              where t.status = 'published' and t.kind = $1
                and (t.listed_at, t.id) < (now() - interval '4000 minutes', 'ffffffff-ffff-ffff-ffff-ffffffffffff'::uuid)
              order by t.listed_at desc, t.id desc limit 31",
        params: &["show"],
    },
    Case {
        id: "H2",
        what: "home grid with a genre",
        sql: "select t.id, t.name from titles t
              where t.status = 'published'
                and exists (select 1 from title_genres tg join genres g on g.id = tg.genre_id
                            where tg.title_id = t.id and g.slug = $1)
              order by t.listed_at desc, t.id desc limit 31",
        params: &["bench-genre-3"],
    },
    Case {
        id: "H3",
        what: "home grid with a content language",
        sql: "select t.id, t.name from titles t
              where t.status = 'published'
                and exists (select 1 from title_languages tl where tl.title_id = t.id and tl.language = $1)
              order by t.listed_at desc, t.id desc limit 31",
        params: &["it"],
    },
    Case {
        id: "H4",
        what: "A to Z, page after a cursor",
        sql: "select t.id, t.name from titles t
              where t.status = 'published' and (t.sort_name, t.id) > ($1, '00000000-0000-0000-0000-000000000000'::uuid)
              order by t.sort_name, t.id limit 31",
        params: &["hidden"],
    },
    Case {
        id: "H5",
        what: "search, a word in 12% of all titles",
        sql: "select t.id, (ts_rank(t.search_doc, websearch_to_tsquery('simple', f_unaccent($1)))
                      + similarity(f_unaccent(lower(t.name)), f_unaccent(lower($1))))::float4 as rank
              from titles t
              where t.status = 'published'
                and (t.search_doc @@ websearch_to_tsquery('simple', f_unaccent($1))
                     or f_unaccent(lower(t.name)) % f_unaccent(lower($1)))
              order by rank desc, t.id desc limit 31",
        params: &["harbour"],
    },
    Case {
        id: "H5",
        what: "search, a rare word, as most real searches are",
        sql: "select t.id, (ts_rank(t.search_doc, websearch_to_tsquery('simple', f_unaccent($1)))
                      + similarity(f_unaccent(lower(t.name)), f_unaccent(lower($1))))::float4 as rank
              from titles t
              where t.status = 'published'
                and (t.search_doc @@ websearch_to_tsquery('simple', f_unaccent($1))
                     or f_unaccent(lower(t.name)) % f_unaccent(lower($1)))
              order by rank desc, t.id desc limit 31",
        params: &["tag4243"],
    },
    Case {
        id: "H5",
        what: "search, a typo",
        sql: "select t.id, (ts_rank(t.search_doc, websearch_to_tsquery('simple', f_unaccent($1)))
                      + similarity(f_unaccent(lower(t.name)), f_unaccent(lower($1))))::float4 as rank
              from titles t
              where t.status = 'published'
                and (t.search_doc @@ websearch_to_tsquery('simple', f_unaccent($1))
                     or f_unaccent(lower(t.name)) % f_unaccent(lower($1)))
              order by rank desc, t.id desc limit 31",
        params: &["brakng rivr"],
    },
    Case {
        id: "H8",
        what: "title page, seasons with episode counts",
        sql: "select s.id, s.number, (select count(*) from episodes e where e.season_id = s.id)
              from seasons s
              where s.title_id = (select id from titles where original_name = $1 and kind = 'show' limit 1)
              order by s.number",
        params: &[MARK],
    },
    Case {
        id: "H9",
        what: "episodes of one season",
        sql: "select e.id, e.number, e.name from episodes e
              where e.season_id = (select s.id from seasons s join titles t on t.id = s.title_id
                                   where t.original_name = $1 and s.number = 2 limit 1)
              order by e.number",
        params: &[MARK],
    },
];

/// Runs every case a few times and returns the Markdown report.
pub async fn report(db: &PgPool) -> Result<String, sqlx::Error> {
    let mut out = String::from("| # | Query | Median ms | Max ms | Plan |\n|---|---|---|---|---|\n");
    let mut plans = String::new();
    for c in CASES {
        let mut times = Vec::new();
        let mut plan = Vec::new();
        for _ in 0..7 {
            let sql = format!("explain (analyze, buffers, format text) {}", c.sql);
            // The statement text is built from the constants above only, never from input.
            let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
            for p in c.params {
                q = q.bind(if p.is_empty() { None } else { Some(*p) });
            }
            let rows = q.fetch_all(db).await?;
            plan = rows.iter().map(|r| r.get::<String, _>(0)).collect::<Vec<_>>();
            let exec = plan
                .iter()
                .find_map(|l| l.trim().strip_prefix("Execution Time: "))
                .and_then(|v| v.trim_end_matches(" ms").parse::<f64>().ok())
                .unwrap_or(f64::NAN);
            times.push(exec);
        }
        times.sort_by(f64::total_cmp);
        let first = plan
            .first()
            .map(|l| l.trim().split("  (").next().unwrap_or_default().to_owned());
        let _ = writeln!(
            out,
            "| {} | {} | {:.2} | {:.2} | {} |",
            c.id,
            c.what,
            times[times.len() / 2],
            times[times.len() - 1],
            first.unwrap_or_default().replace('|', "/")
        );
        let _ = write!(plans, "\n### {} {}\n\n```\n{}\n```\n", c.id, c.what, plan.join("\n"));
    }
    Ok(format!("{out}\n## Plans\n{plans}"))
}
