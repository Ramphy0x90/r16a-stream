//! The image pipeline end to end (SPEC 8.2, 8.3) against real Garage and real ffmpeg.
//!
//! The API side runs as in production, the worker side connects as `r16a_stream_worker` with
//! the worker Garage key, so the grants of SPEC 4.9 and the key scoping are exercised too.
//! Needs `R16A_TEST_S3_ENDPOINT`, `R16A_TEST_WORKER_DATABASE_URL` and ffmpeg in `/usr/bin`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
    Extension, Router,
    body::Body,
    extract::{ConnectInfo, Path, State},
    http::{Method, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use common::idp::MockIdp;
use r16a_stream_backend::{
    app,
    images::worker::{ImageWorker, scaled_height},
    state::AppState,
    storage::{BucketName, GarageClient},
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;

/// File name to content type and bytes.
type Files = HashMap<&'static str, (&'static str, Vec<u8>)>;

/// Serves fixed files under `/t/p/original/`, like `image.tmdb.org`.
async fn image_server(files: Files) -> String {
    let files = Arc::new(files);
    let app = Router::new()
        .route(
            "/t/p/original/{file}",
            get(|State(f): State<Arc<Files>>, Path(file): Path<String>| async move {
                match f.get(file.as_str()) {
                    Some((ct, body)) => ([(header::CONTENT_TYPE, *ct)], body.clone()).into_response(),
                    None => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        )
        .with_state(files);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/t/p/original/", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

/// Makes a test image with ffmpeg's colour source, outside the sandboxed code path.
fn make_image(dir: &std::path::Path, name: &str, size: &str, codec_args: &[&str]) -> Vec<u8> {
    let out = dir.join(name);
    let status = std::process::Command::new("/usr/bin/ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=0x10b981:s={size}"),
            "-frames:v",
            "1",
        ])
        .args(codec_args)
        .arg("-y")
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::read(out).unwrap()
}

struct Env {
    state: AppState,
    admin: String,
    worker: ImageWorker,
    storage_worker: Arc<GarageClient>,
    _dir: tempdir::Dir,
}

mod tempdir {
    /// A scratch directory removed on drop, without a crate for it.
    pub struct Dir(pub std::path::PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

async fn setup(pool: PgPool, files: Files) -> Option<Env> {
    let s3 = common::integration_env("R16A_TEST_S3_ENDPOINT")?;
    let worker_url = common::integration_env("R16A_TEST_WORKER_DATABASE_URL")?;
    if !std::path::Path::new("/usr/bin/ffmpeg").exists() {
        assert!(
            std::env::var("R16A_REQUIRE_INTEGRATION").as_deref() != Ok("1"),
            "ffmpeg is required"
        );
        return None;
    }
    let idp = MockIdp::start().await;
    let base = image_server(files).await;
    let mut vars = idp.vars("http://localhost:4200");
    vars.insert("R16A_S3_INTERNAL_ENDPOINTS", s3.clone());
    vars.insert("R16A_TMDB_IMAGE_BASE_URL", base);
    let state = common::state_with(pool.clone(), &vars);
    state.write_health.record(true);

    // The worker on the same test database, as the worker role.
    let db_name = pool.connect_options().get_database().unwrap().to_owned();
    let mut url = url::Url::parse(&worker_url).unwrap();
    url.set_path(&db_name);
    let worker_pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(url.as_str())
        .await
        .unwrap();
    let mut wvars = vars.clone();
    wvars.insert("R16A_S3_ACCESS_KEY_ID", common::WORKER_KEY_ID.into());
    wvars.insert("R16A_S3_SECRET_ACCESS_KEY", common::WORKER_SECRET.into());
    let storage_worker = Arc::new(GarageClient::new(&common::config_from(&wvars).s3).unwrap());
    let dir = std::env::temp_dir().join(format!("r16a-images-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let worker = ImageWorker {
        db: worker_pool,
        storage: storage_worker.clone(),
        ffmpeg: PathBuf::from("/usr/bin/ffmpeg"),
        ffprobe: PathBuf::from("/usr/bin/ffprobe"),
        scratch_root: dir.clone(),
        instance: "test-worker".into(),
    };
    let admin = idp.mobile_token("admin", true, true);
    let env = Env {
        state,
        admin,
        worker,
        storage_worker,
        _dir: tempdir::Dir(dir),
    };
    env.call(Method::GET, "/api/v1/me", None).await;
    Some(env)
}

impl Env {
    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut b = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.admin));
        let body = match body {
            Some(v) => {
                b = b.header(header::CONTENT_TYPE, "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let res: Response = app::router(self.state.clone())
            .layer(Extension(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 1)))))
            .oneshot(b.body(body).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn create_with_poster(&self, path: &str) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            "/api/v1/admin/titles",
            Some(json!({"kind": "movie", "name": format!("Poster {path}"), "posterPath": path})),
        )
        .await
    }

    async fn job(&self) -> (String, i16, i32, Option<String>) {
        sqlx::query_as("select status, attempts, storage_waits, failure_code from jobs")
            .fetch_one(&self.state.db)
            .await
            .unwrap()
    }
}

#[sqlx::test]
async fn a_tmdb_poster_becomes_presigned_avif_and_webp(pool: PgPool) {
    let tmp = std::env::temp_dir();
    let jpeg = make_image(&tmp, "r16a-poster.jpg", "600x900", &["-q:v", "3"]);
    let Some(env) = setup(pool, HashMap::from([("poster.jpg", ("image/jpeg", jpeg))])).await else {
        return;
    };
    let (status, created) = env.create_with_poster("/poster.jpg").await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();
    let (image_id, image_status): (uuid::Uuid, String) = sqlx::query_as("select id, status from images")
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert_eq!(image_status, "pending");

    assert!(env.worker.run_once().await.unwrap(), "the worker claims the job");
    assert!(!env.worker.run_once().await.unwrap(), "nothing left");
    assert_eq!(env.job().await.0, "succeeded");

    let (w, h, placeholder): (i32, i32, String) =
        sqlx::query_as("select source_width, source_height, placeholder_hsl from images")
            .fetch_one(&env.state.db)
            .await
            .unwrap();
    assert_eq!((w, h), (600, 900));
    let hue: i32 = placeholder.split(' ').next().unwrap().parse().unwrap();
    assert!((155..=165).contains(&hue), "emerald, got {placeholder}");

    let variants: Vec<(String, i16, i16, String)> =
        sqlx::query_as("select format, width, height, object_key from image_variants order by format, width")
            .fetch_all(&env.state.db)
            .await
            .unwrap();
    assert_eq!(
        variants.len(),
        6,
        "185, 342 and 500 in two formats, never wider than 600"
    );
    for (format, width, height, key) in &variants {
        let (bytes, cache) = env.storage_worker.get_object(BucketName::Media, key).await.unwrap();
        assert_ne!(bytes.len(), 0);
        assert_eq!(cache.as_deref(), Some("public, max-age=31536000, immutable"));
        assert_eq!(*height, scaled_height(600, 900, u32::try_from(*width).unwrap()));
        if format == "webp" {
            // The stored height is what ffmpeg really produced.
            let p = std::env::temp_dir().join(format!("r16a-{image_id}-{width}.webp"));
            std::fs::write(&p, &bytes).unwrap();
            let out = std::process::Command::new("/usr/bin/ffprobe")
                .args(["-v", "error", "-show_entries", "stream=height", "-of", "csv=p=0"])
                .arg(&p)
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), height.to_string());
        }
    }
    // Metadata such as EXIF is not copied, and nothing else was written.
    let (status, page) = env.call(Method::GET, &format!("/api/v1/admin/titles/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let sources = page["poster"]["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 6);
    let url = sources[0]["url"].as_str().unwrap();
    assert!(url.starts_with("http://localhost:4200/r16a-stream-media/i/"), "{url}");
    assert!(url.contains("X-Amz-Expires=172800"));
    assert_eq!(page["poster"]["placeholderHsl"], placeholder);
}

#[sqlx::test]
async fn files_that_are_not_images_fail_once_without_retries(pool: PgPool) {
    // A playlist that points at a local file. The format whitelist must refuse it before any
    // reference is followed.
    let playlist = b"#EXTM3U\n#EXTINF:1,\nfile:///etc/passwd\n".to_vec();
    let Some(env) = setup(pool, HashMap::from([("evil.jpg", ("image/jpeg", playlist))])).await else {
        return;
    };
    let (status, _) = env.create_with_poster("/evil.jpg").await;
    assert_eq!(status, StatusCode::CREATED);
    env.worker.run_once().await.unwrap();
    let (job_status, attempts, _, code) = env.job().await;
    assert_eq!(
        (job_status.as_str(), attempts, code.as_deref()),
        ("failed", 1, Some("not-an-image"))
    );
    let image: String = sqlx::query_scalar("select status from images")
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert_eq!(image, "failed");
    let detail: String = sqlx::query_scalar("select failure_detail from jobs")
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert!(!detail.contains('/'), "no paths reach the admin: {detail}");
}

#[sqlx::test]
async fn oversized_images_are_refused_before_decoding(pool: PgPool) {
    let tmp = std::env::temp_dir();
    let wide = make_image(&tmp, "r16a-wide.png", "16400x16", &[]);
    let Some(env) = setup(pool, HashMap::from([("wide.png", ("image/png", wide))])).await else {
        return;
    };
    env.create_with_poster("/wide.png").await;
    env.worker.run_once().await.unwrap();
    let (job_status, _, _, code) = env.job().await;
    assert_eq!((job_status.as_str(), code.as_deref()), ("failed", Some("too-large")));
}

#[sqlx::test]
async fn an_unreachable_garage_requeues_without_counting_a_failure(pool: PgPool) {
    let tmp = std::env::temp_dir();
    let jpeg = make_image(&tmp, "r16a-down.jpg", "200x300", &[]);
    let Some(mut env) = setup(pool, HashMap::from([("p.jpg", ("image/jpeg", jpeg))])).await else {
        return;
    };
    env.create_with_poster("/p.jpg").await;
    let mut vars = common::test_vars("http://localhost:4200", "http://127.0.0.1:9");
    vars.insert("R16A_S3_ACCESS_KEY_ID", common::WORKER_KEY_ID.into());
    vars.insert("R16A_S3_SECRET_ACCESS_KEY", common::WORKER_SECRET.into());
    env.worker.storage = Arc::new(GarageClient::new(&common::config_from(&vars).s3).unwrap());
    env.worker.run_once().await.unwrap();
    let (job_status, attempts, waits, _) = env.job().await;
    assert_eq!((job_status.as_str(), attempts, waits), ("queued", 0, 1));
    let blocked: Option<String> = sqlx::query_scalar("select blocked_reason from jobs")
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert_eq!(blocked.as_deref(), Some("storage_degraded"));
}

#[sqlx::test]
async fn the_api_refuses_unsafe_image_downloads(pool: PgPool) {
    let big = vec![0u8; 21 * 1024 * 1024];
    let Some(env) = setup(
        pool,
        HashMap::from([
            ("page.jpg", ("text/html", b"<html>".to_vec())),
            ("big.jpg", ("image/jpeg", big)),
        ]),
    )
    .await
    else {
        return;
    };
    for (path, why) in [
        ("/page.jpg", "content type"),
        ("/big.jpg", "20 MiB limit"),
        ("/missing.jpg", "TMDB 404"),
    ] {
        let (status, _) = env.create_with_poster(path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}");
    }
    let (status, _) = env.create_with_poster("/../secret.jpg").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "only plain TMDB file paths");
    let titles: i64 = sqlx::query_scalar("select count(*) from titles")
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert_eq!(titles, 0, "a refused image saves nothing");

    env.state.write_health.record(false);
    let (status, _) = env.create_with_poster("/page.jpg").await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "storage_degraded while the write probe fails"
    );
}
