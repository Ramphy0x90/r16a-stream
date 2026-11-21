//! The transcode pipeline of SPEC 7.3, from a claimed job to a switched variant.
//!
//! Every step maps its failures to the classes of SPEC 7.2. Storage problems requeue and never
//! count, invalid sources fail at once, everything else is transient. The scratch directory is
//! removed in every outcome, and an attempt that does not become current is marked failed or
//! abandoned with its objects left for encode GC.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use sqlx::{PgPool, postgres::PgListener};
use tokio::{sync::watch, task::JoinSet};
use uuid::Uuid;

use super::{
    ffmpeg::{self, Refusal, Rung, Source},
    repository::{self as repo, RenditionRow, Switch, Switched},
    runner::{self, RunEnd},
};
use crate::{
    config::Encoder,
    images::{
        ffmpeg::{Scratch, ToolError, run as run_tool},
        worker::CHANNEL,
    },
    jobs::repository::{self as jobs, Claimed},
    storage::{BucketName, GarageClient, StorageError},
};

const POLL: Duration = Duration::from_secs(10);
const HEARTBEAT: Duration = Duration::from_secs(15);
const OUTPUT_CACHE: &str = "public, max-age=31536000, immutable";
/// Largest source the worker downloads, as the API admits (SPEC 4.5).
const MAX_SOURCE: u64 = 64_424_509_440;
const UPLOAD_PARALLELISM: usize = 8;
const PUT_TRIES: u32 = 3;

pub struct TranscodeWorker {
    pub db: PgPool,
    pub storage: Arc<GarageClient>,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub scratch_root: PathBuf,
    pub instance: String,
    /// The encoder after the start-up check, x264 when a hardware backend failed it.
    pub encoder: Encoder,
    pub scratch_reserve: u64,
    pub min_free: u64,
    /// Scratch bytes promised to jobs running in this process (SPEC 7.3 step 1).
    pub scratch_in_use: Arc<AtomicU64>,
}

impl std::fmt::Debug for TranscodeWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranscodeWorker")
            .field("instance", &self.instance)
            .field("encoder", &self.encoder)
            .finish_non_exhaustive()
    }
}

/// How a run ended, decided by the failure classes of SPEC 7.2.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Garage could not be read or written.
    Storage,
    /// Waits for space, `scratch_full` or `storage_full`, with the retry delay in seconds.
    Blocked(&'static str, u32),
    /// The source will never work. Code and the text shown to the admin.
    Invalid(&'static str, &'static str),
    Transient(&'static str),
    Cancelled,
    /// The job was taken away by the reaper, or its variant deleted. Nothing to settle.
    Lost,
}

/// What `settle` needs to know beyond the outcome.
#[derive(Debug, Default)]
struct Ctx {
    attempt: Option<Uuid>,
    upload: Option<Uuid>,
}

/// Returns the reserved scratch bytes when a job ends, however it ends.
struct ScratchClaim {
    counter: Arc<AtomicU64>,
    bytes: u64,
}

impl Drop for ScratchClaim {
    fn drop(&mut self) {
        self.counter.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

/// Free bytes of the file system holding `path`, for unprivileged users.
pub fn free_bytes(path: &Path) -> Option<u64> {
    let s = rustix::fs::statvfs(path).ok()?;
    Some(s.f_bavail.saturating_mul(s.f_frsize))
}

/// Logical free space from fresh capacity rows, `None` when they are missing or older than
/// 10 minutes, in which case the source reservation covers the outputs (SPEC 7.3 step 7).
pub fn logical_free(rows: &[repo::CapacityRow]) -> Option<u64> {
    if rows.is_empty() || rows.iter().any(|r| !r.recent) {
        return None;
    }
    let min = rows.iter().map(|r| u64::try_from(r.data_avail).unwrap_or(0)).min()?;
    let replication = u64::try_from(rows.iter().map(|r| r.replication).max()?.max(1)).unwrap_or(1);
    Some(min.saturating_mul(rows.len() as u64) / replication)
}

fn invalid(r: Refusal) -> Outcome {
    Outcome::Invalid(r.code(), r.detail())
}

fn storage(e: &StorageError, what: &'static str) -> Outcome {
    match e {
        StorageError::NotFound => Outcome::Invalid("source-missing", "The uploaded file is no longer stored."),
        e if e.is_retryable() => Outcome::Storage,
        e => {
            tracing::warn!(error = %e, what, "storage refused the worker");
            Outcome::Storage
        }
    }
}

/// Every file under `dir` as its path relative to it, with its size.
async fn list_files(dir: &Path) -> std::io::Result<BTreeMap<String, u64>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&d).await?;
        while let Some(e) = entries.next_entry().await? {
            let meta = e.metadata().await?;
            if meta.is_dir() {
                stack.push(e.path());
            } else if meta.is_file() {
                let rel = e
                    .path()
                    .strip_prefix(dir)
                    .map_or_else(|_| String::new(), |p| p.to_string_lossy().into_owned());
                out.insert(rel, meta.len());
            }
        }
    }
    Ok(out)
}

