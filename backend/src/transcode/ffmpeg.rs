//! ffprobe rules, the encoding ladder and the ffmpeg argument vectors (SPEC 7.3, 7.4, 7.5).
//!
//! Pure functions only, so unit tests pin every flag. The process rules of `images::ffmpeg`
//! apply unchanged, local files only, `-protocol_whitelist file`, a demuxer whitelist, an
//! argument vector, an empty environment and a process group of its own.
//!
//! One ffmpeg process decodes the source once and feeds every rung, the audio rendition and the
//! scrub sprites from it. The sprites come from the same decoded frames as the rungs, so they
//! need no second decode and no HLS demuxer in the whitelist.

use std::{ffi::OsString, fmt::Write as _, path::Path};

use serde::Deserialize;

use crate::config::Encoder;

/// Containers a source may be read with. Playlists, concat lists, image sequences and every
/// network format are refused by ffmpeg itself.
pub const VIDEO_FORMATS: &str = "matroska,webm,mov,mp4,m4a,3gp,3g2,mj2,mpegts,avi";
/// Seconds between scrub frames and the size of one sprite sheet.
pub const SPRITE_INTERVAL_SECS: u64 = 10;
pub const SPRITE_WIDTH: u32 = 160;
pub const SPRITE_GRID: u32 = 10;
/// The render node a hardware encoder uses (SPEC 7.5).
pub const RENDER_NODE: &str = "/dev/dri/renderD128";

/// One rung of the ladder (SPEC 7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rung {
    pub height: u32,
    pub width: u32,
    pub maxrate_kbps: u32,
    pub profile: &'static str,
    pub level: &'static str,
    /// RFC 6381 codec string for the master playlist.
    pub codecs: &'static str,
}

impl Rung {
    /// Directory and object key segment, `v1080`.
    pub fn name(&self) -> String {
        format!("v{}", self.height)
    }
}

/// Height, maximum width, VBV maxrate, profile, level and codec string of each rung.
const LADDER: [(u32, u32, u32, &str, &str, &str); 4] = [
    (1080, 1920, 6000, "high", "4.1", "avc1.640029"),
    (720, 1280, 3500, "high", "4.0", "avc1.640028"),
    (480, 854, 1800, "main", "3.1", "avc1.4d401f"),
    (360, 640, 900, "main", "3.0", "avc1.4d401e"),
];

fn even(v: f64) -> u32 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "bounded by the probe limits"
    )]
    let n = v.round().max(2.0) as u32;
    n - n % 2
}

/// The rungs for a source of the given display size. Rungs above the source height are skipped,
/// a source below 360p gets one rung at its own even height. Widths keep the aspect ratio and
/// respect each rung's maximum width.
pub fn ladder(display_width: u32, display_height: u32) -> Vec<Rung> {
    let aspect = f64::from(display_width) / f64::from(display_height);
    let fit = |height: u32, max_width: u32| {
        let w = even(f64::from(height) * aspect);
        if w > max_width {
            (max_width, even(f64::from(max_width) / aspect))
        } else {
            (w, height)
        }
    };
    let mut out: Vec<Rung> = LADDER
        .iter()
        .filter(|(h, ..)| *h <= display_height)
        .map(|&(h, max_w, rate, profile, level, codecs)| {
            let (width, height) = fit(h, max_w);
            Rung {
                height,
                width,
                maxrate_kbps: rate,
                profile,
                level,
                codecs,
            }
        })
        .collect();
    if out.is_empty() {
        let (_, _, rate, profile, level, codecs) = LADDER[3];
        let (width, height) = fit(even(f64::from(display_height)), 640);
        out.push(Rung {
            height,
            width,
            maxrate_kbps: rate,
            profile,
            level,
            codecs,
        });
    }
    out
}

/// Height of one scrub frame, 160 px wide with the source aspect ratio.
pub fn sprite_height(display_width: u32, display_height: u32) -> u32 {
    even(f64::from(SPRITE_WIDTH) * f64::from(display_height) / f64::from(display_width))
}

