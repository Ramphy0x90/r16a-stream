//! Garage capacity, upload admission and the storage view (SPEC 7.10).
//!
//! Free space comes from each node's `/metrics` on the admin port, read with the read-only
//! `metrics_token`. Used space comes from our own records. Admission reserves the source plus a
//! ladder estimate, serialised by an advisory lock so two uploads cannot spend the same space.

pub mod repository;

use std::time::Duration;

use reqwest::Client;
use secrecy::ExposeSecret;
use sqlx::PgPool;

use crate::config::StoragePolicy;

/// Rows older than this count as unknown.
pub const FRESH: Duration = Duration::from_mins(5);
/// Poll interval and timeout (SPEC 7.10).
pub const POLL: Duration = Duration::from_mins(1);
const TIMEOUT: Duration = Duration::from_secs(5);
/// Largest metrics page read. Garage's is a few KiB.
const MAX_BODY: usize = 512 * 1024;

/// Ladder VBV maxrates up to 1080p plus 128 kbit/s audio (SPEC 7.4), bits per second.
const LADDER_BPS: u64 = (6_000 + 3_500 + 1_800 + 900 + 128) * 1000;
/// Sprites and playlists.
const LADDER_EXTRA: u64 = 64 * 1024 * 1024;

/// Values of one node's metrics page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeMetrics {
    pub data_avail: u64,
    pub data_total: u64,
    pub meta_avail: Option<u64>,
    pub meta_total: Option<u64>,
    pub replication: u16,
}

/// Parses only the five values used, ignoring every other line (SPEC 7.10, verified names of
/// Garage 1.0.1, re-checked on upgrades per DECISIONS section C).
pub fn parse_metrics(text: &str) -> Option<NodeMetrics> {
    let mut data_avail = None;
    let mut data_total = None;
    let mut meta_avail = None;
    let mut meta_total = None;
    let mut replication = None;
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let Some((name_labels, value)) = line.rsplit_once(' ') else {
            continue;
        };
        let Ok(v) = value.trim().parse::<f64>() else { continue };
        if !v.is_finite() || v < 0.0 {
            continue;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let n = v as u64;
        let (name, labels) = name_labels.split_once('{').map_or((name_labels, ""), |(a, b)| (a, b));
        let data = labels.contains("volume=\"data\"");
        let meta = labels.contains("volume=\"metadata\"");
        match name {
            "garage_local_disk_avail" if data => data_avail = Some(n),
            "garage_local_disk_total" if data => data_total = Some(n),
            "garage_local_disk_avail" if meta => meta_avail = Some(n),
            "garage_local_disk_total" if meta => meta_total = Some(n),
            "garage_replication_factor" => replication = u16::try_from(n).ok(),
            _ => {}
        }
    }
    Some(NodeMetrics {
        data_avail: data_avail?,
        data_total: data_total?,
        meta_avail,
        meta_total,
        replication: replication.filter(|r| (1..=7).contains(r))?,
    })
}

/// Free space usable for new objects, from the fresh rows of every configured node, or `None`
/// when any node is missing or stale. `min(avail) x nodes / replication`, so with two nodes and
/// replication 2 it is the fuller node's free space.
pub fn logical_free(nodes: &[(u64, u16)], expected: usize) -> Option<u64> {
    if nodes.is_empty() || nodes.len() < expected {
        return None;
    }
    let min = nodes.iter().map(|(avail, _)| *avail).min()?;
    let replication = u64::from(nodes.iter().map(|(_, r)| *r).max()?.max(1));
    Some(min.saturating_mul(nodes.len() as u64) / replication)
}

/// Upper bound of a transcode's outputs (SPEC 7.10). Without a duration the source size stands
/// in for it.
pub fn ladder_bytes(duration_secs: Option<u64>, source_bytes: u64) -> u64 {
    match duration_secs {
        Some(d) => (d.saturating_mul(LADDER_BPS) / 8).saturating_mul(105) / 100 + LADDER_EXTRA,
        None => source_bytes,
    }
}

