//! Catalog browsing, search, admin editing and the TMDB lookup (SPEC 4.4, 5, 8.1).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::net::SocketAddr;

use axum::{
    Extension,
    body::Body,
    extract::ConnectInfo,
    http::{Method, Request, header},
    response::Response,
};
use common::{idp::MockIdp, tmdb::MockTmdb};
use r16a_stream_backend::{app, state::AppState};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

struct Api {
    state: AppState,
    member: String,
    admin: String,
    tmdb: MockTmdb,
}

impl Api {
    async fn new(pool: PgPool) -> Self {
        let idp = MockIdp::start().await;
        let tmdb = MockTmdb::start().await;
        let mut vars = idp.vars("http://localhost:4200");
        vars.insert("R16A_TMDB_BASE_URL", tmdb.base.clone());
        let state = common::state_with(pool, &vars);
        let api = Self {
            state,
            member: idp.mobile_token("member", true, false),
            admin: idp.mobile_token("admin", true, true),
            tmdb,
        };
        // Provision both users, the audit log references the admin.
        api.call(Method::GET, "/api/v1/me", &api.admin.clone(), None).await;
        api
    }

    async fn call(&self, method: Method, uri: &str, token: &str, body: Option<Value>) -> Response {
        let mut b = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"));
        let body = match body {
            Some(v) => {
                b = b.header(header::CONTENT_TYPE, "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        app::router(self.state.clone())
            .layer(Extension(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 1)))))
            .oneshot(b.body(body).unwrap())
            .await
            .unwrap()
    }

    async fn json(&self, method: Method, uri: &str, token: &str, body: Option<Value>) -> (u16, Value) {
        let res = self.call(method, uri, token, body).await;
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), 4 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn create(&self, body: Value) -> String {
        let (status, v) = self
            .json(Method::POST, "/api/v1/admin/titles", &self.admin.clone(), Some(body))
            .await;
        assert_eq!(status, 201, "{v}");
        v["id"].as_str().unwrap().to_owned()
    }

    async fn publish(&self, id: &str) {
        let res = self
            .call(
                Method::POST,
                &format!("/api/v1/admin/titles/{id}/publish"),
                &self.admin,
                None,
            )
            .await;
        assert_eq!(res.status(), 204);
    }

    async fn names(&self, uri: &str) -> (Vec<String>, Option<String>) {
        let (status, v) = self.json(Method::GET, uri, &self.member.clone(), None).await;
        assert_eq!(status, 200, "{uri} {v}");
        let names = v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["name"].as_str().unwrap().to_owned())
            .collect();
        (names, v["nextCursor"].as_str().map(String::from))
    }
}

#[sqlx::test]
async fn recent_pages_walk_every_published_title_once(pool: PgPool) {
    let api = Api::new(pool).await;
    for i in 0..5 {
        let id = api.create(json!({"kind": "movie", "name": format!("Movie {i}")})).await;
        api.publish(&id).await;
    }
    api.create(json!({"kind": "movie", "name": "Unpublished draft"})).await;

    let mut seen = Vec::new();
    let mut uri = String::from("/api/v1/titles?limit=2");
    loop {
        let (names, next) = api.names(&uri).await;
        seen.extend(names);
        let Some(c) = next else { break };
        uri = format!("/api/v1/titles?limit=2&cursor={c}");
    }
    assert_eq!(seen, ["Movie 4", "Movie 3", "Movie 2", "Movie 1", "Movie 0"]);
}