// ---------------------------------------------------------------- probe

#[derive(Debug, Deserialize)]
struct ProbeOut {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    index: u32,
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    sample_aspect_ratio: Option<String>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    channels: Option<u32>,
    #[serde(default)]
    disposition: Disposition,
    #[serde(default)]
    tags: Tags,
}

#[derive(Debug, Default, Deserialize)]
struct Disposition {
    #[serde(default)]
    default: u8,
    #[serde(default)]
    attached_pic: u8,
}

#[derive(Debug, Default, Deserialize)]
struct Tags {
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

/// What the encode needs to know about an accepted source.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub video_index: u32,
    pub audio_index: u32,
    pub audio_channels: u32,
    pub duration_ms: u64,
    /// Size after applying the sample aspect ratio.
    pub display_width: u32,
    pub display_height: u32,
    pub frame_rate: f64,
}

/// Why a source is refused. Each is a fixed code and text shown to the admin (SPEC 7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Unreadable,
    NoVideo,
    NoAudio,
    Duration,
    TooLarge,
    TooSmall,
    FrameRate,
}

impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unreadable => "probe-unreadable",
            Self::NoVideo => "no-video",
            Self::NoAudio => "no-audio",
            Self::Duration => "duration",
            Self::TooLarge => "too-large",
            Self::TooSmall => "too-small",
            Self::FrameRate => "frame-rate",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::Unreadable => "The file is not a video container the server accepts.",
            Self::NoVideo => "The file has no video stream.",
            Self::NoAudio => "The file has no audio stream.",
            Self::Duration => "The video must be between 10 seconds and 8 hours long.",
            Self::TooLarge => "The picture is larger than 7680 pixels.",
            Self::TooSmall => "The picture is smaller than 144 pixels high.",
            Self::FrameRate => "The frame rate is unknown or above 120.",
        }
    }
}

fn ratio(raw: Option<&str>) -> Option<f64> {
    let (n, d) = raw?.split_once(['/', ':'])?;
    let (n, d) = (n.trim().parse::<f64>().ok()?, d.trim().parse::<f64>().ok()?);
    (n > 0.0 && d > 0.0).then(|| n / d)
}

/// `ffprobe` arguments printing streams and format as JSON.
pub fn probe_args(input: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = [
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        VIDEO_FORMATS,
        "-show_format",
        "-show_streams",
        "-of",
        "json",
    ]
    .map(OsString::from)
    .to_vec();
    a.push(input.into());
    a
}

