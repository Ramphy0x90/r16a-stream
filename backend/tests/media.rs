//! Variants, uploads, admission, retention and job control (SPEC 4.5, 7.1, 7.2, 7.9, 7.10)
//! against real Garage. Part uploads go to the URLs the API signs, as a browser sends them.
//! Needs `R16A_TEST_S3_ENDPOINT`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{collections::HashMap, net::SocketAddr, sync::Arc};

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
    media::tasks,
    state::AppState,
    storage::{BucketName, GarageClient},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

struct Env {
    state: AppState,
    admin: String,
    member: String,
    _idp: MockIdp,
}

async fn env(pool: PgPool, extra: &[(&'static str, &str)]) -> Option<Env> {
    let s3 = common::integration_env("R16A_TEST_S3_ENDPOINT")?;
    let idp = MockIdp::start().await;
    let mut vars: HashMap<&'static str, String> = idp.vars("http://localhost:4200");
    vars.insert("R16A_S3_INTERNAL_ENDPOINTS", s3.clone());
    // The browser reaches Garage on the app's origin through Traefik. Here it talks to Garage
    // directly, which validates the same signature.
    vars.insert("R16A_S3_PUBLIC_ENDPOINT", s3);
    for (k, v) in extra {
        vars.insert(k, (*v).to_owned());
    }
    let state = common::state_with(pool, &vars);
    state.write_health.record(true);
    let env = Env {
        state,
        admin: idp.mobile_token("admin", true, true),
        member: idp.mobile_token("member", true, false),
        _idp: idp,
    };
    env.call(Method::GET, "/api/v1/me", None).await;
    env.as_member(Method::GET, "/api/v1/me", None).await;
    Some(env)
}

impl Env {
    async fn send(&self, token: &str, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
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
        let res: Response = app::router(self.state.clone())
            .layer(Extension(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 1)))))
            .oneshot(b.body(body).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        self.send(&self.admin, method, uri, body).await
    }

    async fn as_member(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        self.send(&self.member, method, uri, body).await
    }

    /// A movie with a one minute runtime and an Italian variant.
    async fn movie_variant(&self) -> (String, String) {
        let (s, t) = self
            .call(
                Method::POST,
                "/api/v1/admin/titles",
                Some(json!({"kind": "movie", "name": format!("M {}", Uuid::new_v4().simple()), "runtimeMinutes": 1})),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED);
        let title = t["id"].as_str().unwrap().to_owned();
        let (s, v) = self
            .call(
                Method::POST,
                "/api/v1/admin/variants",
                Some(json!({"titleId": title, "language": "IT"})),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        (title, v["id"].as_str().unwrap().to_owned())
    }

    async fn start(&self, variant: &str, size: u64) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            &format!("/api/v1/admin/variants/{variant}/uploads"),
            Some(json!({"fileName": "film.mkv", "sizeBytes": size})),
        )
        .await
    }

    /// Uploads part 1 through a signed URL and returns its tag.
    async fn put_part(&self, upload: &str, body: Vec<u8>) -> String {
        let (s, urls) = self
            .call(
                Method::POST,
                &format!("/api/v1/admin/uploads/{upload}/parts"),
                Some(json!({"partNumbers": [1]})),
            )
            .await;
        assert_eq!(s, StatusCode::OK);
        let url = urls["parts"][0]["url"].as_str().unwrap();
        let res = reqwest::Client::new().put(url).body(body).send().await.unwrap();
        assert_eq!(res.status(), 200);
        res.headers()["etag"].to_str().unwrap().to_owned()
    }

    fn garage(&self) -> &GarageClient {
        &self.state.storage
    }

    async fn audit_actions(&self) -> Vec<String> {
        sqlx::query_scalar("select action from audit_log order by id")
            .fetch_all(&self.state.db)
            .await
            .unwrap()
    }
}

fn install() {
    r16a_stream_backend::install_crypto_provider();
}