/// The duration used for the estimate, the larger of the browser's (10 s to 8 h) and the
/// metadata runtime plus 10 %.
pub fn estimate_duration(browser_secs: Option<u64>, runtime_minutes: Option<i32>) -> Option<u64> {
    let browser = browser_secs.filter(|s| (10..=8 * 3600).contains(s));
    let metadata = runtime_minutes
        .and_then(|m| u64::try_from(m).ok())
        .filter(|m| *m > 0)
        .map(|m| m * 60 * 11 / 10);
    browser.max(metadata)
}

/// Polls every node once and stores what it got. Nodes that fail keep their old row, which goes
/// stale and makes capacity unknown.
pub async fn poll_once(db: &PgPool, http: &Client, policy: &StoragePolicy) {
    let Some(token) = &policy.metrics_token else { return };
    for url in &policy.metrics_endpoints {
        let node = node_name(url);
        let res = http
            .get(url.clone())
            .bearer_auth(token.expose_secret())
            .timeout(TIMEOUT)
            .send()
            .await;
        let text = match res {
            Ok(r) if r.status().is_success() => match read_capped(r).await {
                Some(t) => t,
                None => continue,
            },
            Ok(r) => {
                tracing::warn!(%node, status = %r.status(), "Garage metrics refused");
                continue;
            }
            Err(e) => {
                tracing::warn!(%node, timeout = e.is_timeout(), "Garage metrics unreachable");
                continue;
            }
        };
        let Some(m) = parse_metrics(&text) else {
            tracing::warn!(%node, "Garage metrics lack the capacity values");
            continue;
        };
        if let Err(e) = repository::store_node(db, &node, &m).await {
            tracing::warn!(error = %e, "storing capacity failed");
        }
    }
}

/// Host and port of a metrics URL, the row key in `garage_capacity`.
pub fn node_name(url: &url::Url) -> String {
    format!(
        "{}:{}",
        url.host_str().unwrap_or("unknown"),
        url.port_or_known_default().unwrap_or(0)
    )
}

async fn read_capped(mut res: reqwest::Response) -> Option<String> {
    let mut out = Vec::new();
    while let Ok(Some(chunk)) = res.chunk().await {
        if out.len() + chunk.len() > MAX_BODY {
            return None;
        }
        out.extend_from_slice(&chunk);
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"# HELP garage_local_disk_avail Number of bytes available
# TYPE garage_local_disk_avail gauge
garage_local_disk_avail{volume="data"} 4.5e11
garage_local_disk_avail{volume="metadata"} 9.0e9
garage_local_disk_total{volume="data"} 1.0e12
garage_local_disk_total{volume="metadata"} 2.0e10
garage_replication_factor 2
cluster_storage_nodes 2
api_s3_request_counter{api_endpoint="GetObject"} 12
"#;

    #[test]
    fn parses_the_capacity_values_and_ignores_the_rest() {
        let m = parse_metrics(PAGE).expect("parsed");
        assert_eq!(m.data_avail, 450_000_000_000);
        assert_eq!(m.data_total, 1_000_000_000_000);
        assert_eq!(m.meta_avail, Some(9_000_000_000));
        assert_eq!(m.replication, 2);
        assert!(
            parse_metrics("garage_replication_factor 2").is_none(),
            "data values missing"
        );
        assert!(parse_metrics("garage_local_disk_avail{volume=\"data\"} -5\n").is_none());
    }

    #[test]
    fn logical_free_is_the_fuller_node_with_two_copies() {
        assert_eq!(logical_free(&[(400, 2), (300, 2)], 2), Some(300));
        assert_eq!(logical_free(&[(400, 2), (300, 2), (500, 2)], 3), Some(450));
        assert_eq!(logical_free(&[(400, 2)], 2), None, "a node is missing");
        assert_eq!(logical_free(&[], 0), None);
    }

    #[test]
    fn ladder_estimate_matches_the_spec() {
        // About 5.6 GB per hour with every rung (SPEC 7.10).
        let hour = ladder_bytes(Some(3600), 1);
        assert!((5_700_000_000..5_900_000_000).contains(&hour), "{hour}");
        assert_eq!(ladder_bytes(None, 123), 123);
        assert_eq!(estimate_duration(Some(5), None), None, "below 10 s is ignored");
        assert_eq!(estimate_duration(Some(3000), Some(60)), Some(3960));
        assert_eq!(estimate_duration(Some(5000), Some(60)), Some(5000));
    }
}