/// Applies the source rules of SPEC 7.3 step 3. The audio stream tagged with the variant's
/// language wins, otherwise the first.
pub fn parse_probe(out: &[u8], language: &str) -> Result<Source, Refusal> {
    let p: ProbeOut = serde_json::from_slice(out).map_err(|_| Refusal::Unreadable)?;
    let videos: Vec<&ProbeStream> = p
        .streams
        .iter()
        .filter(|s| s.codec_type.as_deref() == Some("video") && s.disposition.attached_pic == 0)
        .collect();
    let video = videos
        .iter()
        .find(|s| s.disposition.default == 1)
        .or_else(|| videos.first())
        .ok_or(Refusal::NoVideo)?;
    let audios: Vec<&ProbeStream> = p
        .streams
        .iter()
        .filter(|s| s.codec_type.as_deref() == Some("audio"))
        .collect();
    let primary = language.split('-').next().unwrap_or(language);
    let audio = audios
        .iter()
        .find(|s| {
            s.tags
                .language
                .as_deref()
                .is_some_and(|l| l.eq_ignore_ascii_case(language) || l.eq_ignore_ascii_case(primary))
        })
        .or_else(|| audios.first())
        .ok_or(Refusal::NoAudio)?;

    let seconds = p
        .format
        .as_ref()
        .and_then(|f| f.duration.as_deref())
        .and_then(|d| d.trim().parse::<f64>().ok())
        .filter(|d| d.is_finite())
        .ok_or(Refusal::Duration)?;
    if !(10.0..=8.0 * 3600.0).contains(&seconds) {
        return Err(Refusal::Duration);
    }
    let (w, h) = (video.width.unwrap_or(0), video.height.unwrap_or(0));
    if w == 0 || h == 0 {
        return Err(Refusal::NoVideo);
    }
    if w > 7680 || h > 7680 {
        return Err(Refusal::TooLarge);
    }
    let sar = ratio(video.sample_aspect_ratio.as_deref()).unwrap_or(1.0);
    let display_width = even(f64::from(w) * sar);
    let display_height = h - h % 2;
    if display_height < 144 || display_width < 16 {
        return Err(Refusal::TooSmall);
    }
    if display_width > 7680 {
        return Err(Refusal::TooLarge);
    }
    let fps = ratio(video.avg_frame_rate.as_deref())
        .or_else(|| ratio(video.r_frame_rate.as_deref()))
        .ok_or(Refusal::FrameRate)?;
    if fps > 120.0 {
        return Err(Refusal::FrameRate);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "at most 8 h")]
    let duration_ms = (seconds * 1000.0).round() as u64;
    Ok(Source {
        video_index: video.index,
        audio_index: audio.index,
        audio_channels: audio.channels.unwrap_or(2).clamp(1, 8),
        duration_ms,
        display_width,
        display_height,
        frame_rate: fps,
    })
}

// ---------------------------------------------------------------- encode

fn hls(a: &mut Vec<OsString>, dir: &Path) {
    a.extend(
        [
            "-map_metadata",
            "-1",
            "-map_chapters",
            "-1",
            "-f",
            "hls",
            "-hls_time",
            "6",
            "-hls_playlist_type",
            "vod",
            "-hls_segment_type",
            "fmp4",
            "-hls_flags",
            "independent_segments",
            "-hls_fmp4_init_filename",
            "init.mp4",
            "-hls_segment_filename",
        ]
        .map(OsString::from),
    );
    a.push(dir.join("seg_%d.m4s").into());
    a.push(dir.join("index.m3u8").into());
}

/// The video encoder options of one rung for the chosen backend.
fn video_codec(encoder: Encoder, rung: &Rung) -> Vec<String> {
    let rate = format!("{}k", rung.maxrate_kbps);
    let buf = format!("{}k", rung.maxrate_kbps * 2);
    let mut a: Vec<String> = match encoder {
        Encoder::X264 => [
            "-c:v", "libx264", "-preset", "medium", "-crf", "21", "-pix_fmt", "yuv420p",
        ]
        .map(String::from)
        .to_vec(),
        // Quality targets for the hardware encoders are starting points until the ThinkStation
        // measurements of SPEC 7.5 replace them.
        Encoder::Qsv => [
            "-c:v",
            "h264_qsv",
            "-global_quality",
            "23",
            "-look_ahead",
            "0",
            "-forced_idr",
            "1",
        ]
        .map(String::from)
        .to_vec(),
        Encoder::Vaapi => ["-c:v", "h264_vaapi", "-rc_mode", "QVBR", "-global_quality", "23"]
            .map(String::from)
            .to_vec(),
    };
    a.extend(
        [
            "-maxrate",
            &rate,
            "-bufsize",
            &buf,
            "-profile:v",
            rung.profile,
            "-level:v",
            rung.level,
            "-force_key_frames",
            "expr:gte(t,n_forced*2)",
        ]
        .map(String::from),
    );
    if encoder == Encoder::X264 {
        a.extend(["-sc_threshold", "0", "-flags", "+cgop"].map(String::from));
    }
    a
}

