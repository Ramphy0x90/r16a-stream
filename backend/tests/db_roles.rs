//! Database role boundaries, SPEC 4.9 and DECISIONS 052.
//!
//! Runs against a database migrated by the owner role, the same ownership as production, so
//! the grants in the migrations are what is tested. Set `R16A_TEST_WORKER_DATABASE_URL` and
//! `R16A_TEST_APP_DATABASE_URL`, as CI and the dev compose setup do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use sqlx::{Connection, PgConnection};

async fn connect(var: &str) -> Option<PgConnection> {
    let url = common::integration_env(var)?;
    Some(PgConnection::connect(&url).await.expect("connect"))
}

/// Runs a statement in a transaction that is always rolled back, and returns the error code.
async fn try_sql(conn: &mut PgConnection, sql: &'static str) -> Result<(), String> {
    let mut tx = conn.begin().await.unwrap();
    let res = sqlx::query(sql).execute(&mut *tx).await;
    tx.rollback().await.unwrap();
    res.map(|_| ()).map_err(|e| match e {
        sqlx::Error::Database(d) => d.code().map_or_else(String::new, std::borrow::Cow::into_owned),
        other => other.to_string(),
    })
}

const DENIED: &str = "42501";

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one checklist of every grant, easier to audit in one place"
)]
async fn worker_role_reaches_only_its_grants() {
    let Some(mut c) = connect("R16A_TEST_WORKER_DATABASE_URL").await else {
        return;
    };

    try_sql(
        &mut c,
        "insert into component_health (component, instance, storage_writable) values ('worker', 'role-test', true)
         on conflict (component, instance) do update set storage_writable = excluded.storage_writable",
    )
    .await
    .expect("worker writes its health row");
    try_sql(&mut c, "select count(*) from garage_capacity")
        .await
        .expect("worker reads capacity");
    for (sql, why) in [
        ("select id, kind, status, listed_at from titles", "readiness checks"),
        (
            "update titles set listed_at = now() where false",
            "bumping Recently added",
        ),
        ("select id, title_id, number from seasons", "season numbers"),
        (
            "select id, season_id, title_id, number from episodes",
            "episode numbers",
        ),
        ("select title_id, language from title_languages", "language set"),
        (
            "select id, kind, status, source_key, source_bytes from images",
            "image sources",
        ),
        (
            "update images set status = 'ready', ready_at = now() where false",
            "finishing images",
        ),
        ("delete from image_variants where false", "replacing image variants"),
        (
            "select id, title_id, episode_id, language, status, current_attempt_id from media_variants",
            "variants",
        ),
        (
            "update media_variants set status = 'ready' where false",
            "switching a variant",
        ),
        (
            "insert into transcode_attempts (variant_id, upload_id, encoder, storage_prefix) select null, null, 'x264', 'x' where false",
            "attempts",
        ),
        (
            "insert into renditions (attempt_id, height, width, bandwidth, average_bandwidth, codecs, frame_rate, playlist_key) select null, 360, 640, 1, 1, 'x', 25, 'x' where false",
            "renditions",
        ),
        (
            "select id, purpose, variant_id, image_id, object_key, size_bytes, status, reserved_bytes from uploads",
            "upload sources",
        ),
        (
            "update uploads set reserved_bytes = 0 where false",
            "releasing a reservation",
        ),
        (
            "update subtitle_tracks set status = 'ready' where false",
            "promoting subtitles",
        ),
        ("delete from gc_orphans where false", "encode GC"),
    ] {
        try_sql(&mut c, sql)
            .await
            .unwrap_or_else(|e| panic!("worker needs {why}, got {e}"));
    }

    for (sql, why) in [
        ("delete from component_health", "worker never deletes health rows"),
        (
            "insert into garage_capacity (node, data_avail, data_total, replication) values ('n', 1, 1, 2)",
            "only the API records capacity",
        ),
        ("create table worker_made (id int)", "worker cannot change the schema"),
        ("drop table component_health", "worker cannot drop tables"),
        (
            "create extension if not exists pgcrypto",
            "worker cannot add extensions",
        ),
        (
            "select s3_upload_id from uploads",
            "worker cannot see multipart upload ids",
        ),
        ("select created_by from uploads", "worker cannot see who uploaded"),
        (
            "update uploads set status = 'aborted'",
            "worker cannot change upload state",
        ),
        ("delete from uploads", "worker cannot delete uploads"),
        ("delete from media_variants", "worker cannot delete variants"),
        (
            "update media_variants set language = 'xx'",
            "worker cannot relabel a variant",
        ),
        ("delete from transcode_attempts", "worker cannot delete attempts"),
        ("update renditions set bandwidth = 1", "renditions are written once"),
        (
            "select label from subtitle_tracks",
            "worker cannot read subtitle labels",
        ),
        ("select name from titles", "worker cannot read title text"),
        ("select overview from episodes", "worker cannot read episode text"),
        ("select count(*) from title_credits", "worker cannot read credits"),
        ("select count(*) from genres", "worker cannot read genres"),
        ("select count(*) from tmdb_cache", "worker cannot read the TMDB cache"),
        ("update titles set status = 'published'", "worker cannot publish"),
        ("delete from titles", "worker cannot delete titles"),
        (
            "update images set kind = 'poster'",
            "worker cannot change an image's kind",
        ),
        ("select count(*) from users", "worker cannot read users"),
        ("update users set display_name = 'x'", "worker cannot change users"),
        ("select count(*) from sessions", "worker cannot read sessions"),
        ("delete from sessions", "worker cannot end sessions"),
        ("select count(*) from oidc_flows", "worker cannot read login flows"),
        ("select count(*) from audit_log", "worker cannot read the audit log"),
        (
            "insert into audit_log (action, target_kind) values ('x', 'y')",
            "worker cannot write the audit log",
        ),
    ] {
        assert_eq!(try_sql(&mut c, sql).await, Err(DENIED.into()), "{why}");
    }
}

#[tokio::test]
async fn app_role_cannot_change_the_schema_or_rewrite_the_audit_log() {
    let Some(mut c) = connect("R16A_TEST_APP_DATABASE_URL").await else {
        return;
    };

    try_sql(
        &mut c,
        "insert into garage_capacity (node, data_avail, data_total, replication) values ('role-test', 1, 2, 2)",
    )
    .await
    .expect("app writes rows");
    try_sql(
        &mut c,
        "insert into audit_log (action, target_kind, details) values ('role.test', 'test', '{}')",
    )
    .await
    .expect("app appends to the audit log");
    for sql in [
        "update audit_log set action = 'rewritten'",
        "delete from audit_log",
        "truncate audit_log",
        "create table app_made (id int)",
        "alter table component_health add column x int",
        "drop table garage_capacity",
    ] {
        assert_eq!(try_sql(&mut c, sql).await, Err(DENIED.into()), "{sql}");
    }
}
