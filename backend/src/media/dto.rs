//! Request and response shapes of variant, upload, job and storage administration.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::error::AppError;

/// Largest source accepted, 60 GiB (SPEC 4.5).
pub const MAX_SOURCE_BYTES: u64 = 64_424_509_440;
/// Every part is 50 MiB except the last.
pub const PART_SIZE: u64 = 52_428_800;
/// Part URLs per request.
pub const MAX_PART_URLS: usize = 50;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VariantCreate {
    pub title_id: Uuid,
    /// Required for shows, absent for movies.
    pub episode_id: Option<Uuid>,
    /// Content language, BCP 47, for example `it` or `pt-BR`.
    #[schema(max_length = 35)]
    pub language: String,
}

/// Normalises a content language tag, lowercase language and uppercase region, `pt-br` to
/// `pt-BR` (SPEC 3). `None` when it is not a tag the database accepts.
pub fn normalize_language(raw: &str) -> Option<String> {
    let mut parts = raw.trim().split(['-', '_']);
    let primary = parts.next()?.to_ascii_lowercase();
    let mut out = primary;
    for p in parts {
        out.push('-');
        match p.len() {
            2 if p.bytes().all(|b| b.is_ascii_alphabetic()) => out.push_str(&p.to_ascii_uppercase()),
            4 if p.bytes().all(|b| b.is_ascii_alphabetic()) => {
                out.push_str(&p[..1].to_ascii_uppercase());
                out.push_str(&p[1..].to_ascii_lowercase());
            }
            _ => out.push_str(p),
        }
    }
    crate::me::dto::is_language_tag(&out).then_some(out)
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobSummary {
    pub id: Uuid,
    /// `queued`, `running`, `cancelling`, `succeeded`, `failed` or `cancelled`.
    pub status: String,
    pub progress: Option<f32>,
    pub failure_detail: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminVariant {
    pub id: Uuid,
    pub episode_id: Option<Uuid>,
    pub language: String,
    /// `awaiting_upload`, `processing`, `ready` or `failed`.
    pub status: String,
    pub duration_ms: Option<i64>,
    /// Bytes of the current encode.
    pub current_bytes: Option<i64>,
    /// Bytes of the source while it is kept.
    pub source_bytes: Option<i64>,
    /// A re-transcode needs the source. False once retention deleted it.
    pub source_kept: bool,
    /// An upload that can still be resumed.
    pub open_upload_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub job: Option<JobSummary>,
}

/// Per-title storage and the editor's variant list (SPEC 7.10).
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TitleStorage {
    pub variants: Vec<AdminVariant>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UploadStart {
    #[schema(max_length = 255)]
    pub file_name: String,
    pub size_bytes: u64,
    /// Read from the file by the browser when it can, 10 s to 8 h, for the space estimate.
    pub duration_seconds: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UploadStarted {
    pub upload_id: Uuid,
    pub part_size: u64,
    pub part_count: u32,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PartsRequest {
    /// At most 50 per request.
    pub part_numbers: Vec<u16>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PartUrl {
    pub part_number: u16,
    /// Presigned PUT on the app's own origin, valid one hour. Never log it.
    pub url: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PartUrls {
    pub parts: Vec<PartUrl>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StoredPartDto {
    pub part_number: u16,
    pub etag: String,
    pub size_bytes: u64,
}

/// An upload with the parts Garage already holds, for resuming.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UploadState {
    pub upload_id: Uuid,
    /// `initiated`, `completed` or `aborted`.
    pub status: String,
    pub size_bytes: u64,
    pub part_size: u64,
    pub part_count: u32,
    pub parts: Vec<StoredPartDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompletedPart {
    pub part_number: u16,
    #[schema(max_length = 128)]
    pub etag: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UploadComplete {
    pub parts: Vec<CompletedPart>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobRef {
    pub job_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminJob {
    pub id: Uuid,
    /// `transcode`, `image`, `subtitle` or `encode_gc`.
    pub kind: String,
    pub status: String,
    pub progress: Option<f32>,
    pub attempts: i16,
    pub max_attempts: i16,
    pub storage_waits: i32,
    /// `storage_degraded`, `storage_full` or `scratch_full` while the job waits.
    pub blocked_reason: Option<String>,
    pub failure_code: Option<String>,
    pub failure_detail: Option<String>,
    pub variant_id: Option<Uuid>,
    pub image_id: Option<Uuid>,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<time::OffsetDateTime>,
    #[schema(value_type = Option<String>, format = DateTime)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub finished_at: Option<time::OffsetDateTime>,
    #[schema(value_type = String, format = DateTime)]
    #[serde(with = "time::serde::rfc3339")]
    pub run_after: time::OffsetDateTime,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminJobPage {
    pub items: Vec<AdminJob>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct JobListQuery {
    pub status: Option<String>,
    pub kind: Option<String>,
    #[param(max_length = 512)]
    pub cursor: Option<String>,
    #[param(minimum = 1, maximum = 60)]
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NodeCapacity {
    pub node: String,
    pub data_avail_bytes: i64,
    pub data_total_bytes: i64,
    pub meta_avail_bytes: Option<i64>,
    pub meta_total_bytes: Option<i64>,
    /// Updated within five minutes.
    pub fresh: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComponentHealthDto {
    /// `api` or `worker`.
    pub component: String,
    pub instance: String,
    pub storage_writable: bool,
    /// The worker's encoder, `x264 (fallback)` when Quick Sync failed its test.
    pub encoder: Option<String>,
    pub fresh: bool,
}

/// The admin storage page (SPEC 7.8, 7.10).
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StorageOverview {
    pub capacity_known: bool,
    /// Why capacity is unknown, `not_configured` or `stale`.
    pub unknown_reason: Option<String>,
    pub logical_free_bytes: Option<u64>,
    pub reserved_bytes: u64,
    /// Free space minus reservations below the warning threshold, or capacity unknown.
    pub warning: bool,
    /// Some component cannot write to Garage right now.
    pub degraded: bool,
    pub nodes: Vec<NodeCapacity>,
    pub components: Vec<ComponentHealthDto>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TitleUsageDto {
    pub title_id: Uuid,
    pub name: String,
    pub bytes: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TitleUsagePage {
    pub items: Vec<TitleUsageDto>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct CursorQuery {
    #[param(max_length = 512)]
    pub cursor: Option<String>,
    #[param(minimum = 1, maximum = 60)]
    pub limit: Option<i64>,
}

/// `ETag` values from Garage are quoted hex. Anything else did not come from Garage.
pub fn valid_etag(e: &str) -> bool {
    (1..=128).contains(&e.len()) && e.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'"' || b == b'-')
}

pub fn part_count(size: u64) -> Result<u32, AppError> {
    if size == 0 || size > MAX_SOURCE_BYTES {
        return Err(AppError::BadRequest("invalid-size"));
    }
    u32::try_from(size.div_ceil(PART_SIZE)).map_err(|_| AppError::BadRequest("invalid-size"))
}

/// Largest manual image, 20 MiB, uploaded as one part (SPEC 8.2).
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// Largest subtitle file, 2 MiB (SPEC 5.2).
pub const MAX_SUBTITLE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageUploadStart {
    /// `poster` or `backdrop` for a title, `still` for an episode.
    #[schema(max_length = 16)]
    pub kind: String,
    pub title_id: Option<Uuid>,
    pub episode_id: Option<Uuid>,
    pub size_bytes: u64,
    /// `image/jpeg`, `image/png` or `image/webp`.
    #[schema(max_length = 32)]
    pub content_type: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageUploadStarted {
    pub upload_id: Uuid,
    pub image_id: Uuid,
    /// Presigned PUT of the single part, valid one hour. Never log it.
    pub part_url: String,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleQuery {
    /// Language of the text, BCP 47.
    #[param(max_length = 35)]
    pub language: String,
    #[param(max_length = 60)]
    pub label: Option<String>,
    /// `subtitles` (default), `captions` or `forced`.
    pub kind: Option<String>,
}

/// A WebVTT file as the player can load it. UTF-8, optional byte order mark, the `WEBVTT`
/// signature followed by a space, tab or line end, and no NUL anywhere.
pub fn is_webvtt(raw: &[u8]) -> bool {
    let body = raw.strip_prefix(b"\xEF\xBB\xBF".as_slice()).unwrap_or(raw);
    let Ok(text) = std::str::from_utf8(body) else {
        return false;
    };
    let Some(rest) = text.strip_prefix("WEBVTT") else {
        return false;
    };
    !text.contains('\0') && (rest.is_empty() || rest.starts_with([' ', '\t', '\n', '\r']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_normalise_to_the_stored_form() {
        assert_eq!(normalize_language("IT").as_deref(), Some("it"));
        assert_eq!(normalize_language("pt-br").as_deref(), Some("pt-BR"));
        assert_eq!(normalize_language("zh_hant_tw").as_deref(), Some("zh-Hant-TW"));
        assert_eq!(normalize_language("es-419").as_deref(), Some("es-419"));
        for bad in ["", "english", "e", "it-", "it--x", "it;drop"] {
            assert!(normalize_language(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn part_counts_follow_the_50_mib_size() {
        assert_eq!(part_count(1).expect("ok"), 1);
        assert_eq!(part_count(PART_SIZE).expect("ok"), 1);
        assert_eq!(part_count(PART_SIZE + 1).expect("ok"), 2);
        assert_eq!(part_count(MAX_SOURCE_BYTES).expect("ok"), 1229);
        assert!(part_count(0).is_err() && part_count(MAX_SOURCE_BYTES + 1).is_err());
    }

    #[test]
    fn webvtt_needs_its_signature() {
        assert!(is_webvtt(b"WEBVTT\n\n00:00.000 --> 00:01.000\nHi\n"));
        assert!(is_webvtt(b"\xEF\xBB\xBFWEBVTT - title\n"));
        assert!(is_webvtt(b"WEBVTT"));
        assert!(!is_webvtt(b"WEBVTTX\n"));
        assert!(!is_webvtt(b"1\n00:00:00,000 --> 00:00:01,000\nSRT\n"));
        assert!(!is_webvtt(b"WEBVTT\n\0"));
        assert!(!is_webvtt(b"WEBVTT\n\xff"));
    }

    #[test]
    fn etags_look_like_garage_ones() {
        assert!(valid_etag("\"9b2cf535f27731c974343645a3985328\""));
        assert!(!valid_etag("\"x\"<script>"));
        assert!(!valid_etag(""));
    }
}
