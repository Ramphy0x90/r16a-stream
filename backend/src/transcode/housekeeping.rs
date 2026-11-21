//! Subtitle promotion and encode GC, the worker's small jobs (SPEC 7.2, 7.6, 7.9).
//!
//! Encode GC lists the media bucket one level at a time, compares it with the live set from
//! Postgres, and deletes a prefix only after it was seen orphaned for the grace period and a
//! final check under the variant's row lock still finds it dead. It never deletes a name it
//! cannot parse, and never touches the uploads bucket.

use std::{collections::HashSet, sync::Arc, time::Duration};

use sqlx::{PgPool, postgres::PgListener};
use uuid::Uuid;

use super::repository as repo;
use crate::{
    images::worker::CHANNEL,
    jobs::repository::{self as jobs, Claimed},
    media::dto::{MAX_SUBTITLE_BYTES, is_webvtt},
    storage::{BucketName, GarageClient, StorageError},
};

const POLL: Duration = Duration::from_secs(10);
const HEARTBEAT: Duration = Duration::from_secs(15);
/// Encode GC runs every 6 hours (SPEC 7.2).
pub const GC_EVERY_SECS: f64 = 6.0 * 3600.0;
const OUTPUT_CACHE: &str = "public, max-age=31536000, immutable";

pub struct HousekeepingWorker {
    pub db: PgPool,
    pub storage: Arc<GarageClient>,
    pub instance: String,
    /// How long a prefix stays orphaned before deletion, at least the longest playback token
    /// plus one hour (config validation, SPEC 7.6 step 5).
    pub gc_grace: Duration,
}

impl std::fmt::Debug for HousekeepingWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HousekeepingWorker")
            .field("instance", &self.instance)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Storage,
    Invalid(&'static str, &'static str),
}

/// A media bucket entry encode GC understands, parsed from its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Attempt { variant: Uuid, attempt: Uuid },
    Image(Uuid),
    Subtitle(Uuid),
}

impl Entry {
    /// `v/<variant>/<attempt>/`, `i/<image>/` or `s/<subtitle>.vtt`. Anything else is `None` and
    /// left alone.
    pub fn parse(prefix: &str) -> Option<Self> {
        let parts: Vec<&str> = prefix.split('/').collect();
        match parts.as_slice() {
            ["v", v, a, ""] => Some(Self::Attempt {
                variant: v.parse().ok()?,
                attempt: a.parse().ok()?,
            }),
            ["i", i, ""] => Some(Self::Image(i.parse().ok()?)),
            ["s", file] => Some(Self::Subtitle(file.strip_suffix(".vtt")?.parse().ok()?)),
            _ => None,
        }
    }
}

/// Key of a promoted subtitle in the media bucket (SPEC 10.2).
pub fn subtitle_key(id: Uuid) -> String {
    format!("s/{id}.vtt")
}

