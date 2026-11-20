//! ffprobe and ffmpeg for images (SPEC 7.3, 8.2).
//!
//! Untrusted files are only ever read from local scratch paths. Every invocation gets
//! `-protocol_whitelist file` and a `-format_whitelist` of image demuxers, an argument vector
//! (never a shell), an empty environment, its own process group, a time limit, and capped
//! output. The argument builders are pure functions so tests can pin every flag.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use tokio::{io::AsyncReadExt, process::Command};

/// Demuxers an image source may be read with. Anything else, video containers, playlists,
/// concat lists, network formats, is refused by ffmpeg itself.
pub const IMAGE_FORMATS: &str = "image2,png_pipe,jpeg_pipe,webp_pipe";
/// Per process wall clock limit. A single still decodes and encodes in well under a second.
const TIME_LIMIT: Duration = Duration::from_secs(60);
/// Most stdout or stderr read from a process.
const MAX_OUTPUT: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("could not start {0}")]
    Spawn(&'static str),
    #[error("{0} timed out")]
    Timeout(&'static str),
    /// The tool refused the input or failed. `stderr` is for logs only, it can contain paths.
    #[error("{tool} exited with {code:?}")]
    Failed {
        tool: &'static str,
        code: Option<i32>,
        stderr: String,
    },
}

fn common_input(input: &Path) -> Vec<OsString> {
    vec![
        "-protocol_whitelist".into(),
        "file".into(),
        "-format_whitelist".into(),
        IMAGE_FORMATS.into(),
        "-i".into(),
        input.into(),
    ]
}

/// `ffprobe` arguments that print the first video stream's size as JSON.
pub fn probe_args(input: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec!["-v".into(), "error".into()];
    // ffprobe takes the input as a positional argument, not with -i.
    let mut input_args = common_input(input);
    input_args.retain(|x| x != "-i");
    a.extend(input_args);
    a.extend(
        [
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "json",
        ]
        .map(OsString::from),
    );
    a
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Avif,
    Webp,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Self::Avif => "avif",
            Self::Webp => "webp",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Avif => "image/avif",
            Self::Webp => "image/webp",
        }
    }
}

/// `ffmpeg` arguments that scale to `width` and encode one still. Metadata is dropped, so EXIF
/// location data and the like never reach the media bucket.
pub fn encode_args(input: &Path, output: &Path, width: u32, format: Format) -> Vec<OsString> {
    let mut a: Vec<OsString> = ["-nostdin", "-hide_banner", "-v", "error", "-threads", "2"]
        .map(OsString::from)
        .to_vec();
    a.extend(common_input(input));
    a.extend(
        [
            "-map_metadata".to_owned(),
            "-1".to_owned(),
            "-frames:v".to_owned(),
            "1".to_owned(),
            "-vf".to_owned(),
            format!("scale={width}:-2:flags=lanczos"),
        ]
        .map(OsString::from),
    );
    let codec: &[&str] = match format {
        Format::Avif => &[
            "-c:v",
            "libaom-av1",
            "-still-picture",
            "1",
            "-crf",
            "30",
            "-cpu-used",
            "6",
            "-pix_fmt",
            "yuv420p",
            "-f",
            "avif",
        ],
        Format::Webp => &["-c:v", "libwebp", "-quality", "80", "-f", "webp"],
    };
    a.extend(codec.iter().map(OsString::from));
    a.push("-y".into());
    a.push(output.into());
    a
}

/// `ffmpeg` arguments that average the image down to one RGB pixel, for the placeholder colour.
pub fn average_args(input: &Path, output: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = ["-nostdin", "-hide_banner", "-v", "error"].map(OsString::from).to_vec();
    a.extend(common_input(input));
    a.extend(
        [
            "-frames:v",
            "1",
            "-vf",
            "scale=1:1:flags=area",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "-y",
        ]
        .map(OsString::from),
    );
    a.push(output.into());
    a
}

/// Runs a tool and returns its stdout.
pub async fn run(binary: &Path, tool: &'static str, args: &[OsString]) -> Result<Vec<u8>, ToolError> {
    let mut child = Command::new(binary)
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| ToolError::Spawn(tool))?;
    let mut stdout = child.stdout.take().ok_or(ToolError::Spawn(tool))?;
    let mut stderr = child.stderr.take().ok_or(ToolError::Spawn(tool))?;
    let work = async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut out_reader = (&mut stdout).take(MAX_OUTPUT);
        let mut err_reader = (&mut stderr).take(MAX_OUTPUT);
        let (a, b) = tokio::join!(out_reader.read_to_end(&mut out), err_reader.read_to_end(&mut err),);
        a.and(b).map_err(|_| ToolError::Spawn(tool))?;
        let status = child.wait().await.map_err(|_| ToolError::Spawn(tool))?;
        Ok::<_, ToolError>((status, out, err))
    };
    let (status, out, err) = tokio::time::timeout(TIME_LIMIT, work)
        .await
        .map_err(|_| ToolError::Timeout(tool))??;
    if status.success() {
        Ok(out)
    } else {
        Err(ToolError::Failed {
            tool,
            code: status.code(),
            stderr: String::from_utf8_lossy(&err).chars().take(500).collect(),
        })
    }
}

/// Paths of one job's scratch directory, removed in every outcome by the caller.
pub struct Scratch {
    pub dir: PathBuf,
}

impl Scratch {
    pub async fn create(root: &Path, job: uuid::Uuid) -> std::io::Result<Self> {
        let dir = root.join(format!("job-{job}"));
        tokio::fs::create_dir_all(&dir).await?;
        Ok(Self { dir })
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub async fn remove(self) {
        if let Err(e) = tokio::fs::remove_dir_all(&self.dir).await {
            tracing::warn!(error = %e, "scratch cleanup failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[OsString]) -> Vec<String> {
        v.iter().map(|x| x.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn every_invocation_whitelists_protocol_and_formats() {
        let input = Path::new("/scratch/job/source");
        let output = Path::new("/scratch/job/out");
        for args in [
            probe_args(input),
            encode_args(input, output, 342, Format::Avif),
            encode_args(input, output, 342, Format::Webp),
            average_args(input, output),
        ] {
            let a = s(&args);
            let p = a
                .iter()
                .position(|x| x == "-protocol_whitelist")
                .expect("protocol whitelist");
            assert_eq!(a[p + 1], "file");
            let f = a
                .iter()
                .position(|x| x == "-format_whitelist")
                .expect("format whitelist");
            assert_eq!(a[f + 1], IMAGE_FORMATS);
            assert!(
                f < a.iter().position(|x| x.ends_with("source")).expect("input"),
                "options before the input"
            );
        }
    }

    #[test]
    fn encode_drops_metadata_and_pins_the_codec() {
        let a = s(&encode_args(Path::new("/s/in"), Path::new("/s/out"), 500, Format::Avif));
        assert!(a.windows(2).any(|w| w == ["-map_metadata", "-1"]));
        assert!(a.windows(2).any(|w| w == ["-c:v", "libaom-av1"]));
        assert!(a.windows(2).any(|w| w == ["-still-picture", "1"]));
        assert!(a.contains(&"scale=500:-2:flags=lanczos".to_owned()));
        assert_eq!(a.last().map(String::as_str), Some("/s/out"));
        let w = s(&encode_args(Path::new("/s/in"), Path::new("/s/out"), 500, Format::Webp));
        assert!(w.windows(2).any(|x| x == ["-c:v", "libwebp"]));
    }
}