#[sqlx::test]
async fn filters_sort_and_search(pool: PgPool) {
    let api = Api::new(pool).await;
    let titles = [
        json!({"kind": "show", "name": "Breaking Bad", "newGenres": [{"name": "Drama"}]}),
        json!({"kind": "movie", "name": "Café Society", "newGenres": [{"name": "Comedy"}]}),
        json!({"kind": "movie", "name": "Amélie", "overview": "A shy waitress in Montmartre.", "newGenres": [{"name": "Comedy"}]}),
    ];
    for t in titles {
        let id = api.create(t).await;
        api.publish(&id).await;
    }
    assert_eq!(
        api.names("/api/v1/titles?sort=az").await.0,
        ["Amélie", "Breaking Bad", "Café Society"]
    );
    assert_eq!(api.names("/api/v1/titles?type=show").await.0, ["Breaking Bad"]);
    assert_eq!(
        api.names("/api/v1/titles?genre=comedy&sort=az").await.0,
        ["Amélie", "Café Society"]
    );
    assert_eq!(
        api.names("/api/v1/titles?q=cafe").await.0,
        ["Café Society"],
        "accent insensitive"
    );
    assert_eq!(
        api.names("/api/v1/titles?q=brakng%20bad").await.0,
        ["Breaking Bad"],
        "typos via trigrams"
    );
    assert_eq!(
        api.names("/api/v1/titles?q=montmartre").await.0,
        ["Amélie"],
        "overview words"
    );
    assert!(
        api.names("/api/v1/titles?language=it").await.0.is_empty(),
        "no ready variants yet"
    );

    for bad in [
        "/api/v1/titles?type=episode",
        "/api/v1/titles?sort=random",
        "/api/v1/titles?limit=61",
        "/api/v1/titles?cursor=not-base64!",
        "/api/v1/titles?language=English",
    ] {
        let (status, _) = api.json(Method::GET, bad, &api.member.clone(), None).await;
        assert_eq!(status, 400, "{bad}");
    }
}

