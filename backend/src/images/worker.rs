//! The worker's image pipeline (SPEC 8.2 steps 3 and 4).

use std::{path::PathBuf, sync::Arc, time::Duration};

use sqlx::{PgPool, postgres::PgListener};
use uuid::Uuid;

use super::{
    MAX_PIXELS, MAX_SIDE,
    ffmpeg::{self, Format, Scratch, ToolError},
    hsl,
    repository::{self as repo, NewVariant},
    variant_key, widths,
};
use crate::{
    jobs::repository::{self as jobs, Claimed},
    storage::{BucketName, GarageClient, StorageError},
};

/// Channel the API notifies when it queues a job. Polling is the fallback (SPEC 7.2).
pub const CHANNEL: &str = "r16a_stream_jobs";
const POLL: Duration = Duration::from_secs(10);
const OUTPUT_CACHE: &str = "public, max-age=31536000, immutable";

pub struct ImageWorker {
    pub db: PgPool,
    pub storage: Arc<GarageClient>,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub scratch_root: PathBuf,
    pub instance: String,
}

impl std::fmt::Debug for ImageWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageWorker")
            .field("instance", &self.instance)
            .finish_non_exhaustive()
    }
}

/// How a job ended, decided by the failure classes of SPEC 7.2.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Garage could not be read or written. Requeued, not a failure.
    Storage,
    /// The file is not an image we accept. Failed, never retried.
    Invalid(&'static str),
    /// Something that may work next time, such as an encoder crash or a timeout.
    Transient(&'static str),
}

impl ImageWorker {
    /// Claims and runs image jobs until the process stops. Wakes on `NOTIFY` or every 10 s.
    pub async fn run(self: Arc<Self>) {
        let mut listener = match PgListener::connect_with(&self.db).await {
            Ok(mut l) => match l.listen(CHANNEL).await {
                Ok(()) => Some(l),
                Err(e) => {
                    tracing::warn!(error = %e, "LISTEN failed, polling only");
                    None
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "job listener unavailable, polling only");
                None
            }
        };
        loop {
            match self.run_once().await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(e) => tracing::warn!(error = %e, "image job loop error"),
            }
            match listener.as_mut() {
                Some(l) => {
                    let _ = tokio::time::timeout(POLL, l.recv()).await;
                }
                None => tokio::time::sleep(POLL).await,
            }
        }
    }

    /// Claims one image job and runs it. Returns whether there was one.
    pub async fn run_once(&self) -> Result<bool, sqlx::Error> {
        let Some(job) = jobs::claim(&self.db, &self.instance, &[String::from("image")]).await? else {
            return Ok(false);
        };
        let outcome = self.process(&job).await;
        self.settle(&job, outcome).await?;
        Ok(true)
    }

    async fn settle(&self, job: &Claimed, outcome: Result<Outcome, sqlx::Error>) -> Result<(), sqlx::Error> {
        let outcome = outcome.unwrap_or_else(|e| {
            tracing::warn!(job = %job.id, error = %e, "image job database error");
            Outcome::Transient("database")
        });
        match outcome {
            Outcome::Done => {}
            Outcome::Storage => {
                // 30 s doubling to 15 min, with jitter so workers do not retry in step.
                let base = (30.0 * 2f64.powi(job.storage_waits.min(5))).min(900.0);
                let jitter = f64::from(Uuid::new_v4().as_bytes()[0]) / 255.0 * 10.0;
                jobs::requeue_storage(&self.db, job.id, base + jitter).await?;
                tracing::info!(job = %job.id, "image job waits for storage");
            }
            Outcome::Invalid(code) => {
                let mut tx = self.db.begin().await?;
                jobs::fail_invalid(&mut tx, job.id, code, invalid_detail(code)).await?;
                if let Some(image) = job.image_id {
                    repo::mark_failed(&mut tx, image).await?;
                }
                tx.commit().await?;
                tracing::info!(job = %job.id, code, "image refused");
            }
            Outcome::Transient(code) => {
                let delay = [60.0, 300.0, 1500.0][usize::try_from(job.attempts).unwrap_or(0).min(2)];
                let mut tx = self.db.begin().await?;
                let dead =
                    jobs::fail_transient(&mut tx, job.id, code, "the image could not be processed", delay).await?;
                if dead && let Some(image) = job.image_id {
                    repo::mark_failed(&mut tx, image).await?;
                }
                tx.commit().await?;
                tracing::warn!(job = %job.id, code, final_attempt = dead, "image job failed");
            }
        }
        Ok(())
    }

    async fn process(&self, job: &Claimed) -> Result<Outcome, sqlx::Error> {
        let Some(image_id) = job.image_id else {
            return Ok(Outcome::Invalid("no-image"));
        };
        let Some(source) = repo::source(&self.db, image_id).await? else {
            return Ok(Outcome::Invalid("no-image"));
        };
        let scratch = match Scratch::create(&self.scratch_root, job.id).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "scratch directory unusable");
                return Ok(Outcome::Transient("scratch"));
            }
        };
        let outcome = self.convert(job, image_id, &source, &scratch).await;
        scratch.remove().await;
        outcome
    }

    async fn convert(
        &self,
        job: &Claimed,
        image_id: Uuid,
        source: &repo::ImageSource,
        scratch: &Scratch,
    ) -> Result<Outcome, sqlx::Error> {
        if source
            .source_bytes
            .is_some_and(|b| usize::try_from(b).map_or(true, |b| b > super::ingest::MAX_BYTES))
        {
            return Ok(Outcome::Invalid("too-large"));
        }
        let bytes = match self.storage.get_object(BucketName::Uploads, &source.source_key).await {
            Ok((b, _)) => b,
            Err(StorageError::NotFound) => return Ok(Outcome::Invalid("source-missing")),
            Err(e) if e.is_retryable() => return Ok(Outcome::Storage),
            Err(e) => {
                tracing::warn!(error = %e, "image source read refused");
                return Ok(Outcome::Transient("source-read"));
            }
        };
        if bytes.len() > super::ingest::MAX_BYTES {
            return Ok(Outcome::Invalid("too-large"));
        }
        let input = scratch.path("source");
        if tokio::fs::write(&input, &bytes).await.is_err() {
            return Ok(Outcome::Transient("scratch"));
        }

        // Probe before any decode, so a decompression bomb never gets decoded.
        let (width, height) = match ffmpeg::run(&self.ffprobe, "ffprobe", &ffmpeg::probe_args(&input)).await {
            Ok(out) => match parse_probe(&out) {
                Some(size) => size,
                None => return Ok(Outcome::Invalid("not-an-image")),
            },
            Err(ToolError::Failed { stderr, .. }) => {
                tracing::debug!(%stderr, "ffprobe refused the image");
                return Ok(Outcome::Invalid("not-an-image"));
            }
            Err(e) => {
                tracing::warn!(error = %e, "ffprobe did not finish");
                return Ok(Outcome::Transient("probe"));
            }
        };
        if width > MAX_SIDE || height > MAX_SIDE || u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Ok(Outcome::Invalid("too-large"));
        }
        jobs::heartbeat(&self.db, job.id, 0.1).await?;

        let avg = scratch.path("average.rgb");
        let placeholder = match ffmpeg::run(&self.ffmpeg, "ffmpeg", &ffmpeg::average_args(&input, &avg)).await {
            Ok(_) => match tokio::fs::read(&avg).await {
                Ok(px) if px.len() == 3 => hsl(px[0], px[1], px[2]),
                _ => return Ok(Outcome::Transient("encode")),
            },
            Err(e) => return Ok(tool_failure(&e)),
        };

        let variants = match self
            .encode_all(job, image_id, &source.kind, (width, height), &input, scratch)
            .await?
        {
            Ok(v) => v,
            Err(outcome) => return Ok(outcome),
        };

        let mut tx = self.db.begin().await?;
        repo::finish(
            &mut tx,
            image_id,
            i32::try_from(width).unwrap_or(i32::MAX),
            i32::try_from(height).unwrap_or(i32::MAX),
            &placeholder,
            &variants,
        )
        .await?;
        jobs::succeed(&mut tx, job.id).await?;
        tx.commit().await?;
        tracing::info!(job = %job.id, image = %image_id, variants = variants.len(), "image ready");
        Ok(Outcome::Done)
    }
    /// Encodes and uploads every width in both formats. `Err` carries how the job ends.
    async fn encode_all(
        &self,
        job: &Claimed,
        image_id: Uuid,
        kind: &str,
        (width, height): (u32, u32),
        input: &std::path::Path,
        scratch: &Scratch,
    ) -> Result<Result<Vec<NewVariant>, Outcome>, sqlx::Error> {
        let rungs = widths(kind, width);
        let total = rungs.len() * 2;
        let mut variants = Vec::with_capacity(total);
        for (i, w) in rungs.iter().copied().enumerate() {
            for (j, format) in [Format::Avif, Format::Webp].into_iter().enumerate() {
                let out = scratch.path(&format!("{w}.{}", format.ext()));
                if let Err(e) = ffmpeg::run(&self.ffmpeg, "ffmpeg", &ffmpeg::encode_args(input, &out, w, format)).await
                {
                    return Ok(Err(tool_failure(&e)));
                }
                let Ok(data) = tokio::fs::read(&out).await else {
                    return Ok(Err(Outcome::Transient("encode")));
                };
                let key = variant_key(image_id, w, format.ext());
                match self
                    .storage
                    .put_object(
                        BucketName::Media,
                        &key,
                        &data,
                        format.content_type(),
                        Some(OUTPUT_CACHE),
                    )
                    .await
                {
                    Ok(()) => {}
                    Err(e) if e.is_retryable() => return Ok(Err(Outcome::Storage)),
                    Err(e) => {
                        tracing::warn!(error = %e, "image output write refused");
                        return Ok(Err(Outcome::Transient("output-write")));
                    }
                }
                variants.push(NewVariant {
                    format: format.ext(),
                    width: i16::try_from(w).unwrap_or(i16::MAX),
                    height: scaled_height(width, height, w),
                    object_key: key,
                    byte_size: i32::try_from(data.len()).unwrap_or(i32::MAX),
                });
                #[allow(clippy::cast_precision_loss)]
                let progress = 0.1 + 0.9 * ((i * 2 + j + 1) as f32 / total as f32);
                jobs::heartbeat(&self.db, job.id, progress.min(0.99)).await?;
            }
        }

        Ok(Ok(variants))
    }
}