impl HousekeepingWorker {
    pub async fn run(self: Arc<Self>) {
        let mut listener = match PgListener::connect_with(&self.db).await {
            Ok(mut l) => l.listen(CHANNEL).await.ok().map(|()| l),
            Err(_) => None,
        };
        loop {
            match self.run_once().await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(e) => tracing::warn!(error = %e, "housekeeping loop error"),
            }
            match listener.as_mut() {
                Some(l) => {
                    let _ = tokio::time::timeout(POLL, l.recv()).await;
                }
                None => tokio::time::sleep(POLL).await,
            }
        }
    }

    pub async fn run_once(&self) -> Result<bool, sqlx::Error> {
        let kinds = [String::from("subtitle"), String::from("encode_gc")];
        let Some(job) = jobs::claim(&self.db, &self.instance, &kinds).await? else {
            return Ok(false);
        };
        let beat = {
            let (db, id, instance) = (self.db.clone(), job.id, self.instance.clone());
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(HEARTBEAT);
                loop {
                    tick.tick().await;
                    let _ = jobs::heartbeat_status(&db, id, &instance, 0.0).await;
                }
            })
        };
        let outcome = if job.kind == "encode_gc" {
            self.gc(&job).await
        } else {
            self.promote(&job).await
        };
        beat.abort();
        let outcome = outcome.unwrap_or_else(|e| {
            tracing::warn!(job = %job.id, error = %e, "housekeeping database error");
            Outcome::Storage
        });
        match outcome {
            Outcome::Done => {}
            Outcome::Storage => {
                let base = (30.0 * 2f64.powi(job.storage_waits.min(5))).min(900.0);
                jobs::requeue_storage(&self.db, job.id, base).await?;
            }
            Outcome::Invalid(code, detail) => {
                let mut tx = self.db.begin().await?;
                jobs::fail_invalid(&mut tx, job.id, code, detail).await?;
                if let Some(id) = subtitle_id(&job) {
                    repo::subtitle_failed(&mut tx, id).await?;
                }
                tx.commit().await?;
            }
        }
        Ok(true)
    }

    /// Copies a staged WebVTT file into the media bucket after checking it again. The API
    /// deletes the staged file once the track is ready or failed.
    async fn promote(&self, job: &Claimed) -> Result<Outcome, sqlx::Error> {
        let Some(id) = subtitle_id(job) else {
            return Ok(Outcome::Invalid("bad-job", "The job does not name a subtitle."));
        };
        let Some(track) = repo::subtitle(&self.db, id).await? else {
            return Ok(Outcome::Invalid("subtitle-missing", "The subtitle track was deleted."));
        };
        if track.status != "pending" {
            let mut tx = self.db.begin().await?;
            jobs::succeed(&mut tx, job.id).await?;
            tx.commit().await?;
            return Ok(Outcome::Done);
        }
        let body = match self.storage.get_object(BucketName::Uploads, &track.staged_key).await {
            Ok((b, _)) => b,
            Err(StorageError::NotFound) => {
                return Ok(Outcome::Invalid(
                    "subtitle-missing",
                    "The subtitle file is no longer stored.",
                ));
            }
            Err(_) => return Ok(Outcome::Storage),
        };
        if body.len() > MAX_SUBTITLE_BYTES || !is_webvtt(&body) {
            return Ok(Outcome::Invalid("invalid-webvtt", "The file is not a WebVTT subtitle."));
        }
        let key = subtitle_key(id);
        if self
            .storage
            .put_object(
                BucketName::Media,
                &key,
                &body,
                "text/vtt; charset=utf-8",
                Some(OUTPUT_CACHE),
            )
            .await
            .is_err()
        {
            return Ok(Outcome::Storage);
        }
        let mut tx = self.db.begin().await?;
        // A track deleted meanwhile leaves an orphan object, which encode GC removes.
        repo::subtitle_ready(&mut tx, id, &key, i32::try_from(body.len()).unwrap_or(i32::MAX)).await?;
        jobs::succeed(&mut tx, job.id).await?;
        tx.commit().await?;
        Ok(Outcome::Done)
    }

    /// Every entry of the media bucket GC understands, as prefixes.
    async fn list_media(&self) -> Result<Vec<String>, StorageError> {
        let mut out = Vec::new();
        for variant in self.storage.list_dirs(BucketName::Media, "v/").await? {
            out.extend(self.storage.list_dirs(BucketName::Media, &variant).await?);
        }
        out.extend(self.storage.list_dirs(BucketName::Media, "i/").await?);
        out.extend(self.storage.list_dirs(BucketName::Media, "s/").await?);
        Ok(out)
    }

    /// One encode GC run (SPEC 7.6), then the next one is queued.
    async fn gc(&self, job: &Claimed) -> Result<Outcome, sqlx::Error> {
        // Listing first, then the live set, so anything started meanwhile counts as live.
        let Ok(listed) = self.list_media().await else {
            return Ok(Outcome::Storage);
        };
        let live = repo::live_set(&self.db).await?;
        let (attempts, images, subtitles): (HashSet<Uuid>, HashSet<Uuid>, HashSet<Uuid>) = (
            live.attempts.into_iter().collect(),
            live.images.into_iter().collect(),
            live.subtitles.into_iter().collect(),
        );
        let orphans: Vec<String> = listed
            .into_iter()
            .filter(|p| match Entry::parse(p) {
                Some(Entry::Attempt { attempt, .. }) => !attempts.contains(&attempt),
                Some(Entry::Image(i)) => !images.contains(&i),
                Some(Entry::Subtitle(s)) => !subtitles.contains(&s),
                None => false,
            })
            .collect();
        repo::record_orphans(&self.db, &orphans).await?;

        let mut deleted = 0usize;
        for prefix in repo::due_orphans(&self.db, self.gc_grace.as_secs_f64()).await? {
            let still_dead = match Entry::parse(&prefix) {
                Some(Entry::Attempt { variant, attempt }) => !repo::attempt_is_live(&self.db, variant, attempt).await?,
                Some(Entry::Image(i)) => !repo::image_exists(&self.db, i).await?,
                Some(Entry::Subtitle(s)) => !repo::subtitle_exists(&self.db, s).await?,
                None => false,
            };
            if still_dead {
                match self.storage.delete_prefix(BucketName::Media, &prefix).await {
                    Ok(n) => deleted += n,
                    Err(e) => {
                        tracing::warn!(error = %e, "encode GC delete failed");
                        return Ok(Outcome::Storage);
                    }
                }
            }
            repo::forget_orphan(&self.db, &prefix).await?;
        }
        repo::finish_gc(&self.db, job.id, GC_EVERY_SECS).await?;
        tracing::info!(orphans = orphans.len(), objects_deleted = deleted, "encode GC done");
        Ok(Outcome::Done)
    }
}

fn subtitle_id(job: &Claimed) -> Option<Uuid> {
    if job.kind != "subtitle" {
        return None;
    }
    job.payload.get("subtitleId")?.as_str()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gc_parses_only_the_layout_it_owns() {
        let (v, a) = (Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(
            Entry::parse(&format!("v/{v}/{a}/")),
            Some(Entry::Attempt { variant: v, attempt: a })
        );
        assert_eq!(Entry::parse(&format!("i/{v}/")), Some(Entry::Image(v)));
        assert_eq!(Entry::parse(&format!("s/{v}.vtt")), Some(Entry::Subtitle(v)));
        for other in [
            "_health/worker-x".to_owned(),
            format!("v/{v}/"),
            format!("v/{v}/not-a-uuid/"),
            format!("v/{v}/{a}/v720/seg_1.m4s"),
            format!("s/{v}.srt"),
            "i/../".to_owned(),
        ] {
            assert_eq!(Entry::parse(&other), None, "{other}");
        }
    }
}