#[sqlx::test]
async fn catalog_reads_revalidate_with_etags(pool: PgPool) {
    let api = Api::new(pool).await;
    let id = api.create(json!({"kind": "movie", "name": "Heat"})).await;
    api.publish(&id).await;
    let uri = format!("/api/v1/titles/{id}");
    let first = api.call(Method::GET, &uri, &api.member, None).await;
    assert_eq!(first.headers()[header::CACHE_CONTROL], "private, no-cache");
    let tag = first.headers()[header::ETAG].to_str().unwrap().to_owned();
    let again = app::router(api.state.clone())
        .oneshot(
            Request::get(&uri)
                .header(header::AUTHORIZATION, format!("Bearer {}", api.member))
                .header(header::IF_NONE_MATCH, &tag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(again.status(), 304);
    api.call(
        Method::PATCH,
        &format!("/api/v1/admin/titles/{id}"),
        &api.admin,
        Some(json!({"year": 1995})),
    )
    .await;
    let changed = api.call(Method::GET, &uri, &api.member, None).await;
    assert_ne!(changed.headers()[header::ETAG].to_str().unwrap(), tag);
}

#[sqlx::test]
async fn admin_creates_a_show_with_episodes_and_edits_it(pool: PgPool) {
    let api = Api::new(pool).await;
    let id = api
        .create(json!({
            "kind": "show", "name": "The Wire", "year": 2002,
            "credits": [{"personName": "David Simon", "role": "creator"}, {"personName": "Dominic West", "character": "McNulty", "role": "cast"}],
            "seasons": [{"number": 1, "episodes": [{"number": 1, "name": "The Target", "airDate": "2002-06-02"}, {"number": 2}]}]
        }))
        .await;
    let (_, t) = api
        .json(
            Method::GET,
            &format!("/api/v1/admin/titles/{id}"),
            &api.admin.clone(),
            None,
        )
        .await;
    assert_eq!(t["status"], "draft");
    assert_eq!(t["seasons"][0]["episodes"].as_array().unwrap().len(), 2);
    assert_eq!(t["seasons"][0]["episodes"][0]["airDate"], "2002-06-02");
    assert_eq!(t["credits"][0]["role"], "creator");

    // Null clears, missing keeps.
    let res = api
        .call(
            Method::PATCH,
            &format!("/api/v1/admin/titles/{id}"),
            &api.admin,
            Some(json!({"year": null, "overview": "Baltimore."})),
        )
        .await;
    assert_eq!(res.status(), 204);
    let (_, t) = api
        .json(
            Method::GET,
            &format!("/api/v1/admin/titles/{id}"),
            &api.admin.clone(),
            None,
        )
        .await;
    assert_eq!(t["year"], Value::Null);
    assert_eq!(t["overview"], "Baltimore.");
    assert_eq!(t["name"], "The Wire");

    let season = t["seasons"][0]["id"].as_str().unwrap().to_owned();
    let dup = api
        .call(
            Method::POST,
            &format!("/api/v1/admin/seasons/{season}/episodes"),
            &api.admin,
            Some(json!({"number": 2})),
        )
        .await;
    assert_eq!(dup.status(), 409);
    let dup = api
        .call(
            Method::POST,
            &format!("/api/v1/admin/titles/{id}/seasons"),
            &api.admin,
            Some(json!({"number": 1})),
        )
        .await;
    assert_eq!(dup.status(), 409);

    let actions: Vec<String> = sqlx::query_scalar("select action from audit_log order by id")
        .fetch_all(&api.state.db)
        .await
        .unwrap();
    assert_eq!(actions, ["title.create", "title.update"]);
}

#[sqlx::test]
async fn movies_and_shows_keep_their_shape(pool: PgPool) {
    let api = Api::new(pool).await;
    let movie = api
        .create(json!({"kind": "movie", "name": "Heat", "runtimeMinutes": 170}))
        .await;
    let res = api
        .call(
            Method::POST,
            &format!("/api/v1/admin/titles/{movie}/seasons"),
            &api.admin,
            Some(json!({"number": 1})),
        )
        .await;
    assert_eq!(res.status(), 409, "a movie has no seasons");
    let (status, _) = api
        .json(
            Method::POST,
            "/api/v1/admin/titles",
            &api.admin.clone(),
            Some(json!({"kind": "show", "name": "X", "runtimeMinutes": 30})),
        )
        .await;
    assert_eq!(status, 400);
    let (status, _) = api
        .json(
            Method::POST,
            "/api/v1/admin/titles",
            &api.admin.clone(),
            Some(json!({"kind": "movie", "name": "X", "kind2": 1})),
        )
        .await;
    assert_eq!(status, 400, "unknown fields");
    let (status, _) = api
        .json(
            Method::POST,
            "/api/v1/admin/titles",
            &api.admin.clone(),
            Some(json!({"kind": "movie", "name": "Y", "genreIds": [999]})),
        )
        .await;
    assert_eq!(status, 400, "unknown genre");
}

#[sqlx::test]
async fn republishing_keeps_the_place_in_recently_added(pool: PgPool) {
    let api = Api::new(pool).await;
    let a = api.create(json!({"kind": "movie", "name": "First"})).await;
    api.publish(&a).await;
    let b = api.create(json!({"kind": "movie", "name": "Second"})).await;
    api.publish(&b).await;
    api.call(
        Method::POST,
        &format!("/api/v1/admin/titles/{a}/unpublish"),
        &api.admin,
        None,
    )
    .await;
    api.publish(&a).await;
    assert_eq!(api.names("/api/v1/home/recently-added").await.0, ["Second", "First"]);
}

#[sqlx::test]
async fn deleting_a_title_removes_its_seasons_and_episodes(pool: PgPool) {
    let api = Api::new(pool).await;
    let id = api
        .create(json!({"kind": "show", "name": "Gone", "seasons": [{"number": 1, "episodes": [{"number": 1}]}]}))
        .await;
    assert_eq!(
        api.call(Method::DELETE, &format!("/api/v1/admin/titles/{id}"), &api.admin, None)
            .await
            .status(),
        204
    );
    let left: i64 = sqlx::query_scalar("select count(*) from episodes")
        .fetch_one(&api.state.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
    assert_eq!(
        api.call(Method::DELETE, &format!("/api/v1/admin/titles/{id}"), &api.admin, None)
            .await
            .status(),
        404
    );
}

#[sqlx::test]
async fn bodies_above_the_route_limit_are_refused(pool: PgPool) {
    let api = Api::new(pool).await;
    let big = "x".repeat(20 * 1024);
    let id = api.create(json!({"kind": "movie", "name": "Heat"})).await;
    let res = api
        .call(
            Method::PATCH,
            &format!("/api/v1/admin/titles/{id}"),
            &api.admin,
            Some(json!({"overview": big})),
        )
        .await;
    assert_eq!(res.status(), 413, "16 KiB default");
    // A large show is fine on the create route, up to 2 MiB.
    let episodes: Vec<Value> = (1..=400)
        .map(|n| json!({"number": n, "overview": "y".repeat(500)}))
        .collect();
    let (status, _) = api
        .json(
            Method::POST,
            "/api/v1/admin/titles",
            &api.admin.clone(),
            Some(json!({"kind": "show", "name": "Long", "seasons": [{"number": 1, "episodes": episodes}]})),
        )
        .await;
    assert_eq!(status, 201);
}

#[sqlx::test]
async fn tmdb_lookup_builds_a_clean_draft_and_caches(pool: PgPool) {
    let api = Api::new(pool).await;
    let (status, d) = api
        .json(
            Method::GET,
            "/api/v1/admin/tmdb/lookup?ref=https%3A%2F%2Fwww.themoviedb.org%2Fmovie%2F949-heat",
            &api.admin.clone(),
            None,
        )
        .await;
    assert_eq!(status, 200, "{d}");
    assert_eq!(d["kind"], "movie");
    assert_eq!(d["name"], "Heat");
    assert_eq!(d["originalName"], Value::Null, "same as the name");
    assert_eq!(d["year"], 1995);
    assert_eq!(d["runtimeMinutes"], 170);
    assert_eq!(
        d["overview"], "Obsessive master thief and a detective.",
        "control characters stripped"
    );
    assert_eq!(d["newGenres"][0], json!({"tmdbId": 28, "name": "Action"}));
    assert_eq!(
        d["credits"][0],
        json!({"personName": "Michael Mann", "character": null, "role": "director"})
    );
    assert!(api.tmdb.last_auth().unwrap().starts_with("Bearer test-tmdb-read-token"));

    let before = api.tmdb.requests();
    api.json(
        Method::GET,
        "/api/v1/admin/tmdb/lookup?ref=949&kind=movie",
        &api.admin.clone(),
        None,
    )
    .await;
    assert_eq!(api.tmdb.requests(), before, "served from the cache");

    // The draft posts back unchanged, genres are created on the way.
    let (status, _) = api
        .json(Method::POST, "/api/v1/admin/titles", &api.admin.clone(), Some(d))
        .await;
    assert_eq!(status, 201);
    let genre: (String, Option<i32>) = sqlx::query_as("select slug, tmdb_id from genres where name = 'Action'")
        .fetch_one(&api.state.db)
        .await
        .unwrap();
    assert_eq!(genre, ("action".into(), Some(28)));
}

#[sqlx::test]
async fn tmdb_show_lookup_fetches_each_season(pool: PgPool) {
    let api = Api::new(pool).await;
    let (status, d) = api
        .json(
            Method::GET,
            "/api/v1/admin/tmdb/lookup?ref=1396&kind=show",
            &api.admin.clone(),
            None,
        )
        .await;
    assert_eq!(status, 200, "{d}");
    assert_eq!(d["kind"], "show");
    assert_eq!(d["runtimeMinutes"], Value::Null);
    assert_eq!(d["credits"][0]["role"], "creator");
    let seasons = d["seasons"].as_array().unwrap();
    assert_eq!(seasons.len(), 2, "specials and season 1");
    assert_eq!(
        seasons[1]["episodes"].as_array().unwrap().len(),
        2,
        "duplicate episode numbers dropped"
    );
    let (status, _) = api
        .json(Method::POST, "/api/v1/admin/titles", &api.admin.clone(), Some(d))
        .await;
    assert_eq!(status, 201, "the draft is valid as it comes");
}

#[sqlx::test]
async fn tmdb_lookup_refuses_foreign_links_and_unknown_ids(pool: PgPool) {
    let api = Api::new(pool).await;
    for bad in [
        "/api/v1/admin/tmdb/lookup?ref=https%3A%2F%2Fevil.example%2Fmovie%2F949",
        "/api/v1/admin/tmdb/lookup?ref=949",
        "/api/v1/admin/tmdb/lookup?ref=..%2F..%2Fsecret&kind=movie",
    ] {
        let (status, _) = api.json(Method::GET, bad, &api.admin.clone(), None).await;
        assert_eq!(status, 400, "{bad}");
    }
    assert_eq!(api.tmdb.requests(), 0, "nothing reached TMDB");
    let (status, _) = api
        .json(
            Method::GET,
            "/api/v1/admin/tmdb/lookup?ref=1&kind=movie",
            &api.admin.clone(),
            None,
        )
        .await;
    assert_eq!(status, 404);
}
