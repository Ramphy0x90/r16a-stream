//! The transcode pipeline end to end (SPEC 7.3 to 7.5) against real Garage and real ffmpeg.
//!
//! The API side starts and completes the upload as an admin's browser would, the worker side
//! connects as `r16a_stream_worker` with the worker Garage key, so the grants of SPEC 4.9 and the
//! key scoping are exercised too. Clips are generated with ffmpeg's `lavfi` sources, hostile ones
//! included. Needs `R16A_TEST_S3_ENDPOINT`, `R16A_TEST_WORKER_DATABASE_URL` and ffmpeg in
//! `/usr/bin`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    Extension,
    body::Body,
    extract::ConnectInfo,
    http::{Method, Request, StatusCode, header},
    response::Response,
};
use common::idp::MockIdp;
use r16a_stream_backend::{
    app,
    config::Encoder,
    jobs::repository as jobs,
    state::AppState,
    storage::{BucketName, GarageClient},
    transcode::{housekeeping::HousekeepingWorker, worker::TranscodeWorker},
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

struct Env {
    state: AppState,
    admin: String,
    worker: TranscodeWorker,
    housekeeping: HousekeepingWorker,
    media: Arc<GarageClient>,
    dir: PathBuf,
    _idp: MockIdp,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn setup(pool: PgPool) -> Option<Env> {
    let s3 = common::integration_env("R16A_TEST_S3_ENDPOINT")?;
    let worker_url = common::integration_env("R16A_TEST_WORKER_DATABASE_URL")?;
    if !Path::new("/usr/bin/ffmpeg").exists() {
        assert!(
            std::env::var("R16A_REQUIRE_INTEGRATION").as_deref() != Ok("1"),
            "ffmpeg is required"
        );
        return None;
    }
    r16a_stream_backend::install_crypto_provider();
    let idp = MockIdp::start().await;
    let mut vars = idp.vars("http://localhost:4200");
    vars.insert("R16A_S3_INTERNAL_ENDPOINTS", s3.clone());
    vars.insert("R16A_S3_PUBLIC_ENDPOINT", s3);
    let state = common::state_with(pool.clone(), &vars);
    state.write_health.record(true);

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
    let media = Arc::new(GarageClient::new(&common::config_from(&wvars).s3).unwrap());
    let dir = std::env::temp_dir().join(format!("r16a-transcode-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(dir.join("scratch")).unwrap();
    let worker = TranscodeWorker {
        db: worker_pool,
        storage: media.clone(),
        ffmpeg: PathBuf::from("/usr/bin/ffmpeg"),
        ffprobe: PathBuf::from("/usr/bin/ffprobe"),
        scratch_root: dir.join("scratch"),
        instance: format!("test-{}", Uuid::new_v4().simple()),
        encoder: Encoder::X264,
        scratch_reserve: 0,
        min_free: 1000,
        scratch_in_use: Arc::default(),
    };
    let housekeeping = HousekeepingWorker {
        db: worker.db.clone(),
        storage: media.clone(),
        instance: worker.instance.clone(),
        gc_grace: std::time::Duration::from_secs(3600),
    };
    let env = Env {
        state,
        admin: idp.mobile_token("admin", true, true),
        worker,
        housekeeping,
        media,
        dir,
        _idp: idp,
    };
    env.call(Method::GET, "/api/v1/me", None).await;
    Some(env)
}

/// Writes a clip with ffmpeg's test sources. `extra` comes before the output.
fn clip(dir: &Path, name: &str, inputs: &[&str], extra: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let status = std::process::Command::new("/usr/bin/ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(inputs)
        .args(extra)
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success(), "{name}");
    out
}

fn good_clip(dir: &Path) -> PathBuf {
    clip(
        dir,
        "good.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=854x480:rate=25",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100",
        ],
        &[
            "-t",
            "21",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            "-metadata:s:a:0",
            "language=ita",
        ],
    )
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

    /// A published show with one episode and an Italian variant.
    async fn variant(&self) -> (Uuid, Uuid) {
        let (s, t) = self
            .call(
                Method::POST,
                "/api/v1/admin/titles",
                Some(json!({"kind": "show", "name": format!("S {}", Uuid::new_v4().simple()),
                            "seasons": [{"number": 1, "episodes": [{"number": 1, "runtimeMinutes": 1}]}]})),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED);
        let title: Uuid = t["id"].as_str().unwrap().parse().unwrap();
        self.call(Method::POST, &format!("/api/v1/admin/titles/{title}/publish"), None)
            .await;
        let (_, d) = self
            .call(Method::GET, &format!("/api/v1/admin/titles/{title}"), None)
            .await;
        let episode = d["seasons"][0]["episodes"][0]["id"].as_str().unwrap();
        let (s, v) = self
            .call(
                Method::POST,
                "/api/v1/admin/variants",
                Some(json!({"titleId": title, "episodeId": episode, "language": "it"})),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED);
        (title, v["id"].as_str().unwrap().parse().unwrap())
    }

    /// Uploads a file through the API flow and returns the queued job.
    async fn upload(&self, variant: Uuid, file: &Path) -> Uuid {
        let bytes = std::fs::read(file).unwrap();
        let (s, started) = self
            .call(
                Method::POST,
                &format!("/api/v1/admin/variants/{variant}/uploads"),
                Some(json!({"fileName": "clip.mkv", "sizeBytes": bytes.len()})),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{started}");
        let upload = started["uploadId"].as_str().unwrap();
        let (_, urls) = self
            .call(
                Method::POST,
                &format!("/api/v1/admin/uploads/{upload}/parts"),
                Some(json!({"partNumbers": [1]})),
            )
            .await;
        let res = reqwest::Client::new()
            .put(urls["parts"][0]["url"].as_str().unwrap())
            .body(bytes)
            .send()
            .await
            .unwrap();
        let etag = res.headers()["etag"].to_str().unwrap().to_owned();
        let (s, done) = self
            .call(
                Method::POST,
                &format!("/api/v1/admin/uploads/{upload}/complete"),
                Some(json!({"parts": [{"partNumber": 1, "etag": etag}]})),
            )
            .await;
        assert_eq!(s, StatusCode::ACCEPTED, "{done}");
        done["jobId"].as_str().unwrap().parse().unwrap()
    }

    async fn job(&self, id: Uuid) -> (String, Option<String>, Option<String>, i16) {
        sqlx::query_as("select status, failure_code, blocked_reason, attempts from jobs where id = $1")
            .bind(id)
            .fetch_one(&self.state.db)
            .await
            .unwrap()
    }

    async fn variant_state(&self, id: Uuid) -> (String, Option<Uuid>) {
        sqlx::query_as("select status, current_attempt_id from media_variants where id = $1")
            .bind(id)
            .fetch_one(&self.state.db)
            .await
            .unwrap()
    }

    async fn reserved(&self) -> i64 {
        sqlx::query_scalar("select coalesce(sum(reserved_bytes), 0)::bigint from uploads")
            .fetch_one(&self.state.db)
            .await
            .unwrap()
    }

    /// The stored `Content-Type`, read with a presigned GET as a player would.
    async fn content_type(&self, key: &str) -> String {
        let s3 = common::integration_env("R16A_TEST_S3_ENDPOINT").unwrap();
        let bucket = rusty_s3::Bucket::new(
            s3.parse().unwrap(),
            rusty_s3::UrlStyle::Path,
            "r16a-stream-media".to_owned(),
            "garage".to_owned(),
        )
        .unwrap();
        let creds = rusty_s3::Credentials::new(common::WORKER_KEY_ID, common::WORKER_SECRET);
        let url = rusty_s3::S3Action::sign(
            &bucket.get_object(Some(&creds), key),
            std::time::Duration::from_secs(60),
        );
        let res = reqwest::get(url).await.unwrap();
        res.headers()[header::CONTENT_TYPE].to_str().unwrap().to_owned()
    }

    fn scratch_is_empty(&self) -> bool {
        std::fs::read_dir(&self.worker.scratch_root).unwrap().next().is_none()
    }
}

#[sqlx::test]
async fn a_clip_becomes_a_verified_ladder_and_the_variant_switches(pool: PgPool) {
    let Some(env) = setup(pool).await else { return };
    let (title, variant) = env.variant().await;
    let job = env.upload(variant, &good_clip(&env.dir)).await;
    assert!(env.reserved().await > 0);
    assert!(env.worker.run_once().await.unwrap());

    assert_eq!(env.job(job).await, ("succeeded".into(), None, None, 0));
    let (status, current) = env.variant_state(variant).await;
    assert_eq!(status, "ready");
    let attempt = current.unwrap();
    assert_eq!(env.reserved().await, 0, "the reservation ends with the transcode");
    assert!(env.scratch_is_empty(), "scratch removed");

    let (prefix, total, encoder, scrub, audio): (String, i64, String, String, String) = sqlx::query_as(
        "select storage_prefix, total_bytes, encoder, scrub_vtt_key, audio_playlist_key
         from transcode_attempts where id = $1 and status = 'ready' and verified_at is not null",
    )
    .bind(attempt)
    .fetch_one(&env.state.db)
    .await
    .unwrap();
    assert_eq!(prefix, format!("v/{variant}/{attempt}/"));
    assert_eq!(encoder, "x264");
    let renditions: Vec<(i16, i16, i32, i32, String)> = sqlx::query_as(
        "select height, width, bandwidth, average_bandwidth, codecs from renditions where attempt_id = $1
         order by height desc",
    )
    .bind(attempt)
    .fetch_all(&env.state.db)
    .await
    .unwrap();
    assert_eq!(
        renditions.iter().map(|r| (r.0, r.1, r.4.as_str())).collect::<Vec<_>>(),
        [(480, 854, "avc1.4d401f"), (360, 640, "avc1.4d401e")]
    );
    assert!(
        renditions.iter().all(|r| r.2 >= r.3 && r.3 > 0),
        "peak at least the average"
    );

    // Everything listed back from Garage, with the content types of SPEC 10.4.
    let objects = env.media.list_prefix(BucketName::Media, &prefix).await.unwrap();
    assert_eq!(
        objects.iter().map(|o| o.size).sum::<u64>(),
        u64::try_from(total).unwrap()
    );
    let keys: Vec<String> = objects
        .iter()
        .map(|o| o.key.strip_prefix(&prefix).unwrap().to_owned())
        .collect();
    for k in [
        "v480/index.m3u8",
        "v480/init.mp4",
        "v480/seg_0.m4s",
        "v360/index.m3u8",
        "a/index.m3u8",
        "a/init.mp4",
        "scrub/sprite_1.webp",
        "scrub/scrub.vtt",
    ] {
        assert!(keys.contains(&k.to_owned()), "{k} in {keys:?}");
    }
    for (key, want) in [
        (audio.clone(), "application/vnd.apple.mpegurl"),
        (format!("{prefix}a/seg_0.m4s"), "audio/mp4"),
        (format!("{prefix}v480/seg_0.m4s"), "video/mp4"),
        (format!("{prefix}scrub/sprite_1.webp"), "image/webp"),
        (scrub.clone(), "text/vtt; charset=utf-8"),
    ] {
        let (_, cache) = env.media.get_object(BucketName::Media, &key).await.unwrap();
        assert_eq!(cache.as_deref(), Some("public, max-age=31536000, immutable"), "{key}");
        assert_eq!(env.content_type(&key).await, want, "{key}");
    }
    let (vtt, _) = env.media.get_object(BucketName::Media, &scrub).await.unwrap();
    let vtt = String::from_utf8(vtt).unwrap();
    assert!(vtt.contains("00:00:20.000 --> 00:00:21."), "{vtt}");
    assert!(vtt.contains("sprite_1.webp#xywh=320,0,160,90"), "{vtt}");

    let langs: Vec<String> = sqlx::query_scalar("select language from title_languages where title_id = $1")
        .bind(title)
        .fetch_all(&env.state.db)
        .await
        .unwrap();
    assert_eq!(langs, ["it"]);
    let (_, eps) = env
        .call(Method::GET, &format!("/api/v1/titles/{title}/seasons/1/episodes"), None)
        .await;
    assert_eq!(
        eps[0]["languages"],
        json!(["it"]),
        "episode languages come from ready variants"
    );

    // A re-transcode switches to a new attempt and keeps the variant ready meanwhile.
    let (s, re) = env
        .call(
            Method::POST,
            &format!("/api/v1/admin/variants/{variant}/retranscode"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{re}");
    assert_eq!(env.variant_state(variant).await, ("ready".into(), Some(attempt)));
    assert!(env.worker.run_once().await.unwrap());
    let (status, second) = env.variant_state(variant).await;
    assert_eq!(status, "ready");
    assert_ne!(second, Some(attempt));
    assert_eq!(env.reserved().await, 0);

    for p in [prefix, format!("v/{variant}/{}/", second.unwrap())] {
        env.media.delete_prefix(BucketName::Media, &p).await.unwrap();
    }
}

#[sqlx::test]
async fn hostile_and_unfit_sources_fail_once_without_retries(pool: PgPool) {
    let Some(env) = setup(pool).await else { return };
    let dir = env.dir.clone();
    let text = dir.join("text.mkv");
    std::fs::write(&text, b"this is not a video at all, just text").unwrap();
    let playlist = dir.join("list.mkv");
    std::fs::write(&playlist, b"#EXTM3U\n#EXTINF:10,\nfile:///etc/passwd\n#EXT-X-ENDLIST\n").unwrap();
    let silent = clip(
        &dir,
        "silent.mkv",
        &["-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25"],
        &["-t", "12", "-c:v", "libx264", "-preset", "ultrafast"],
    );
    let short = clip(
        &dir,
        "short.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=25",
            "-f",
            "lavfi",
            "-i",
            "sine",
        ],
        &["-t", "4", "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac"],
    );
    for (file, code) in [
        (&text, "probe-unreadable"),
        (&playlist, "probe-unreadable"),
        (&silent, "no-audio"),
        (&short, "duration"),
    ] {
        let (_, variant) = env.variant().await;
        let job = env.upload(variant, file).await;
        assert!(env.worker.run_once().await.unwrap());
        assert_eq!(
            env.job(job).await,
            ("failed".into(), Some(code.into()), None, 1),
            "{code}"
        );
        assert_eq!(env.variant_state(variant).await, ("failed".into(), None));
        assert_eq!(env.reserved().await, 0, "{code} released its reservation");
        assert!(env.scratch_is_empty());
    }
}

#[sqlx::test]
async fn space_shortages_requeue_without_counting_a_failure(pool: PgPool) {
    let Some(mut env) = setup(pool).await else { return };
    let (_, variant) = env.variant().await;
    let job = env.upload(variant, &good_clip(&env.dir)).await;

    env.worker.scratch_reserve = u64::MAX / 4;
    assert!(env.worker.run_once().await.unwrap());
    assert_eq!(
        env.job(job).await,
        ("queued".into(), None, Some("scratch_full".into()), 0)
    );
    assert!(env.reserved().await > 0, "a waiting job keeps its reservation");

    // Garage too full for the outputs, by fresh capacity rows.
    env.worker.scratch_reserve = 0;
    sqlx::query(
        "insert into garage_capacity (node, data_avail, data_total, replication)
         values ('a', 1000, 1000000, 2), ('b', 1000, 1000000, 2)",
    )
    .execute(&env.state.db)
    .await
    .unwrap();
    sqlx::query("update jobs set run_after = now()")
        .execute(&env.state.db)
        .await
        .unwrap();
    assert!(env.worker.run_once().await.unwrap());
    assert_eq!(
        env.job(job).await,
        ("queued".into(), None, Some("storage_full".into()), 0)
    );
    let abandoned: i64 =
        sqlx::query_scalar("select count(*) from transcode_attempts where status = 'abandoned' and variant_id = $1")
            .bind(variant)
            .fetch_one(&env.state.db)
            .await
            .unwrap();
    assert_eq!(abandoned, 1);
    assert!(env.scratch_is_empty());

    // Cancelled while waiting, the reservation goes and the variant shows it.
    let (s, _) = env
        .call(Method::POST, &format!("/api/v1/admin/jobs/{job}/cancel"), None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(env.reserved().await, 0);
    assert_eq!(env.variant_state(variant).await.0, "failed");
    // A retry reserves again, once there is room.
    sqlx::query("delete from garage_capacity")
        .execute(&env.state.db)
        .await
        .unwrap();
    let (s, _) = env
        .call(Method::POST, &format!("/api/v1/admin/jobs/{job}/retry"), None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(env.reserved().await > 0);
    assert_eq!(env.variant_state(variant).await.0, "processing");
}

#[sqlx::test]
async fn the_reaper_abandons_attempts_of_dead_workers(pool: PgPool) {
    let Some(env) = setup(pool).await else { return };
    let (_, variant) = env.variant().await;
    let job = env.upload(variant, &good_clip(&env.dir)).await;
    let upload: Uuid = sqlx::query_scalar("select id from uploads where variant_id = $1")
        .bind(variant)
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    sqlx::query(
        "update jobs set status = 'running', locked_by = 'gone', heartbeat_at = now() - interval '5 minutes'
         where id = $1",
    )
    .bind(job)
    .execute(&env.state.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into transcode_attempts (variant_id, upload_id, job_id, encoder, storage_prefix)
         values ($1, $2, $3, 'x264', 'v/' || $1 || '/dead/')",
    )
    .bind(variant)
    .bind(upload)
    .bind(job)
    .execute(&env.state.db)
    .await
    .unwrap();
    assert_eq!(
        jobs::reap_stale(&env.worker.db, 120.0).await.unwrap(),
        1,
        "as the worker role"
    );
    assert_eq!(env.job(job).await.0, "queued");
    let status: String = sqlx::query_scalar("select status from transcode_attempts where job_id = $1")
        .bind(job)
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert_eq!(status, "abandoned");

    // A job that was being cancelled ends cancelled, not queued.
    sqlx::query(
        "update jobs set status = 'cancelling', locked_by = 'gone', heartbeat_at = now() - interval '5 minutes'
         where id = $1",
    )
    .bind(job)
    .execute(&env.state.db)
    .await
    .unwrap();
    jobs::reap_stale(&env.worker.db, 120.0).await.unwrap();
    assert_eq!(env.job(job).await.0, "cancelled");
}

#[sqlx::test]
async fn subtitles_are_promoted_and_their_staged_files_removed(pool: PgPool) {
    let Some(env) = setup(pool).await else { return };
    let (_, variant) = env.variant().await;
    let vtt: &'static [u8] = b"WEBVTT\n\n00:00.000 --> 00:01.000\nCiao\n";
    let send = |language: &'static str| {
        let req = Request::builder()
            .method(Method::POST)
            .uri(format!(
                "/api/v1/admin/variants/{variant}/subtitles?language={language}"
            ))
            .header(header::AUTHORIZATION, format!("Bearer {}", env.admin))
            .header(header::CONTENT_TYPE, "text/vtt")
            .body(Body::from(vtt))
            .unwrap();
        app::router(env.state.clone())
            .layer(Extension(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 1)))))
            .oneshot(req)
    };
    let res = send("it").await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(res.into_body(), 1 << 16).await.unwrap();
    let id: Uuid = serde_json::from_slice::<Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    assert!(env.housekeeping.run_once().await.unwrap());
    let (status, key, size): (String, String, i32) =
        sqlx::query_as("select status, object_key, byte_size from subtitle_tracks where id = $1")
            .bind(id)
            .fetch_one(&env.state.db)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), key.as_str()),
        ("ready", format!("s/{id}.vtt").as_str())
    );
    assert_eq!(usize::try_from(size).unwrap(), vtt.len());
    assert_eq!(env.content_type(&key).await, "text/vtt; charset=utf-8");

    // The API removes the staged copy, the worker key could not.
    let staged = format!("subtitles/{id}.vtt");
    assert!(
        env.state
            .storage
            .head_object(BucketName::Uploads, &staged)
            .await
            .unwrap()
            .is_some()
    );
    let removed = r16a_stream_backend::media::tasks::cleanup_staged_subtitles(&env.state.db, &env.state.storage)
        .await
        .unwrap();
    assert_eq!(removed, 1);
    assert_eq!(
        env.state
            .storage
            .head_object(BucketName::Uploads, &staged)
            .await
            .unwrap(),
        None
    );

    // A staged file swapped for something else after the API checked it is refused again.
    let res = send("en").await.unwrap();
    let body = axum::body::to_bytes(res.into_body(), 1 << 16).await.unwrap();
    let other: Uuid = serde_json::from_slice::<Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    env.state
        .storage
        .put_object(
            BucketName::Uploads,
            &format!("subtitles/{other}.vtt"),
            b"<html>",
            "text/vtt",
            None,
        )
        .await
        .unwrap();
    assert!(env.housekeeping.run_once().await.unwrap());
    let status: String = sqlx::query_scalar("select status from subtitle_tracks where id = $1")
        .bind(other)
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert_eq!(status, "failed");
    env.media.delete_object(BucketName::Media, &key).await.unwrap();
    r16a_stream_backend::media::tasks::cleanup_staged_subtitles(&env.state.db, &env.state.storage)
        .await
        .unwrap();
}