/// The scale step of one rung. Hardware backends upload the decoded frames and scale on the GPU.
fn scale(encoder: Encoder, rung: &Rung) -> String {
    match encoder {
        Encoder::X264 => format!("scale={}:{}:flags=bicubic,setsar=1", rung.width, rung.height),
        Encoder::Qsv => format!(
            "format=nv12,hwupload=extra_hw_frames=16,scale_qsv=w={}:h={}",
            rung.width, rung.height
        ),
        Encoder::Vaapi => format!("format=nv12,hwupload,scale_vaapi=w={}:h={}", rung.width, rung.height),
    }
}

/// The filter graph, one decoded video split into every rung plus the sprite sheets.
pub fn filter_graph(encoder: Encoder, src: &Source, rungs: &[Rung]) -> String {
    let n = rungs.len() + 1;
    let mut g = format!("[0:{}]split={n}", src.video_index);
    for i in 0..n {
        let _ = write!(g, "[s{i}]");
    }
    for (i, r) in rungs.iter().enumerate() {
        let _ = write!(g, ";[s{i}]{}[v{i}]", scale(encoder, r));
    }
    let _ = write!(
        g,
        ";[s{}]fps=1/{SPRITE_INTERVAL_SECS},scale={SPRITE_WIDTH}:{},setsar=1,tile={SPRITE_GRID}x{SPRITE_GRID}[sp]",
        n - 1,
        sprite_height(src.display_width, src.display_height)
    );
    g
}

/// The whole encode of SPEC 7.3 step 5 and 6, in the layout of SPEC 10.2. Outputs go to
/// `out/v<height>/`, `out/a/` and `out/scrub/`, directories the caller creates first.
pub fn encode_args(encoder: Encoder, input: &Path, out: &Path, src: &Source, rungs: &[Rung]) -> Vec<OsString> {
    let mut a: Vec<OsString> = [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-loglevel",
        "error",
        "-nostats",
        "-progress",
        "pipe:1",
    ]
    .map(OsString::from)
    .to_vec();
    match encoder {
        Encoder::X264 => {}
        Encoder::Qsv => a.extend(
            [
                "-init_hw_device".into(),
                format!("qsv=hw:{RENDER_NODE}"),
                "-filter_hw_device".into(),
                "hw".into(),
            ]
            .map(OsString::from),
        ),
        Encoder::Vaapi => a.extend(["-vaapi_device", RENDER_NODE].map(OsString::from)),
    }
    a.extend(["-protocol_whitelist", "file", "-format_whitelist", VIDEO_FORMATS, "-i"].map(OsString::from));
    a.push(input.into());
    a.push("-filter_complex".into());
    a.push(filter_graph(encoder, src, rungs).into());
    for (i, r) in rungs.iter().enumerate() {
        a.extend(["-map".into(), format!("[v{i}]"), "-an".into()].map(OsString::from));
        a.extend(video_codec(encoder, r).into_iter().map(OsString::from));
        hls(&mut a, &out.join(r.name()));
    }
    a.extend(
        [
            "-map".into(),
            format!("0:{}", src.audio_index),
            "-vn".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "128k".into(),
            "-ac".into(),
            "2".into(),
            "-ar".into(),
            "48000".into(),
        ]
        .map(OsString::from),
    );
    hls(&mut a, &out.join("a"));
    a.extend(
        [
            "-map",
            "[sp]",
            "-c:v",
            "libwebp",
            "-quality",
            "75",
            "-fps_mode",
            "passthrough",
            "-map_metadata",
            "-1",
            "-f",
            "image2",
            "-start_number",
            "1",
        ]
        .map(OsString::from),
    );
    a.push(out.join("scrub").join("sprite_%d.webp").into());
    a
}

