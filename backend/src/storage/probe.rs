//! Periodic Garage write probe.
//!
//! With replication 2 a single node outage blocks every write while reads keep working, and
//! the first sign of it should not be a failed upload. Each component that writes puts a tiny
//! object into its own writable bucket every interval and records the result in memory and in
//! `component_health`. Only the worker exposes the result in `/readyz` (SPEC 7.8). The API
//! uses it to answer `503 storage_degraded` on routes that write.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::Duration,
};

use sqlx::PgPool;

use super::{BucketName, GarageClient};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    Api,
    Worker,
}

impl Component {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Worker => "worker",
        }
    }

    /// The bucket this component's key may write.
    fn bucket(self) -> BucketName {
        match self {
            Self::Api => BucketName::Uploads,
            Self::Worker => BucketName::Media,
        }
    }
}

/// Shared view of the latest probe result.
#[derive(Debug, Default)]
pub struct WriteHealth {
    writable: AtomicBool,
    /// Unix seconds of the last finished probe, 0 before the first one.
    checked_at: AtomicI64,
}

impl WriteHealth {
    pub fn record(&self, ok: bool) {
        self.writable.store(ok, Ordering::Relaxed);
        self.checked_at
            .store(time::OffsetDateTime::now_utc().unix_timestamp(), Ordering::Relaxed);
    }

    /// True only when the last probe succeeded and is recent. A probe that stopped running
    /// counts as a failure, so a stuck task cannot keep the state green.
    pub fn is_writable(&self, interval: Duration) -> bool {
        let checked = self.checked_at.load(Ordering::Relaxed);
        let max_age = i64::try_from(interval.as_secs().saturating_mul(3)).unwrap_or(i64::MAX);
        let fresh = checked > 0 && time::OffsetDateTime::now_utc().unix_timestamp() - checked <= max_age;
        fresh && self.writable.load(Ordering::Relaxed)
    }
}

/// Runs one probe, records it, and reports it to Postgres. Errors writing the health row are
/// logged and otherwise ignored, the probe result in memory is what callers rely on.
pub async fn probe_once(
    client: &GarageClient,
    db: &PgPool,
    health: &WriteHealth,
    component: Component,
    instance: &str,
) -> bool {
    let key = format!("_health/{}-{instance}", component.as_str());
    let ok = match client
        .put_object(component.bucket(), &key, b"ok", "text/plain", Some("no-store"))
        .await
    {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(component = component.as_str(), error = %e, "storage write probe failed");
            false
        }
    };
    health.record(ok);
    if let Err(e) = upsert_health(db, component, instance, ok).await {
        tracing::warn!(error = %e, "could not record component health");
    }
    ok
}

async fn upsert_health(db: &PgPool, component: Component, instance: &str, writable: bool) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"insert into component_health (component, instance, storage_writable, checked_at)
           values ($1, $2, $3, now())
           on conflict (component, instance)
           do update set storage_writable = excluded.storage_writable, checked_at = excluded.checked_at"#,
        component.as_str(),
        instance,
        writable,
    )
    .execute(db)
    .await
    .map(|_| ())
}

/// Spawns the probe loop. It runs until the process exits.
pub fn spawn(
    client: Arc<GarageClient>,
    db: PgPool,
    health: Arc<WriteHealth>,
    component: Component,
    instance: String,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            probe_once(&client, &db, &health, component, &instance).await;
        }
    })
}