#[sqlx::test]
async fn encode_gc_waits_out_the_grace_and_keeps_live_attempts(pool: PgPool) {
    let Some(env) = setup(pool).await else { return };
    let (_, variant) = env.variant().await;
    let job = env.upload(variant, &good_clip(&env.dir)).await;
    assert!(env.worker.run_once().await.unwrap());
    assert_eq!(env.job(job).await.0, "succeeded");
    let current = env.variant_state(variant).await.1.unwrap();
    let live = format!("v/{variant}/{current}/");
    // An attempt nobody references any more, and a running one that must survive too.
    let dead = format!("v/{variant}/{}/", Uuid::new_v4());
    let running = Uuid::new_v4();
    let upload: Uuid = sqlx::query_scalar("select id from uploads where variant_id = $1")
        .bind(variant)
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    sqlx::query(
        "insert into transcode_attempts (id, variant_id, upload_id, encoder, storage_prefix)
         values ($1, $2, $3, 'x264', 'v/' || $2 || '/' || $1 || '/')",
    )
    .bind(running)
    .bind(variant)
    .bind(upload)
    .execute(&env.state.db)
    .await
    .unwrap();
    let running_prefix = format!("v/{variant}/{running}/");
    for p in [&dead, &running_prefix] {
        env.media
            .put_object(
                BucketName::Media,
                &format!("{p}v360/seg_0.m4s"),
                b"x",
                "video/mp4",
                None,
            )
            .await
            .unwrap();
    }

    let gc = || {
        let db = env.state.db.clone();
        async move {
            sqlx::query(
                "insert into jobs (kind, dedupe_key, run_after) values ('encode_gc', 'encode_gc', now())
                 on conflict (dedupe_key) where status in ('queued', 'running', 'cancelling')
                 do update set run_after = now()",
            )
            .execute(&db)
            .await
            .unwrap();
        }
    };
    gc().await;
    assert!(env.housekeeping.run_once().await.unwrap());
    let seen: Vec<String> = sqlx::query_scalar("select prefix from gc_orphans where prefix like $1")
        .bind(format!("v/{variant}/%"))
        .fetch_all(&env.state.db)
        .await
        .unwrap();
    assert_eq!(seen, std::slice::from_ref(&dead), "only the dead attempt is an orphan");
    assert!(
        !env.media
            .list_prefix(BucketName::Media, &dead)
            .await
            .unwrap()
            .is_empty(),
        "grace not over"
    );
    let next: i64 = sqlx::query_scalar(
        "select count(*) from jobs where kind = 'encode_gc' and status = 'queued' and run_after > now() + interval '5 hours'",
    )
    .fetch_one(&env.state.db)
    .await
    .unwrap();
    assert_eq!(next, 1, "the next run is queued");

    // Only this test's orphan is backdated, so nothing else in the shared bucket becomes due.
    sqlx::query("update gc_orphans set first_seen_at = now() - interval '2 hours' where prefix = $1")
        .bind(&dead)
        .execute(&env.state.db)
        .await
        .unwrap();
    gc().await;
    assert!(env.housekeeping.run_once().await.unwrap());
    assert!(
        env.media
            .list_prefix(BucketName::Media, &dead)
            .await
            .unwrap()
            .is_empty(),
        "deleted"
    );
    assert!(
        !env.media
            .list_prefix(BucketName::Media, &live)
            .await
            .unwrap()
            .is_empty(),
        "current kept"
    );
    assert!(
        !env.media
            .list_prefix(BucketName::Media, &running_prefix)
            .await
            .unwrap()
            .is_empty(),
        "running kept"
    );
    for p in [live, running_prefix] {
        env.media.delete_prefix(BucketName::Media, &p).await.unwrap();
    }
}