/// A one frame test encode for the hardware fallback check of SPEC 7.5. Reads nothing from disk.
pub fn hardware_test_args(encoder: Encoder) -> Vec<OsString> {
    let mut a: Vec<OsString> = ["-hide_banner", "-nostdin", "-loglevel", "error"]
        .map(OsString::from)
        .to_vec();
    let (init, filter, codec): (Vec<String>, &str, &str) = match encoder {
        Encoder::X264 => (vec![], "format=yuv420p", "libx264"),
        Encoder::Qsv => (
            vec![
                "-init_hw_device".into(),
                format!("qsv=hw:{RENDER_NODE}"),
                "-filter_hw_device".into(),
                "hw".into(),
            ],
            "format=nv12,hwupload=extra_hw_frames=16",
            "h264_qsv",
        ),
        Encoder::Vaapi => (
            vec!["-vaapi_device".into(), RENDER_NODE.into()],
            "format=nv12,hwupload",
            "h264_vaapi",
        ),
    };
    a.extend(init.into_iter().map(OsString::from));
    a.extend(
        [
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=256x144:r=25:d=0.04",
            "-vf",
            filter,
            "-frames:v",
            "1",
            "-c:v",
            codec,
            "-f",
            "null",
            "-",
        ]
        .map(OsString::from),
    );
    a
}

// ---------------------------------------------------------------- outputs

/// Content type of an output file by extension (SPEC 10.4). `None` refuses the file.
pub fn content_type(relative: &str, audio: bool) -> Option<&'static str> {
    let ext = relative.rsplit_once('.').map(|(_, e)| e)?;
    Some(match ext {
        "m3u8" => "application/vnd.apple.mpegurl",
        "mp4" | "m4s" if audio => "audio/mp4",
        "mp4" | "m4s" => "video/mp4",
        "vtt" => "text/vtt; charset=utf-8",
        "webp" => "image/webp",
        _ => return None,
    })
}

/// The scrub map for the sprite sheets of SPEC 7.3 step 6, stored next to them in `scrub/`, so
/// cues name the sheets relative to it. Frame `k` shows second `10 k` and covers until the
/// next frame or the end.
pub fn scrub_vtt(duration_ms: u64, width: u32, height: u32) -> String {
    fn ts(ms: u64) -> String {
        format!(
            "{:02}:{:02}:{:02}.{:03}",
            ms / 3_600_000,
            ms / 60_000 % 60,
            ms / 1000 % 60,
            ms % 1000
        )
    }
    let step = SPRITE_INTERVAL_SECS * 1000;
    let per_sheet = u64::from(SPRITE_GRID * SPRITE_GRID);
    let mut out = String::from("WEBVTT\n");
    let mut k = 0u64;
    while k * step < duration_ms {
        let start = k * step;
        let end = (start + step).min(duration_ms);
        let sheet = k / per_sheet + 1;
        let cell = k % per_sheet;
        let x = (cell % u64::from(SPRITE_GRID)) * u64::from(width);
        let y = (cell / u64::from(SPRITE_GRID)) * u64::from(height);
        let _ = write!(
            out,
            "\n{} --> {}\nsprite_{sheet}.webp#xywh={x},{y},{width},{height}\n",
            ts(start),
            ts(end)
        );
        k += 1;
    }
    out
}

/// The init file of a media playlist, and each segment's duration with the file it names.
pub type MediaPlaylist = (Option<String>, Vec<(f64, String)>);

