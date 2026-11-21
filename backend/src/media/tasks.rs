//! Background tasks of the API: capacity polling, stale upload cleanup and source retention.
//!
//! They run in every API replica. Polling is idempotent, cleanup works row by row, and
//! retention takes a session advisory lock so only one replica deletes sources at a time.

use std::{sync::Arc, time::Duration};

use serde_json::json;
use sqlx::PgPool;

use super::repository as repo;
use crate::{
    admin::audit,
    capacity,
    config::{Retention, StoragePolicy},
    storage::{BucketName, GarageClient, StorageError},
};

const CLEANUP_EVERY: Duration = Duration::from_mins(15);
const RETENTION_EVERY: Duration = Duration::from_mins(5);

pub fn spawn(db: PgPool, storage: Arc<GarageClient>, policy: Arc<crate::config::ApiConfig>, http: reqwest::Client) {
    {
        let (db, policy) = (db.clone(), policy.clone());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(capacity::POLL);
            loop {
                tick.tick().await;
                capacity::poll_once(&db, &http, &policy.storage).await;
            }
        });
    }
    {
        let (db, storage) = (db.clone(), storage.clone());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(CLEANUP_EVERY);
            loop {
                tick.tick().await;
                if let Err(e) = cleanup_stale_uploads(&db, &storage).await {
                    tracing::warn!(error = %e, "stale upload cleanup failed");
                }
                if let Err(e) = cleanup_staged_subtitles(&db, &storage).await {
                    tracing::warn!(error = %e, "staged subtitle cleanup failed");
                }
            }
        });
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(RETENTION_EVERY);
        loop {
            tick.tick().await;
            if let Err(e) = retention_round(&db, &storage, &policy.storage).await {
                tracing::warn!(error = %e, "source retention failed");
            }
        }
    });
}

/// Aborts initiated uploads past their 24 hours (SPEC 7.1). A storage failure leaves the row
/// for the next round.
pub async fn cleanup_stale_uploads(db: &PgPool, storage: &GarageClient) -> Result<usize, sqlx::Error> {
    let mut done = 0;
    for u in repo::stale_uploads(db).await? {
        match storage
            .abort_multipart(BucketName::Uploads, &u.object_key, &u.s3_upload_id)
            .await
        {
            Ok(()) | Err(StorageError::NotFound) => {}
            Err(e) => {
                tracing::warn!(error = %e, "aborting a stale upload failed");
                continue;
            }
        }
        let mut tx = db.begin().await?;
        repo::mark_aborted(&mut tx, u.id).await?;
        audit::record_system(&mut tx, "upload.expire", "upload", &u.id.to_string(), json!({})).await?;
        tx.commit().await?;
        done += 1;
    }
    Ok(done)
}

/// Deletes staged subtitle files the worker no longer needs (SPEC 7.9). The worker key cannot
/// write the uploads bucket, so this runs in the API.
pub async fn cleanup_staged_subtitles(db: &PgPool, storage: &GarageClient) -> Result<usize, sqlx::Error> {
    let mut done = 0;
    for t in repo::staged_subtitles(db).await? {
        match storage.delete_object(BucketName::Uploads, &t.staged_key).await {
            Ok(()) | Err(StorageError::NotFound) => {}
            Err(e) => {
                tracing::warn!(error = %e, "deleting a staged subtitle failed");
                continue;
            }
        }
        repo::mark_staged_removed(db, t.id).await?;
        done += 1;
    }
    Ok(done)
}

/// One retention round (SPEC 7.9). Returns the sources deleted, or `None` when another replica
/// holds the lock or retention is `keep`.
pub async fn retention_round(
    db: &PgPool,
    storage: &GarageClient,
    policy: &StoragePolicy,
) -> Result<Option<usize>, sqlx::Error> {
    if policy.retention == Retention::Keep {
        return Ok(None);
    }
    // The lock lives on this connection, so it is held and released on the same one.
    let mut lock = db.acquire().await?;
    if !repo::try_retention_lock(&mut lock).await? {
        return Ok(None);
    }
    let result = delete_sources(db, storage, policy).await;
    repo::release_retention_lock(&mut lock).await?;
    result.map(Some)
}

async fn delete_sources(db: &PgPool, storage: &GarageClient, policy: &StoragePolicy) -> Result<usize, sqlx::Error> {
    let mut done = 0;
    let delay = policy.source_delete_delay.as_secs_f64();
    for c in repo::retention_candidates(db, delay).await? {
        // The row lock is held across one delete call, a short wait for a concurrent
        // re-transcode and the only way to make the check and the delete one step.
        let mut tx = db.begin().await?;
        if !repo::lock_retention_candidate(&mut tx, c.id, delay).await? {
            continue;
        }
        match storage.delete_object(BucketName::Uploads, &c.object_key).await {
            Ok(()) | Err(StorageError::NotFound) => {}
            Err(e) => {
                tracing::warn!(error = %e, "deleting a source failed, retried next round");
                continue;
            }
        }
        repo::mark_source_deleted(&mut tx, c.id).await?;
        audit::record_system(
            &mut tx,
            "source.delete",
            "upload",
            &c.id.to_string(),
            json!({ "variantId": c.variant_id }),
        )
        .await?;
        tx.commit().await?;
        done += 1;
    }
    Ok(done)
}