#[sqlx::test]
async fn an_unreachable_garage_requeues_without_counting_a_failure(pool: PgPool) {
    let Some(mut env) = setup(pool).await else { return };
    let (_, variant) = env.variant().await;
    let job = env.upload(variant, &good_clip(&env.dir)).await;
    let mut vars = common::test_vars("http://localhost:4200", "http://127.0.0.1:9");
    vars.insert("R16A_S3_ACCESS_KEY_ID", common::WORKER_KEY_ID.into());
    vars.insert("R16A_S3_SECRET_ACCESS_KEY", common::WORKER_SECRET.into());
    env.worker.storage = Arc::new(GarageClient::new(&common::config_from(&vars).s3).unwrap());
    assert!(env.worker.run_once().await.unwrap());
    let (status, code, blocked, attempts) = env.job(job).await;
    assert_eq!((status.as_str(), code, attempts), ("queued", None, 0));
    assert_eq!(blocked.as_deref(), Some("storage_degraded"));
    let waits: i32 = sqlx::query_scalar("select storage_waits from jobs where id = $1")
        .bind(job)
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert_eq!(waits, 1);
    assert_eq!(env.variant_state(variant).await.0, "processing", "nothing failed");
    assert!(env.reserved().await > 0, "the reservation stays for the retry");
    assert!(env.scratch_is_empty());
}
