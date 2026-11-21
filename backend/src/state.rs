//! Shared state handed to every API handler.

use std::sync::Arc;

use sqlx::PgPool;

use crate::{
    auth::AuthState,
    config::ApiConfig,
    http::rate_limit::FeatureLimits,
    storage::{GarageClient, probe::WriteHealth},
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ApiConfig>,
    pub db: PgPool,
    pub storage: Arc<GarageClient>,
    pub write_health: Arc<WriteHealth>,
    pub auth: Arc<AuthState>,
    pub limits: Arc<FeatureLimits>,
    pub tmdb: Arc<crate::tmdb::client::TmdbClient>,
    pub tmdb_images: Arc<crate::images::ingest::TmdbImages>,
    pub image_urls: Arc<crate::images::urls::ImageUrls>,
    /// Signs upload part URLs for the admin's browser.
    pub upload_urls: Arc<crate::storage::public::UploadUrls>,
}

impl AppState {
    /// Whether routes that write to Garage may proceed. They answer `503 storage_degraded`
    /// otherwise (SPEC 7.8). Not part of `/readyz`, so playback stays up.
    pub fn storage_writable(&self) -> bool {
        self.write_health.is_writable(self.config.s3.probe_interval)
    }
}
