//! Runs the long ffmpeg encode with progress, cancellation and a wall clock limit (SPEC 7.2,
//! 7.3 step 5).
//!
//! ffmpeg writes `key=value` progress lines on stdout. The runner turns `out_time_us` into a
//! fraction of the source duration and publishes it on a watch channel. The worker's heartbeat
//! task reads it and answers with a cancel flag. On cancel or timeout the whole process group is
//! killed, so nothing the encoder started outlives it.

use std::{ffi::OsString, path::Path, process::Stdio, time::Duration};

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
    sync::watch,
};

/// Most stderr kept from an encode, for the log only.
const MAX_STDERR: u64 = 64 * 1024;
/// Encoding is reported up to this fraction. The rest is upload and verification.
pub const ENCODE_SHARE: f32 = 0.9;

#[derive(Debug, PartialEq, Eq)]
pub enum RunEnd {
    Done,
    Cancelled,
    Timeout,
    /// Could not start the binary.
    Spawn,
    /// Non-zero exit. `stderr` is for logs only, it can contain paths.
    Failed {
        code: Option<i32>,
        stderr: String,
    },
}

/// Fraction of the encode done from an `out_time_us` value.
pub fn fraction(out_time_us: i64, duration_ms: u64) -> f32 {
    if duration_ms == 0 || out_time_us <= 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss, reason = "a progress fraction")]
    let f = (out_time_us as f64 / 1000.0 / duration_ms as f64).clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation, reason = "within 0 and 1")]
    let f = f as f32;
    f * ENCODE_SHARE
}

fn kill_group(pid: Option<u32>) {
    let Some(pid) = pid
        .and_then(|p| i32::try_from(p).ok())
        .and_then(rustix::process::Pid::from_raw)
    else {
        return;
    };
    // The child leads its own group (`process_group(0)`), so this reaches every process it made.
    if let Err(e) = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL) {
        tracing::warn!(error = %e, "killing the encoder process group failed");
    }
}

/// Runs `binary` with `args` until it exits, `cancel` turns true, or `limit` passes.
pub async fn run(
    binary: &Path,
    args: &[OsString],
    limit: Duration,
    duration_ms: u64,
    progress: &watch::Sender<f32>,
    mut cancel: watch::Receiver<bool>,
) -> RunEnd {
    let Ok(mut child) = Command::new(binary)
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
    else {
        return RunEnd::Spawn;
    };
    let pid = child.id();
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        kill_group(pid);
        return RunEnd::Spawn;
    };
    let err_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stderr.take(MAX_STDERR).read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf)
            .chars()
            .rev()
            .take(500)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>()
    });
    let mut lines = BufReader::new(stdout).lines();
    let deadline = tokio::time::sleep(limit);
    tokio::pin!(deadline);
    let mut stdout_open = true;
    // A dropped sender means nobody can cancel any more, not a cancel.
    let mut cancel_open = true;
    let end = loop {
        tokio::select! {
            line = lines.next_line(), if stdout_open => match line {
                Ok(Some(l)) => {
                    if let Some(v) = l.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<i64>().ok()) {
                        progress.send_replace(fraction(v, duration_ms));
                    }
                }
                _ => stdout_open = false,
            },
            status = child.wait(), if !stdout_open => break match status {
                Ok(s) if s.success() => RunEnd::Done,
                Ok(s) => RunEnd::Failed { code: s.code(), stderr: err_task.await.unwrap_or_default() },
                Err(_) => RunEnd::Failed { code: None, stderr: String::new() },
            },
            changed = cancel.changed(), if cancel_open => {
                if changed.is_err() {
                    cancel_open = false;
                } else if *cancel.borrow() {
                    kill_group(pid);
                    let _ = child.wait().await;
                    break RunEnd::Cancelled;
                }
            },
            () = &mut deadline => {
                kill_group(pid);
                let _ = child.wait().await;
                break RunEnd::Timeout;
            },
        }
    };
    if matches!(end, RunEnd::Done) {
        progress.send_replace(ENCODE_SHARE);
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_a_share_of_the_duration() {
        assert!((fraction(5_000_000, 10_000) - 0.45).abs() < 1e-6);
        assert!((fraction(50_000_000, 10_000) - ENCODE_SHARE).abs() < 1e-6, "clamped");
        assert!(
            fraction(-1, 10_000).abs() < f32::EPSILON,
            "ffmpeg prints N/A or negatives at start"
        );
        assert!(fraction(1, 0).abs() < f32::EPSILON);
    }

    fn sh() -> &'static Path {
        Path::new("/bin/sh")
    }

    #[tokio::test]
    async fn progress_lines_are_read_and_exit_codes_reported() {
        let (tx, _rx) = watch::channel(0.0);
        let (_ctx, crx) = watch::channel(false);
        let args: Vec<OsString> = ["-c", "echo out_time_us=5000000; echo progress=end"]
            .map(Into::into)
            .to_vec();
        assert_eq!(
            run(sh(), &args, Duration::from_secs(5), 10_000, &tx, crx).await,
            RunEnd::Done
        );
        assert!((*tx.borrow() - ENCODE_SHARE).abs() < 1e-6);

        let (_ctx, crx) = watch::channel(false);
        let args: Vec<OsString> = ["-c", "echo nope >&2; exit 3"].map(Into::into).to_vec();
        match run(sh(), &args, Duration::from_secs(5), 10_000, &tx, crx).await {
            RunEnd::Failed { code, stderr } => assert_eq!((code, stderr.trim()), (Some(3), "nope")),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_and_timeout_kill_the_whole_group() {
        let (tx, _rx) = watch::channel(0.0);
        // A grandchild that would outlive a plain kill of the shell.
        let args: Vec<OsString> = ["-c", "sleep 30 & sleep 30; wait"].map(Into::into).to_vec();
        let (_ctx, crx) = watch::channel(false);
        let started = std::time::Instant::now();
        assert_eq!(
            run(sh(), &args, Duration::from_millis(300), 10_000, &tx, crx).await,
            RunEnd::Timeout
        );
        assert!(started.elapsed() < Duration::from_secs(5));

        let (ctx, crx) = watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            ctx.send_replace(true);
            tokio::time::sleep(Duration::from_secs(10)).await;
        });
        assert_eq!(
            run(sh(), &args, Duration::from_secs(30), 10_000, &tx, crx).await,
            RunEnd::Cancelled
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn a_missing_binary_is_a_spawn_error() {
        let (tx, _rx) = watch::channel(0.0);
        let (_ctx, crx) = watch::channel(false);
        assert_eq!(
            run(
                Path::new("/nonexistent/ffmpeg"),
                &[],
                Duration::from_secs(1),
                1,
                &tx,
                crx
            )
            .await,
            RunEnd::Spawn
        );
    }
}