#[sqlx::test]
async fn upload_completes_into_a_queued_transcode(pool: PgPool) {
    install();
    let Some(env) = env(pool, &[]).await else { return };
    let (title, variant) = env.movie_variant().await;
    let (s, _) = env
        .call(
            Method::POST,
            "/api/v1/admin/variants",
            Some(json!({"titleId": title, "language": "it"})),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "the language is normalised before the unique check"
    );

    let body = b"not really a film".to_vec();
    let (s, started) = env.start(&variant, body.len() as u64).await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    assert_eq!(started["partCount"], 1);
    let upload = started["uploadId"].as_str().unwrap().to_owned();
    assert_eq!(
        env.start(&variant, 10).await.0,
        StatusCode::CONFLICT,
        "one open upload per variant"
    );

    let etag = env.put_part(&upload, body.clone()).await;
    let (s, state) = env
        .call(Method::GET, &format!("/api/v1/admin/uploads/{upload}"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(state["parts"][0]["etag"], etag.as_str(), "resume sees the stored part");

    let complete = format!("/api/v1/admin/uploads/{upload}/complete");
    let (s, _) = env.call(Method::POST, &complete, Some(json!({"parts": []}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, done) = env
        .call(
            Method::POST,
            &complete,
            Some(json!({"parts": [{"partNumber": 1, "etag": etag}]})),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{done}");
    let job: Uuid = done["jobId"].as_str().unwrap().parse().unwrap();

    let (kind, payload, status): (String, Value, String) =
        sqlx::query_as("select kind, payload, status from jobs where id = $1")
            .bind(job)
            .fetch_one(&env.state.db)
            .await
            .unwrap();
    assert_eq!((kind.as_str(), status.as_str()), ("transcode", "queued"));
    assert_eq!(payload, json!({"uploadId": upload}), "ids only in the payload");
    let (vstatus,): (String,) = sqlx::query_as("select status from media_variants where id = $1::uuid")
        .bind(&variant)
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert_eq!(vstatus, "processing");

    let (_, storage) = env
        .call(Method::GET, &format!("/api/v1/admin/titles/{title}/storage"), None)
        .await;
    let v = &storage["variants"][0];
    assert_eq!(v["sourceKept"], true);
    assert_eq!(v["sourceBytes"], body.len());
    assert_eq!(v["job"]["status"], "queued");

    let (s, _) = env
        .call(
            Method::POST,
            &format!("/api/v1/admin/variants/{variant}/retranscode"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "a job is already queued");
    assert_eq!(
        env.audit_actions().await,
        ["title.create", "variant.create", "upload.start", "upload.complete"]
    );
    let key = format!("sources/{variant}/{upload}");
    env.garage().delete_object(BucketName::Uploads, &key).await.unwrap();
}

#[sqlx::test]
async fn a_short_upload_is_refused_and_removed(pool: PgPool) {
    install();
    let Some(env) = env(pool, &[]).await else { return };
    let (_, variant) = env.movie_variant().await;
    let (_, started) = env.start(&variant, 100).await;
    let upload = started["uploadId"].as_str().unwrap().to_owned();
    let etag = env.put_part(&upload, vec![b'x'; 50]).await;
    let (s, p) = env
        .call(
            Method::POST,
            &format!("/api/v1/admin/uploads/{upload}/complete"),
            Some(json!({"parts": [{"partNumber": 1, "etag": etag}]})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(p["type"], "urn:r16a-stream:problem:size-mismatch");
    let key = format!("sources/{variant}/{upload}");
    assert_eq!(env.garage().head_object(BucketName::Uploads, &key).await.unwrap(), None);
    let (status, reserved): (String, i64) =
        sqlx::query_as("select status, reserved_bytes from uploads where id = $1::uuid")
            .bind(&upload)
            .fetch_one(&env.state.db)
            .await
            .unwrap();
    assert_eq!((status.as_str(), reserved), ("aborted", 0));
    // The variant is free for a new upload.
    let (s, again) = env.start(&variant, 10).await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = env
        .call(
            Method::DELETE,
            &format!("/api/v1/admin/uploads/{}", again["uploadId"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

async fn set_capacity(db: &PgPool, avail: i64, fresh: bool) {
    sqlx::query(
        "insert into garage_capacity (node, data_avail, data_total, replication, checked_at)
         values ('n1', $1, 1000000000000, 2, now() - case when $2 then interval '0' else interval '1 hour' end),
                ('n2', $1, 1000000000000, 2, now() - case when $2 then interval '0' else interval '1 hour' end)
         on conflict (node) do update set data_avail = excluded.data_avail, checked_at = excluded.checked_at",
    )
    .bind(avail)
    .bind(fresh)
    .execute(db)
    .await
    .unwrap();
}

#[sqlx::test]
async fn admission_refuses_what_does_not_fit_with_the_numbers(pool: PgPool) {
    install();
    let extra = [
        (
            "R16A_GARAGE_METRICS_ENDPOINTS",
            "http://10.0.0.1:3903/metrics,http://10.0.0.2:3903/metrics",
        ),
        ("R16A_GARAGE_METRICS_TOKEN", "metrics-token-for-tests-only-7f3a9c2e5b1d"),
        ("R16A_STORAGE_MIN_FREE_BYTES", "1000"),
        ("R16A_STORAGE_UNKNOWN_BUDGET_BYTES", "200000000"),
    ];
    let Some(env) = env(pool, &extra).await else { return };
    let (_, variant) = env.movie_variant().await;
    // One minute of runtime plus 10 % estimates about 106 MB of ladder output.
    set_capacity(&env.state.db, 50_000_000, true).await;
    let (s, p) = env.start(&variant, 1_000_000).await;
    assert_eq!(s, StatusCode::INSUFFICIENT_STORAGE, "{p}");
    assert_eq!(p["type"], "urn:r16a-stream:problem:insufficient_storage");
    assert_eq!(p["availableBytes"], 50_000_000 - 1000);
    assert!(p["neededBytes"].as_u64().unwrap() > 100_000_000);

    set_capacity(&env.state.db, 900_000_000_000, true).await;
    let (s, ok) = env.start(&variant, 1_000_000).await;
    assert_eq!(s, StatusCode::CREATED);
    let reserved: i64 = sqlx::query_scalar("select reserved_bytes from uploads")
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert!(reserved > 100_000_000, "source plus ladder is reserved");

    // Stale rows make capacity unknown, and the budget counts what is already reserved.
    let (_, other) = env
        .call(
            Method::POST,
            "/api/v1/admin/variants",
            Some(json!({"titleId": movie_of(&env, &variant).await, "language": "de"})),
        )
        .await;
    set_capacity(&env.state.db, 900_000_000_000, false).await;
    let (s, p) = env.start(other["id"].as_str().unwrap(), 1_000_000).await;
    assert_eq!(s, StatusCode::INSUFFICIENT_STORAGE);
    assert_eq!(p["availableBytes"].as_i64().unwrap(), 200_000_000 - reserved);
    let (_, overview) = env.call(Method::GET, "/api/v1/admin/storage", None).await;
    assert_eq!(overview["capacityKnown"], false);
    assert_eq!(overview["unknownReason"], "stale");
    assert_eq!(overview["warning"], true);
    assert_eq!(overview["reservedBytes"].as_i64().unwrap(), reserved);
    env.call(
        Method::DELETE,
        &format!("/api/v1/admin/uploads/{}", ok["uploadId"].as_str().unwrap()),
        None,
    )
    .await;
}

async fn movie_of(env: &Env, variant: &str) -> String {
    let id: Uuid = sqlx::query_scalar("select title_id from media_variants where id = $1::uuid")
        .bind(variant)
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    id.to_string()
}

#[sqlx::test]
async fn variants_follow_the_title_kind(pool: PgPool) {
    install();
    let Some(env) = env(pool, &[]).await else { return };
    let (movie, _) = env.movie_variant().await;
    let (_, show) = env
        .call(
            Method::POST,
            "/api/v1/admin/titles",
            Some(json!({"kind": "show", "name": "S", "seasons": [{"number": 1, "episodes": [{"number": 1}]}]})),
        )
        .await;
    let show = show["id"].as_str().unwrap();
    let (_, detail) = env
        .call(Method::GET, &format!("/api/v1/admin/titles/{show}"), None)
        .await;
    let episode = detail["seasons"][0]["episodes"][0]["id"].as_str().unwrap();
    for (body, want) in [
        (json!({"titleId": show, "language": "it"}), StatusCode::BAD_REQUEST),
        (
            json!({"titleId": movie, "episodeId": episode, "language": "de"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"titleId": movie, "language": "english"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"titleId": show, "episodeId": Uuid::new_v4(), "language": "it"}),
            StatusCode::NOT_FOUND,
        ),
        (
            json!({"titleId": show, "episodeId": episode, "language": "pt-br"}),
            StatusCode::CREATED,
        ),
    ] {
        let (s, p) = env
            .call(Method::POST, "/api/v1/admin/variants", Some(body.clone()))
            .await;
        assert_eq!(s, want, "{body} {p}");
    }
    let lang: String = sqlx::query_scalar("select language from media_variants where episode_id is not null")
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert_eq!(lang, "pt-BR");
    let (s, _) = env
        .as_member(Method::GET, &format!("/api/v1/admin/titles/{show}/storage"), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

/// A completed upload with its object in Garage, transcoded into a ready, verified attempt.
async fn ready_variant(env: &Env) -> (String, Uuid, String) {
    let (_, variant) = env.movie_variant().await;
    let admin_id: Uuid = sqlx::query_scalar("select id from users limit 1")
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    let upload = Uuid::new_v4();
    let key = format!("sources/{variant}/{upload}");
    env.garage()
        .put_object(BucketName::Uploads, &key, b"source", "application/octet-stream", None)
        .await
        .unwrap();
    sqlx::query(
        "insert into uploads (id, purpose, variant_id, object_key, s3_upload_id, size_bytes, part_size, part_count,
                              status, completed_at, created_by, expires_at)
         values ($1, 'video_source', $2::uuid, $3, 'x', 6, 52428800, 1, 'completed', now(), $4, now())",
    )
    .bind(upload)
    .bind(&variant)
    .bind(&key)
    .bind(admin_id)
    .execute(&env.state.db)
    .await
    .unwrap();
    let attempt: Uuid = sqlx::query_scalar(
        "insert into transcode_attempts (variant_id, upload_id, encoder, storage_prefix, status, total_bytes, verified_at)
         values ($1::uuid, $2, 'x264', 'v/' || $1 || '/a', 'ready', 10, now()) returning id",
    )
    .bind(&variant)
    .bind(upload)
    .fetch_one(&env.state.db)
    .await
    .unwrap();
    sqlx::query("update media_variants set status = 'ready', current_attempt_id = $2 where id = $1::uuid")
        .bind(&variant)
        .bind(attempt)
        .execute(&env.state.db)
        .await
        .unwrap();
    (variant, upload, key)
}

#[sqlx::test]
async fn retention_deletes_only_verified_sources_without_pending_jobs(pool: PgPool) {
    install();
    let Some(env) = env(pool, &[]).await else { return };
    let (variant, upload, key) = ready_variant(&env).await;
    let policy = &env.state.config.storage;

    // A queued re-transcode keeps the source.
    let (s, job) = env
        .call(
            Method::POST,
            &format!("/api/v1/admin/variants/{variant}/retranscode"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{job}");
    let (_, v) = env
        .call(
            Method::GET,
            &format!("/api/v1/admin/titles/{}/storage", movie_of(&env, &variant).await),
            None,
        )
        .await;
    assert_eq!(
        v["variants"][0]["status"], "ready",
        "stays playable while it re-encodes"
    );
    assert_eq!(
        tasks::retention_round(&env.state.db, env.garage(), policy)
            .await
            .unwrap(),
        Some(0)
    );
    assert_eq!(
        env.garage().head_object(BucketName::Uploads, &key).await.unwrap(),
        Some(6)
    );

    // Once the job ended, the source goes.
    let (s, _) = env
        .call(
            Method::POST,
            &format!("/api/v1/admin/jobs/{}/cancel", job["jobId"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        tasks::retention_round(&env.state.db, env.garage(), policy)
            .await
            .unwrap(),
        Some(1)
    );
    assert_eq!(env.garage().head_object(BucketName::Uploads, &key).await.unwrap(), None);
    let deleted: bool = sqlx::query_scalar("select source_deleted_at is not null from uploads where id = $1")
        .bind(upload)
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert!(deleted);
    let actor: Option<Uuid> = sqlx::query_scalar("select actor_id from audit_log where action = 'source.delete'")
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert_eq!(actor, None, "recorded as the system");

    let (s, p) = env
        .call(
            Method::POST,
            &format!("/api/v1/admin/variants/{variant}/retranscode"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(p["type"], "urn:r16a-stream:problem:source_unavailable");
}

#[sqlx::test]
async fn keep_retention_deletes_nothing(pool: PgPool) {
    install();
    let Some(env) = env(pool, &[("R16A_SOURCE_RETENTION", "keep")]).await else {
        return;
    };
    let (_, _, key) = ready_variant(&env).await;
    assert_eq!(
        tasks::retention_round(&env.state.db, env.garage(), &env.state.config.storage)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        env.garage().head_object(BucketName::Uploads, &key).await.unwrap(),
        Some(6)
    );
    env.garage().delete_object(BucketName::Uploads, &key).await.unwrap();
}

#[sqlx::test]
async fn expired_uploads_are_aborted(pool: PgPool) {
    install();
    let Some(env) = env(pool, &[]).await else { return };
    let (_, variant) = env.movie_variant().await;
    let (_, started) = env.start(&variant, 10).await;
    let upload = started["uploadId"].as_str().unwrap().to_owned();
    env.put_part(&upload, vec![b'y'; 10]).await;
    sqlx::query("update uploads set expires_at = now() - interval '1 second'")
        .execute(&env.state.db)
        .await
        .unwrap();
    let storage: Arc<GarageClient> = env.state.storage.clone();
    assert_eq!(tasks::cleanup_stale_uploads(&env.state.db, &storage).await.unwrap(), 1);
    let (_, state) = env
        .call(Method::GET, &format!("/api/v1/admin/uploads/{upload}"), None)
        .await;
    assert_eq!(state["status"], "aborted");
    assert_eq!(
        env.call(
            Method::POST,
            &format!("/api/v1/admin/uploads/{upload}/parts"),
            Some(json!({"partNumbers": [1]}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[sqlx::test]
async fn jobs_list_retry_and_cancel(pool: PgPool) {
    install();
    let Some(env) = env(pool, &[]).await else { return };
    for (status, kind) in [("failed", "image"), ("queued", "image"), ("succeeded", "encode_gc")] {
        sqlx::query("insert into jobs (kind, status) values ($1, $2)")
            .bind(kind)
            .bind(status)
            .execute(&env.state.db)
            .await
            .unwrap();
    }
    let (_, page) = env.call(Method::GET, "/api/v1/admin/jobs?limit=2", None).await;
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    let cursor = page["nextCursor"].as_str().unwrap();
    let (_, rest) = env
        .call(
            Method::GET,
            &format!("/api/v1/admin/jobs?limit=2&cursor={cursor}"),
            None,
        )
        .await;
    assert_eq!(rest["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        env.call(Method::GET, "/api/v1/admin/jobs?status=lost", None).await.0,
        StatusCode::BAD_REQUEST
    );

    let (_, failed) = env.call(Method::GET, "/api/v1/admin/jobs?status=failed", None).await;
    let failed = failed["items"][0]["id"].as_str().unwrap().to_owned();
    let (_, queued) = env.call(Method::GET, "/api/v1/admin/jobs?status=queued", None).await;
    let queued = queued["items"][0]["id"].as_str().unwrap().to_owned();
    let post = |id: &str, what: &str| format!("/api/v1/admin/jobs/{id}/{what}");
    assert_eq!(
        env.call(Method::POST, &post(&failed, "retry"), None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        env.call(Method::POST, &post(&failed, "retry"), None).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        env.call(Method::POST, &post(&queued, "cancel"), None).await.0,
        StatusCode::NO_CONTENT
    );
    let (_, j) = env
        .call(Method::GET, &format!("/api/v1/admin/jobs/{queued}"), None)
        .await;
    assert_eq!(j["status"], "cancelled");
    let (_, j) = env
        .call(Method::GET, &format!("/api/v1/admin/jobs/{failed}"), None)
        .await;
    assert_eq!(
        (j["status"].as_str(), j["attempts"].as_i64()),
        (Some("queued"), Some(0))
    );
    assert_eq!(
        env.call(Method::POST, &post(&Uuid::new_v4().to_string(), "cancel"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test]
async fn storage_overview_without_metrics_says_not_configured(pool: PgPool) {
    install();
    let Some(env) = env(pool, &[]).await else { return };
    let (s, o) = env.call(Method::GET, "/api/v1/admin/storage", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(o["capacityKnown"], false);
    assert_eq!(o["unknownReason"], "not_configured");
    let (s, t) = env.call(Method::GET, "/api/v1/admin/storage/titles", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(t["items"], json!([]));
    env.state.write_health.record(false);
    let (_, variant) = env.movie_variant().await;
    let (s, p) = env.start(&variant, 10).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(p["type"], "urn:r16a-stream:problem:storage_degraded");
}

impl Env {
    async fn raw(&self, uri: &str, content_type: &str, body: &'static [u8]) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.admin))
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body))
            .unwrap();
        let res: Response = app::router(self.state.clone())
            .layer(Extension(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 1)))))
            .oneshot(req)
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
}

#[sqlx::test]
async fn a_manual_poster_is_attached_when_its_upload_completes(pool: PgPool) {
    install();
    let Some(env) = env(pool, &[]).await else { return };
    let (title, _) = env.movie_variant().await;
    let start = |body: Value| env.call(Method::POST, "/api/v1/admin/images", Some(body));
    for bad in [
        json!({"kind": "still", "titleId": title, "sizeBytes": 10, "contentType": "image/png"}),
        json!({"kind": "poster", "titleId": title, "sizeBytes": 30_000_000, "contentType": "image/png"}),
        json!({"kind": "poster", "titleId": title, "sizeBytes": 10, "contentType": "image/svg+xml"}),
    ] {
        assert_eq!(start(bad.clone()).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (s, _) =
        start(json!({"kind": "poster", "titleId": Uuid::new_v4(), "sizeBytes": 10, "contentType": "image/png"})).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let bytes = b"\x89PNG not a real one".to_vec();
    let (s, started) =
        start(json!({"kind": "poster", "titleId": title, "sizeBytes": bytes.len(), "contentType": "image/png"})).await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let upload = started["uploadId"].as_str().unwrap();
    let image: Uuid = started["imageId"].as_str().unwrap().parse().unwrap();
    let res = reqwest::Client::new()
        .put(started["partUrl"].as_str().unwrap())
        .body(bytes.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let etag = res.headers()["etag"].to_str().unwrap().to_owned();
    let (s, done) = env
        .call(
            Method::POST,
            &format!("/api/v1/admin/uploads/{upload}/complete"),
            Some(json!({"parts": [{"partNumber": 1, "etag": etag}]})),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{done}");
    let (poster, kind, job_kind): (Option<Uuid>, String, String) = sqlx::query_as(
        "select t.poster_image_id, i.kind, j.kind from titles t, images i, jobs j
         where t.id = $1::uuid and i.id = $2 and j.id = $3::uuid",
    )
    .bind(&title)
    .bind(image)
    .bind(done["jobId"].as_str().unwrap())
    .fetch_one(&env.state.db)
    .await
    .unwrap();
    assert_eq!(
        (poster, kind.as_str(), job_kind.as_str()),
        (Some(image), "poster", "image")
    );
    env.garage()
        .delete_object(BucketName::Uploads, &format!("images/{image}/source"))
        .await
        .unwrap();

    // An abandoned upload leaves no image behind.
    let (_, other) =
        start(json!({"kind": "backdrop", "titleId": title, "sizeBytes": 10, "contentType": "image/jpeg"})).await;
    let (s, _) = env
        .call(
            Method::DELETE,
            &format!("/api/v1/admin/uploads/{}", other["uploadId"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let images: i64 = sqlx::query_scalar("select count(*) from images")
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert_eq!(images, 1);
}

#[sqlx::test]
async fn subtitles_are_staged_validated_and_removed(pool: PgPool) {
    install();
    let Some(env) = env(pool, &[]).await else { return };
    let (_, variant) = env.movie_variant().await;
    let uri = format!("/api/v1/admin/variants/{variant}/subtitles?language=EN&label=English%20SDH&kind=captions");
    let vtt: &'static [u8] = b"WEBVTT\n\n00:00.000 --> 00:01.000\nHello\n";
    assert_eq!(env.raw(&uri, "application/json", vtt).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        env.raw(&uri, "text/vtt", b"1\n00:00:00,000 --> 00:00:01,000\nSRT\n")
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let (s, created) = env.raw(&uri, "text/vtt; charset=utf-8", vtt).await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    let id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(env.raw(&uri, "text/vtt", vtt).await.0, StatusCode::CONFLICT);

    let key = format!("subtitles/{id}.vtt");
    assert_eq!(
        env.garage().head_object(BucketName::Uploads, &key).await.unwrap(),
        Some(vtt.len() as u64)
    );
    let (language, label, status): (String, String, String) =
        sqlx::query_as("select language, label, status from subtitle_tracks where id = $1")
            .bind(id)
            .fetch_one(&env.state.db)
            .await
            .unwrap();
    assert_eq!(
        (language.as_str(), label.as_str(), status.as_str()),
        ("en", "English SDH", "pending")
    );
    let job: (String, Value) = sqlx::query_as("select status, payload from jobs where kind = 'subtitle'")
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert_eq!(job, ("queued".to_owned(), json!({"subtitleId": id})));

    let (s, _) = env
        .call(Method::DELETE, &format!("/api/v1/admin/subtitles/{id}"), None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(env.garage().head_object(BucketName::Uploads, &key).await.unwrap(), None);
    let status: String = sqlx::query_scalar("select status from jobs where kind = 'subtitle'")
        .fetch_one(&env.state.db)
        .await
        .unwrap();
    assert_eq!(status, "cancelled");
}