/// Peak and average bits per second of a rung from its playlist and segment sizes.
pub fn bandwidths(segments: &[(f64, String)], sizes: &BTreeMap<String, u64>, dir: &str) -> Option<(i32, i32)> {
    let mut peak = 0f64;
    let mut bits = 0f64;
    let mut seconds = 0f64;
    for (d, name) in segments {
        #[allow(clippy::cast_precision_loss, reason = "segment sizes are a few MB")]
        let b = *sizes.get(&format!("{dir}/{name}"))? as f64 * 8.0;
        if *d > 0.0 {
            peak = peak.max(b / d);
        }
        bits += b;
        seconds += d;
    }
    if seconds <= 0.0 {
        return None;
    }
    let to_i32 = |v: f64| {
        #[allow(clippy::cast_possible_truncation, reason = "checked against i32 range")]
        let n = v.ceil().clamp(1.0, f64::from(i32::MAX)) as i32;
        n
    };
    Some((to_i32(peak), to_i32(bits / seconds)))
}

impl TranscodeWorker {
    pub async fn run(self: Arc<Self>) {
        let mut listener = match PgListener::connect_with(&self.db).await {
            Ok(mut l) => l.listen(CHANNEL).await.ok().map(|()| l),
            Err(_) => None,
        };
        if listener.is_none() {
            tracing::warn!("job listener unavailable, transcode polling only");
        }
        loop {
            match self.run_once().await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(e) => tracing::warn!(error = %e, "transcode loop error"),
            }
            match listener.as_mut() {
                Some(l) => {
                    let _ = tokio::time::timeout(POLL, l.recv()).await;
                }
                None => tokio::time::sleep(POLL).await,
            }
        }
    }

    /// Claims one transcode job and runs it. Returns whether there was one.
    pub async fn run_once(&self) -> Result<bool, sqlx::Error> {
        let Some(job) = jobs::claim(&self.db, &self.instance, &[String::from("transcode")]).await? else {
            return Ok(false);
        };
        tracing::info!(job = %job.id, "transcode started");
        let mut ctx = Ctx::default();
        let outcome = self.process(&job, &mut ctx).await;
        self.settle(&job, &ctx, outcome).await?;
        Ok(true)
    }

    async fn settle(&self, job: &Claimed, ctx: &Ctx, outcome: Result<Outcome, sqlx::Error>) -> Result<(), sqlx::Error> {
        let outcome = outcome.unwrap_or_else(|e| {
            tracing::warn!(job = %job.id, error = %e, "transcode database error");
            Outcome::Transient("database")
        });
        let variant = job.variant_id;
        let mut tx = self.db.begin().await?;
        match &outcome {
            Outcome::Done => {}
            Outcome::Lost => {
                if let Some(a) = ctx.attempt {
                    repo::end_attempt(&mut tx, a, "abandoned").await?;
                }
            }
            Outcome::Storage => {
                if let Some(a) = ctx.attempt {
                    repo::end_attempt(&mut tx, a, "abandoned").await?;
                }
                tx.commit().await?;
                let base = (30.0 * 2f64.powi(job.storage_waits.min(5))).min(900.0);
                let jitter = f64::from(Uuid::new_v4().as_bytes()[0]) / 255.0 * 10.0;
                jobs::requeue_storage(&self.db, job.id, base + jitter).await?;
                tracing::info!(job = %job.id, "transcode waits for storage");
                return Ok(());
            }
            Outcome::Blocked(reason, delay) => {
                if let Some(a) = ctx.attempt {
                    repo::end_attempt(&mut tx, a, "abandoned").await?;
                }
                tx.commit().await?;
                jobs::requeue_blocked(&self.db, job.id, reason, f64::from(*delay)).await?;
                tracing::info!(job = %job.id, reason, "transcode waits for space");
                return Ok(());
            }
            Outcome::Invalid(code, detail) => {
                jobs::fail_invalid(&mut tx, job.id, code, detail).await?;
                if let Some(a) = ctx.attempt {
                    repo::end_attempt(&mut tx, a, "failed").await?;
                }
                if let Some(v) = variant {
                    repo::release(&mut tx, v, ctx.upload).await?;
                }
                tracing::info!(job = %job.id, code, "source refused");
            }
            Outcome::Transient(code) => {
                let delay = [60.0, 300.0, 1500.0][usize::try_from(job.attempts).unwrap_or(0).min(2)];
                let dead =
                    jobs::fail_transient(&mut tx, job.id, code, "The video could not be encoded.", delay).await?;
                if let Some(a) = ctx.attempt {
                    repo::end_attempt(&mut tx, a, "failed").await?;
                }
                if dead && let Some(v) = variant {
                    repo::release(&mut tx, v, ctx.upload).await?;
                }
                tracing::warn!(job = %job.id, code, final_attempt = dead, "transcode failed");
            }
            Outcome::Cancelled => {
                jobs::cancelled(&mut tx, job.id).await?;
                if let Some(a) = ctx.attempt {
                    repo::end_attempt(&mut tx, a, "abandoned").await?;
                }
                if let Some(v) = variant {
                    repo::release(&mut tx, v, ctx.upload).await?;
                }
                tracing::info!(job = %job.id, "transcode cancelled");
            }
        }
        tx.commit().await?;
        if outcome == Outcome::Done {
            tracing::info!(job = %job.id, "transcode done");
        }
        Ok(())
    }

    async fn process(&self, job: &Claimed, ctx: &mut Ctx) -> Result<Outcome, sqlx::Error> {
        let upload_id = job
            .payload
            .get("uploadId")
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse::<Uuid>().ok());
        let (Some(variant_id), Some(upload_id)) = (job.variant_id, upload_id) else {
            return Ok(Outcome::Invalid("bad-job", "The job does not name a source."));
        };
        ctx.upload = Some(upload_id);
        let Some(variant) = repo::variant(&self.db, variant_id).await? else {
            return Ok(Outcome::Lost);
        };
        let source = match repo::source(&self.db, upload_id).await? {
            Some(s) if s.variant_id == Some(variant_id) && s.status == "completed" => s,
            _ => {
                return Ok(Outcome::Invalid(
                    "source-missing",
                    "The uploaded file is no longer stored.",
                ));
            }
        };
        let size = u64::try_from(source.size_bytes).unwrap_or(u64::MAX);

        // Step 1, scratch space for the source copy, the outputs and temporaries.
        let need = size.saturating_mul(8) / 5;
        let free = free_bytes(&self.scratch_root).unwrap_or(0);
        let in_use = self.scratch_in_use.load(Ordering::Relaxed);
        if free < need.saturating_add(self.scratch_reserve).saturating_add(in_use) {
            return Ok(Outcome::Blocked("scratch_full", 300));
        }
        self.scratch_in_use.fetch_add(need, Ordering::Relaxed);
        let _claim = ScratchClaim {
            counter: self.scratch_in_use.clone(),
            bytes: need,
        };
        let scratch = match Scratch::create(&self.scratch_root, job.id).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "scratch directory unusable");
                return Ok(Outcome::Transient("scratch"));
            }
        };

        // Heartbeat with progress, and the cancel flag it reads back.
        let (progress_tx, progress_rx) = watch::channel(0f32);
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let lost = Arc::new(AtomicBool::new(false));
        let beat = {
            let (db, id, instance, lost) = (self.db.clone(), job.id, self.instance.clone(), lost.clone());
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(HEARTBEAT);
                loop {
                    tick.tick().await;
                    let p = *progress_rx.borrow();
                    match jobs::heartbeat_status(&db, id, &instance, p).await {
                        Ok(Some(s)) if s == "cancelling" => {
                            cancel_tx.send_replace(true);
                        }
                        Ok(Some(_)) => {}
                        Ok(None) => {
                            lost.store(true, Ordering::Relaxed);
                            cancel_tx.send_replace(true);
                        }
                        Err(e) => tracing::warn!(error = %e, "heartbeat failed"),
                    }
                }
            })
        };
        let result = self
            .pipeline(job, ctx, &variant, &source, &scratch, &progress_tx, cancel_rx)
            .await;
        beat.abort();
        scratch.remove().await;
        let outcome = result?;
        Ok(match outcome {
            Outcome::Cancelled if lost.load(Ordering::Relaxed) => Outcome::Lost,
            o => o,
        })
    }

    #[allow(clippy::too_many_arguments, reason = "one pipeline, its context passed through once")]
    #[allow(
        clippy::too_many_lines,
        reason = "the steps of SPEC 7.3 in order, easier to audit together"
    )]
    async fn pipeline(
        &self,
        job: &Claimed,
        ctx: &mut Ctx,
        variant: &repo::VariantRow,
        source: &repo::SourceRow,
        scratch: &Scratch,
        progress: &watch::Sender<f32>,
        cancel: watch::Receiver<bool>,
    ) -> Result<Outcome, sqlx::Error> {
        let cancelled = || *cancel.borrow();

        // Step 2, download.
        let declared = u64::try_from(source.size_bytes).unwrap_or(u64::MAX);
        match self.storage.head_object(BucketName::Uploads, &source.object_key).await {
            Ok(Some(n)) if n <= MAX_SOURCE && n == declared => {}
            Ok(Some(_)) => {
                return Ok(Outcome::Invalid(
                    "source-size",
                    "The stored file has an unexpected size.",
                ));
            }
            Ok(None) => {
                return Ok(Outcome::Invalid(
                    "source-missing",
                    "The uploaded file is no longer stored.",
                ));
            }
            Err(e) => return Ok(storage(&e, "source head")),
        }
        let input = scratch.path("source");
        if let Err(e) = self
            .storage
            .get_to_file(BucketName::Uploads, &source.object_key, &input, declared)
            .await
        {
            return Ok(storage(&e, "source download"));
        }
        if cancelled() {
            return Ok(Outcome::Cancelled);
        }

        // Step 3, probe.
        let probe = match run_tool(&self.ffprobe, "ffprobe", &ffmpeg::probe_args(&input)).await {
            Ok(out) => out,
            Err(ToolError::Failed { .. }) => return Ok(invalid(Refusal::Unreadable)),
            Err(ToolError::Timeout(_)) => return Ok(Outcome::Transient("probe-timeout")),
            Err(ToolError::Spawn(_)) => return Ok(Outcome::Transient("ffprobe-missing")),
        };
        let src: Source = match ffmpeg::parse_probe(&probe, &variant.language) {
            Ok(s) => s,
            Err(r) => return Ok(invalid(r)),
        };
        let rungs: Vec<Rung> = ffmpeg::ladder(src.display_width, src.display_height);

        // Step 4, attempt.
        let attempt = Uuid::new_v4();
        let prefix = format!("v/{}/{attempt}/", variant.id);
        repo::insert_attempt(
            &self.db,
            attempt,
            variant.id,
            source.id,
            job.id,
            self.encoder.as_str(),
            &prefix,
        )
        .await?;
        ctx.attempt = Some(attempt);

        // Steps 5 and 6, encode with sprites.
        let out = scratch.path("out");
        for d in rungs.iter().map(Rung::name).chain(["a".into(), "scrub".into()]) {
            if tokio::fs::create_dir_all(out.join(d)).await.is_err() {
                return Ok(Outcome::Transient("scratch"));
            }
        }
        let secs = src.duration_ms / 1000;
        let limit = match self.encoder {
            Encoder::X264 => Duration::from_secs((secs * 4).max(3600)),
            Encoder::Qsv | Encoder::Vaapi => Duration::from_secs(secs.max(1800)),
        };
        let args = ffmpeg::encode_args(self.encoder, &input, &out, &src, &rungs);
        match runner::run(&self.ffmpeg, &args, limit, src.duration_ms, progress, cancel.clone()).await {
            RunEnd::Done => {}
            RunEnd::Cancelled => return Ok(Outcome::Cancelled),
            RunEnd::Timeout => return Ok(Outcome::Transient("encode-timeout")),
            RunEnd::Spawn => return Ok(Outcome::Transient("ffmpeg-missing")),
            RunEnd::Failed { code, stderr } => {
                tracing::warn!(job = %job.id, ?code, stderr, "encode failed");
                return Ok(Outcome::Transient("encode-failed"));
            }
        }
        let (sw, sh) = (
            ffmpeg::SPRITE_WIDTH,
            ffmpeg::sprite_height(src.display_width, src.display_height),
        );
        if tokio::fs::write(
            out.join("scrub").join("scrub.vtt"),
            ffmpeg::scrub_vtt(src.duration_ms, sw, sh),
        )
        .await
        .is_err()
        {
            return Ok(Outcome::Transient("scratch"));
        }

        // Outputs and renditions from the local files.
        let Ok(files) = list_files(&out).await else {
            return Ok(Outcome::Transient("scratch"));
        };
        if files
            .keys()
            .any(|k| ffmpeg::content_type(k, k.starts_with("a/")).is_none())
        {
            return Ok(Outcome::Transient("unexpected-output"));
        }
        let total: u64 = files.values().sum();
        let mut renditions = Vec::new();
        for r in &rungs {
            let name = r.name();
            let text = tokio::fs::read_to_string(out.join(&name).join("index.m3u8"))
                .await
                .unwrap_or_default();
            let Some((_, segments)) = ffmpeg::parse_media_playlist(&text) else {
                return Ok(Outcome::Transient("bad-playlist"));
            };
            let Some((peak, avg)) = bandwidths(&segments, &files, &name) else {
                return Ok(Outcome::Transient("bad-playlist"));
            };
            renditions.push(RenditionRow {
                height: i16::try_from(r.height).unwrap_or(i16::MAX),
                width: i16::try_from(r.width).unwrap_or(i16::MAX),
                bandwidth: peak,
                average_bandwidth: avg,
                codecs: r.codecs,
                frame_rate: (src.frame_rate * 1000.0).round() / 1000.0,
                playlist_key: format!("{prefix}{name}/index.m3u8"),
            });
        }

        // Step 7, Garage capacity.
        if let Some(free) = logical_free(&repo::capacity(&self.db).await?)
            && free.saturating_sub(self.min_free) < total
        {
            return Ok(Outcome::Blocked("storage_full", 900));
        }
        if cancelled() {
            return Ok(Outcome::Cancelled);
        }

        // Step 8, upload.
        if !self.upload_all(&out, &prefix, &files).await {
            return Ok(Outcome::Storage);
        }
        progress.send_replace(0.97);

        // Step 9, verify.
        match self.verify(&prefix, &files).await {
            Ok(true) => {}
            Ok(false) | Err(StorageError::NotFound) => return Ok(Outcome::Transient("verify-failed")),
            Err(e) => return Ok(storage(&e, "verify")),
        }
        repo::mark_verified(&self.db, attempt, i64::try_from(total).unwrap_or(i64::MAX)).await?;

        // Step 10, switch.
        let audio_key = format!("{prefix}a/index.m3u8");
        let scrub_key = format!("{prefix}scrub/scrub.vtt");
        let switched = repo::switch(
            &self.db,
            &self.instance,
            Switch {
                job_id: job.id,
                variant_id: variant.id,
                attempt_id: attempt,
                upload_id: source.id,
                duration_ms: i64::try_from(src.duration_ms).unwrap_or(i64::MAX),
                audio_channels: i16::try_from(src.audio_channels.min(2)).unwrap_or(2),
                audio_playlist_key: &audio_key,
                scrub_vtt_key: &scrub_key,
                renditions: &renditions,
            },
        )
        .await?;
        Ok(match switched {
            Switched::Done => Outcome::Done,
            Switched::Cancelled => Outcome::Cancelled,
            Switched::Lost | Switched::Gone => Outcome::Lost,
        })
    }

    /// Uploads every output with 8 in flight, each PUT retried with backoff. False on any
    /// storage failure, which requeues the job. Written objects stay for encode GC.
    async fn upload_all(&self, out: &Path, prefix: &str, files: &BTreeMap<String, u64>) -> bool {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(UPLOAD_PARALLELISM));
        let mut set = JoinSet::new();
        for rel in files.keys() {
            let (storage, sem) = (self.storage.clone(), semaphore.clone());
            let path = out.join(rel);
            let key = format!("{prefix}{rel}");
            let ct = ffmpeg::content_type(rel, rel.starts_with("a/")).unwrap_or("application/octet-stream");
            set.spawn(async move {
                let Ok(_permit) = sem.acquire_owned().await else {
                    return false;
                };
                for i in 0..PUT_TRIES {
                    match storage
                        .put_file(BucketName::Media, &key, &path, ct, Some(OUTPUT_CACHE))
                        .await
                    {
                        Ok(_) => return true,
                        Err(e) if e.is_retryable() && i + 1 < PUT_TRIES => {
                            tokio::time::sleep(Duration::from_secs(1 << (2 * i))).await;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "output upload failed");
                            return false;
                        }
                    }
                }
                false
            });
        }
        let mut ok = true;
        while let Some(r) = set.join_next().await {
            if !matches!(r, Ok(true)) {
                ok = false;
                set.abort_all();
            }
        }
        ok
    }

    /// Lists the attempt back and compares every key and size, then checks that each playlist
    /// parses and names only objects that exist (SPEC 7.3 step 9).
    async fn verify(&self, prefix: &str, files: &BTreeMap<String, u64>) -> Result<bool, StorageError> {
        let listed: BTreeMap<String, u64> = self
            .storage
            .list_prefix(BucketName::Media, prefix)
            .await?
            .into_iter()
            .filter_map(|o| o.key.strip_prefix(prefix).map(|k| (k.to_owned(), o.size)))
            .collect();
        if &listed != files {
            tracing::warn!(local = files.len(), stored = listed.len(), "stored outputs differ");
            return Ok(false);
        }
        // Output names come from our own argument vectors, always lower case.
        for rel in files
            .keys()
            .filter(|k| Path::new(k).extension().is_some_and(|e| e == "m3u8"))
        {
            let (body, _) = self
                .storage
                .get_object(BucketName::Media, &format!("{prefix}{rel}"))
                .await?;
            let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
            let Some((init, segments)) = std::str::from_utf8(&body).ok().and_then(ffmpeg::parse_media_playlist) else {
                return Ok(false);
            };
            let named = init.into_iter().chain(segments.into_iter().map(|(_, n)| n));
            for n in named {
                if !listed.contains_key(&format!("{dir}/{n}")) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}

/// Checks a hardware backend once at start (SPEC 7.5). Returns the encoder to use and the label
/// reported to admins, `x264 (fallback)` when the device or the test encode failed.
pub async fn choose_encoder(ffmpeg_path: &Path, wanted: Encoder) -> (Encoder, &'static str) {
    if wanted == Encoder::X264 {
        return (Encoder::X264, "x264");
    }
    if tokio::fs::metadata(ffmpeg::RENDER_NODE).await.is_err() {
        tracing::warn!(encoder = wanted.as_str(), "no render node, encoding with x264");
        return (Encoder::X264, "x264 (fallback)");
    }
    match run_tool(ffmpeg_path, "ffmpeg", &ffmpeg::hardware_test_args(wanted)).await {
        Ok(_) => (wanted, wanted.as_str()),
        Err(e) => {
            tracing::warn!(encoder = wanted.as_str(), error = %e, "hardware test encode failed, encoding with x264");
            (Encoder::X264, "x264 (fallback)")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_is_unknown_when_any_row_is_old() {
        let row = |avail: i64, recent: bool| repo::CapacityRow {
            data_avail: avail,
            replication: 2,
            recent,
        };
        assert_eq!(logical_free(&[row(400, true), row(300, true)]), Some(300));
        assert_eq!(logical_free(&[row(400, true), row(300, false)]), None);
        assert_eq!(logical_free(&[]), None);
    }

    #[test]
    fn bandwidths_take_the_peak_segment_and_the_mean() {
        let sizes: BTreeMap<String, u64> = [
            ("v360/seg_0.m4s".to_owned(), 600_000),
            ("v360/seg_1.m4s".to_owned(), 150_000),
        ]
        .into();
        let segs = vec![(6.0, "seg_0.m4s".to_owned()), (2.0, "seg_1.m4s".to_owned())];
        assert_eq!(bandwidths(&segs, &sizes, "v360"), Some((800_000, 750_000)));
        assert_eq!(bandwidths(&[(6.0, "missing.m4s".to_owned())], &sizes, "v360"), None);
    }

    #[test]
    fn scratch_free_space_is_readable() {
        assert!(free_bytes(&std::env::temp_dir()).is_some_and(|b| b > 0));
        assert_eq!(free_bytes(Path::new("/nonexistent/r16a")), None);
    }
}