/// Segment durations of a media playlist with the file each one names, and the init file.
/// `None` when the text is not a complete VOD playlist of plain relative names.
pub fn parse_media_playlist(text: &str) -> Option<MediaPlaylist> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    if lines.next()? != "#EXTM3U" {
        return None;
    }
    let mut init = None;
    let mut segments = Vec::new();
    let mut pending: Option<f64> = None;
    let mut ended = false;
    for l in lines {
        if let Some(rest) = l.strip_prefix("#EXT-X-MAP:URI=\"") {
            init = Some(rest.strip_suffix('"')?.to_owned());
        } else if let Some(rest) = l.strip_prefix("#EXTINF:") {
            pending = Some(rest.split(',').next()?.parse().ok()?);
        } else if l == "#EXT-X-ENDLIST" {
            ended = true;
        } else if !l.starts_with('#') {
            let plain = l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.') && !l.starts_with('.');
            if !plain {
                return None;
            }
            segments.push((pending.take()?, l.to_owned()));
        }
    }
    (ended && !segments.is_empty() && init.as_deref().is_none_or(|i| !i.contains('/'))).then_some((init, segments))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn s(v: &[OsString]) -> Vec<String> {
        v.iter().map(|x| x.to_string_lossy().into_owned()).collect()
    }

    fn source(w: u32, h: u32) -> Source {
        Source {
            video_index: 0,
            audio_index: 1,
            audio_channels: 6,
            duration_ms: 25_000,
            display_width: w,
            display_height: h,
            frame_rate: 25.0,
        }
    }

    #[test]
    fn the_ladder_skips_rungs_above_the_source() {
        let l = ladder(1920, 1080);
        assert_eq!(
            l.iter().map(|r| (r.width, r.height)).collect::<Vec<_>>(),
            [(1920, 1080), (1280, 720), (852, 480), (640, 360)]
        );
        let scope = ladder(1920, 800);
        assert_eq!(
            scope.iter().map(|r| (r.width, r.height)).collect::<Vec<_>>(),
            [(1280, 532), (854, 356), (640, 266)]
        );
        let tiny = ladder(320, 240);
        assert_eq!((tiny.len(), tiny[0].width, tiny[0].height), (1, 320, 240));
        assert_eq!(tiny[0].codecs, "avc1.4d401e");
        // A PAL DVD in four by three after the sample aspect ratio is applied.
        assert_eq!(
            ladder(768, 576).iter().map(|r| r.height).collect::<Vec<_>>(),
            [480, 360]
        );
    }

    const PROBE: &str = r#"{
      "streams": [
        {"index": 0, "codec_type": "video", "width": 720, "height": 576, "sample_aspect_ratio": "16:15",
         "avg_frame_rate": "25/1", "disposition": {"default": 1, "attached_pic": 0}},
        {"index": 1, "codec_type": "audio", "channels": 2, "tags": {"language": "eng"}},
        {"index": 2, "codec_type": "audio", "channels": 6, "tags": {"language": "it"}},
        {"index": 3, "codec_type": "video", "width": 600, "height": 600, "disposition": {"attached_pic": 1}}
      ],
      "format": {"duration": "5400.040000"}
    }"#;

    #[test]
    fn probe_prefers_the_variant_language_and_applies_the_aspect_ratio() {
        let s = parse_probe(PROBE.as_bytes(), "it").expect("accepted");
        assert_eq!((s.video_index, s.audio_index, s.audio_channels), (0, 2, 6));
        assert_eq!((s.display_width, s.display_height), (768, 576));
        assert_eq!(s.duration_ms, 5_400_040);
        assert_eq!(
            parse_probe(PROBE.as_bytes(), "de").expect("accepted").audio_index,
            1,
            "first audio"
        );
    }

    #[test]
    fn probe_refuses_what_the_rules_exclude() {
        let with = |from: &str, to: &str| parse_probe(PROBE.replace(from, to).as_bytes(), "it");
        assert_eq!(with("5400.040000", "9.5"), Err(Refusal::Duration));
        assert_eq!(with("5400.040000", "28801"), Err(Refusal::Duration));
        assert_eq!(with("\"audio\"", "\"data\""), Err(Refusal::NoAudio));
        assert_eq!(with("\"width\": 720", "\"width\": 8000"), Err(Refusal::TooLarge));
        assert_eq!(with("\"height\": 576", "\"height\": 100"), Err(Refusal::TooSmall));
        assert_eq!(with("25/1", "240/1"), Err(Refusal::FrameRate));
        assert_eq!(parse_probe(b"not json", "it"), Err(Refusal::Unreadable));
        let only_cover = r#"{"streams": [{"index": 0, "codec_type": "video", "width": 9, "height": 9,
            "disposition": {"attached_pic": 1}}], "format": {"duration": "60"}}"#;
        assert_eq!(parse_probe(only_cover.as_bytes(), "it"), Err(Refusal::NoVideo));
    }

    #[test]
    fn x264_encode_vector_is_pinned() {
        let src = source(1280, 720);
        let rungs = ladder(1280, 720);
        let a = s(&encode_args(
            Encoder::X264,
            Path::new("/s/source"),
            &PathBuf::from("/s/out"),
            &src,
            &rungs,
        ));
        let joined = a.join(" ");
        assert_eq!(
            joined,
            "-hide_banner -nostdin -y -loglevel error -nostats -progress pipe:1 \
             -protocol_whitelist file -format_whitelist matroska,webm,mov,mp4,m4a,3gp,3g2,mj2,mpegts,avi -i /s/source \
             -filter_complex [0:0]split=4[s0][s1][s2][s3];[s0]scale=1280:720:flags=bicubic,setsar=1[v0];\
             [s1]scale=852:480:flags=bicubic,setsar=1[v1];[s2]scale=640:360:flags=bicubic,setsar=1[v2];\
             [s3]fps=1/10,scale=160:90,setsar=1,tile=10x10[sp] \
             -map [v0] -an -c:v libx264 -preset medium -crf 21 -pix_fmt yuv420p -maxrate 3500k -bufsize 7000k \
             -profile:v high -level:v 4.0 -force_key_frames expr:gte(t,n_forced*2) -sc_threshold 0 -flags +cgop \
             -map_metadata -1 -map_chapters -1 -f hls -hls_time 6 -hls_playlist_type vod -hls_segment_type fmp4 \
             -hls_flags independent_segments -hls_fmp4_init_filename init.mp4 \
             -hls_segment_filename /s/out/v720/seg_%d.m4s /s/out/v720/index.m3u8 \
             -map [v1] -an -c:v libx264 -preset medium -crf 21 -pix_fmt yuv420p -maxrate 1800k -bufsize 3600k \
             -profile:v main -level:v 3.1 -force_key_frames expr:gte(t,n_forced*2) -sc_threshold 0 -flags +cgop \
             -map_metadata -1 -map_chapters -1 -f hls -hls_time 6 -hls_playlist_type vod -hls_segment_type fmp4 \
             -hls_flags independent_segments -hls_fmp4_init_filename init.mp4 \
             -hls_segment_filename /s/out/v480/seg_%d.m4s /s/out/v480/index.m3u8 \
             -map [v2] -an -c:v libx264 -preset medium -crf 21 -pix_fmt yuv420p -maxrate 900k -bufsize 1800k \
             -profile:v main -level:v 3.0 -force_key_frames expr:gte(t,n_forced*2) -sc_threshold 0 -flags +cgop \
             -map_metadata -1 -map_chapters -1 -f hls -hls_time 6 -hls_playlist_type vod -hls_segment_type fmp4 \
             -hls_flags independent_segments -hls_fmp4_init_filename init.mp4 \
             -hls_segment_filename /s/out/v360/seg_%d.m4s /s/out/v360/index.m3u8 \
             -map 0:1 -vn -c:a aac -b:a 128k -ac 2 -ar 48000 \
             -map_metadata -1 -map_chapters -1 -f hls -hls_time 6 -hls_playlist_type vod -hls_segment_type fmp4 \
             -hls_flags independent_segments -hls_fmp4_init_filename init.mp4 \
             -hls_segment_filename /s/out/a/seg_%d.m4s /s/out/a/index.m3u8 \
             -map [sp] -c:v libwebp -quality 75 -fps_mode passthrough -map_metadata -1 -f image2 -start_number 1 \
             /s/out/scrub/sprite_%d.webp"
        );
    }

    #[test]
    fn hardware_vectors_upload_frames_and_keep_the_whitelists() {
        let src = source(1920, 1080);
        let rungs = ladder(1920, 1080);
        for (enc, codec, init) in [
            (Encoder::Qsv, "h264_qsv", "-init_hw_device"),
            (Encoder::Vaapi, "h264_vaapi", "-vaapi_device"),
        ] {
            let a = s(&encode_args(
                enc,
                Path::new("/s/source"),
                Path::new("/s/out"),
                &src,
                &rungs,
            ));
            assert!(a.windows(2).any(|w| w == ["-protocol_whitelist", "file"]));
            assert!(a.windows(2).any(|w| w == ["-format_whitelist", VIDEO_FORMATS]));
            assert_eq!(a.iter().filter(|x| *x == codec).count(), 4, "{enc:?}");
            assert!(a.iter().position(|x| x == init) < a.iter().position(|x| x == "-i"));
            assert!(a.iter().any(|x| x.contains("hwupload")));
            assert!(!a.iter().any(|x| x.contains("sc_threshold")));
            assert!(a.contains(&"libwebp".to_owned()), "sprites stay in software");
        }
        for enc in [Encoder::X264, Encoder::Qsv, Encoder::Vaapi] {
            let a = s(&hardware_test_args(enc));
            assert!(a.windows(2).any(|w| w == ["-frames:v", "1"]));
            assert!(
                !a.iter()
                    .any(|x| x.starts_with('/') && x != RENDER_NODE && !x.contains("qsv=hw"))
            );
        }
    }

    #[test]
    fn only_allowlisted_outputs_have_a_content_type() {
        assert_eq!(
            content_type("v720/index.m3u8", false),
            Some("application/vnd.apple.mpegurl")
        );
        assert_eq!(content_type("a/seg_1.m4s", true), Some("audio/mp4"));
        assert_eq!(content_type("v720/init.mp4", false), Some("video/mp4"));
        assert_eq!(content_type("scrub/sprite_1.webp", false), Some("image/webp"));
        assert_eq!(content_type("scrub/scrub.vtt", false), Some("text/vtt; charset=utf-8"));
        assert_eq!(content_type("v720/seg_1.ts", false), None);
        assert_eq!(content_type("noext", false), None);
    }

    #[test]
    fn scrub_map_covers_the_duration_across_sheets() {
        let v = scrub_vtt(25_000, 160, 90);
        assert!(v.starts_with("WEBVTT\n"));
        assert!(v.contains("00:00:00.000 --> 00:00:10.000\nsprite_1.webp#xywh=0,0,160,90"));
        assert!(v.contains("00:00:20.000 --> 00:00:25.000\nsprite_1.webp#xywh=320,0,160,90"));
        let long = scrub_vtt(1_001_000, 160, 90);
        assert!(long.contains("00:16:30.000 --> 00:16:40.000\nsprite_1.webp#xywh=1440,810,160,90"));
        assert!(long.contains("00:16:40.000 --> 00:16:41.000\nsprite_2.webp#xywh=0,0,160,90"));
    }

    #[test]
    fn media_playlists_must_be_complete_and_local() {
        let ok = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6.000000,\nseg_0.m4s\n#EXTINF:1.04,\nseg_1.m4s\n#EXT-X-ENDLIST\n";
        let (init, segs) = parse_media_playlist(ok).expect("parsed");
        assert_eq!(init.as_deref(), Some("init.mp4"));
        assert_eq!(segs, [(6.0, "seg_0.m4s".to_owned()), (1.04, "seg_1.m4s".to_owned())]);
        assert!(
            parse_media_playlist(&ok.replace("#EXT-X-ENDLIST\n", "")).is_none(),
            "not finished"
        );
        assert!(parse_media_playlist(&ok.replace("seg_1.m4s", "../x/seg_1.m4s")).is_none());
        assert!(parse_media_playlist(&ok.replace("seg_1.m4s", "https://e/seg")).is_none());
    }
}