fn tool_failure(e: &ToolError) -> Outcome {
    match e {
        ToolError::Failed { stderr, .. } => {
            tracing::debug!(%stderr, "ffmpeg failed");
            Outcome::Transient("encode")
        }
        ToolError::Timeout(_) => Outcome::Transient("timeout"),
        ToolError::Spawn(_) => Outcome::Transient("spawn"),
    }
}

/// Fixed texts for the admin, never tool output, which can contain paths.
fn invalid_detail(code: &str) -> &'static str {
    match code {
        "too-large" => "the image is larger than 16384 px a side, 40 megapixels or 20 MiB",
        "not-an-image" => "the file is not a JPEG, PNG or WebP image",
        "source-missing" => "the uploaded source is gone",
        _ => "the image record is missing",
    }
}

/// Width and height of the first video stream from ffprobe's JSON.
pub fn parse_probe(out: &[u8]) -> Option<(u32, u32)> {
    let v: serde_json::Value = serde_json::from_slice(out).ok()?;
    let s = v["streams"].as_array()?.first()?;
    let w = u32::try_from(s["width"].as_u64()?).ok()?;
    let h = u32::try_from(s["height"].as_u64()?).ok()?;
    (w > 0 && h > 0).then_some((w, h))
}

/// The height `scale=w:-2` produces. ffmpeg computes `av_rescale(w, in_h, in_w * 2) * 2`, which
/// rounds half up, so the result keeps the aspect ratio and is even.
pub fn scaled_height(src_w: u32, src_h: u32, w: u32) -> i16 {
    let den = 2 * u64::from(src_w.max(1));
    let half = (u64::from(w) * u64::from(src_h) + den / 2) / den;
    i16::try_from((half * 2).clamp(2, 4096)).unwrap_or(4096)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_output_parses_and_refuses_junk() {
        assert_eq!(
            parse_probe(br#"{"streams":[{"width":2000,"height":3000}]}"#),
            Some((2000, 3000))
        );
        assert_eq!(parse_probe(br#"{"streams":[]}"#), None);
        assert_eq!(parse_probe(br#"{"streams":[{"width":0,"height":3}]}"#), None);
        assert_eq!(parse_probe(b"not json"), None);
    }

    #[test]
    fn scaled_heights_keep_the_aspect_and_stay_even() {
        assert_eq!(scaled_height(2000, 3000, 342), 514);
        assert_eq!(scaled_height(1920, 1080, 780), 438);
        assert_eq!(scaled_height(100, 1, 100), 2);
    }
}
