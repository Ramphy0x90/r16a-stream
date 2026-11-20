//! A mock TMDB API for the lookup tests. Serves one movie and one show with two seasons, counts
//! requests, and records the Authorization header it received.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::json;

#[derive(Default)]
struct Inner {
    requests: AtomicUsize,
    auth: Mutex<Option<String>>,
}

#[derive(Clone)]
pub struct MockTmdb {
    pub base: String,
    inner: Arc<Inner>,
}

impl MockTmdb {
    pub async fn start() -> Self {
        let inner = Arc::new(Inner::default());
        let app = Router::new()
            .route("/3/movie/{id}", get(movie))
            .route("/3/tv/{id}", get(tv))
            .route("/3/tv/{id}/season/{n}", get(season))
            .with_state(inner.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}/3/", listener.local_addr().expect("addr"));
        tokio::spawn(async move { axum::serve(listener, app).await.expect("mock tmdb") });
        Self { base, inner }
    }

    pub fn requests(&self) -> usize {
        self.inner.requests.load(Ordering::SeqCst)
    }

    pub fn last_auth(&self) -> Option<String> {
        self.inner.auth.lock().unwrap().clone()
    }
}

fn note(s: &Inner, h: &HeaderMap) {
    s.requests.fetch_add(1, Ordering::SeqCst);
    *s.auth.lock().unwrap() = h.get("authorization").and_then(|v| v.to_str().ok()).map(String::from);
}

async fn movie(State(s): State<Arc<Inner>>, h: HeaderMap, Path(id): Path<String>) -> Response {
    note(&s, &h);
    if id != "949" {
        return (StatusCode::NOT_FOUND, Json(json!({"status_code": 34}))).into_response();
    }
    Json(json!({
        "id": 949, "title": "Heat", "original_title": "Heat",
        "overview": "Obsessive master thief\u{0007} and a detective.", "release_date": "1995-12-15",
        "runtime": 170, "genres": [{"id": 28, "name": "Action"}, {"id": 80, "name": "Crime"}],
        "credits": {
            "crew": [{"name": "Michael Mann", "job": "Director"}, {"name": "Someone", "job": "Grip"}],
            "cast": [{"name": "Al Pacino", "character": "Vincent Hanna"}, {"name": "Robert De Niro", "character": "Neil McCauley"}]
        }
    }))
    .into_response()
}

async fn tv(State(s): State<Arc<Inner>>, h: HeaderMap, Path(id): Path<String>) -> Response {
    note(&s, &h);
    if id != "1396" {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(json!({
        "id": 1396, "name": "Breaking Bad", "original_name": "Breaking Bad", "overview": "A teacher turns.",
        "first_air_date": "2008-01-20", "genres": [{"id": 18, "name": "Drama"}],
        "created_by": [{"name": "Vince Gilligan"}],
        "credits": {"crew": [], "cast": [{"name": "Bryan Cranston", "character": "Walter White"}]},
        "seasons": [{"season_number": 0}, {"season_number": 1}]
    }))
    .into_response()
}

async fn season(State(s): State<Arc<Inner>>, h: HeaderMap, Path((_id, n)): Path<(String, i32)>) -> Response {
    note(&s, &h);
    Json(json!({
        "season_number": n, "name": format!("Season {n}"), "air_date": "2008-01-20",
        "episodes": [
            {"episode_number": 1, "name": "Pilot", "runtime": 58, "air_date": "2008-01-20"},
            {"episode_number": 2, "name": "Cat's in the Bag...", "runtime": 48, "air_date": "2008-01-27"},
            {"episode_number": 2, "name": "duplicate number from a broken source"}
        ]
    }))
    .into_response()
}
