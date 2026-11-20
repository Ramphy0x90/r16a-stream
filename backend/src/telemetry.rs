//! Log output setup.
//!
//! Production logs are JSON lines. Development logs are human readable. The filter comes from
//! `RUST_LOG` and defaults to `info`. Nothing in this crate logs request bodies, query strings,
//! headers or secrets, see `http::trace`.

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::config::LogFormat;

pub fn init(format: LogFormat) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let registry = tracing_subscriber::registry().with(filter);
    // A second init, as in tests, keeps the first subscriber.
    let _ = match format {
        LogFormat::Json => registry
            .with(fmt::layer().json().flatten_event(true).with_current_span(true))
            .try_init(),
        LogFormat::Pretty => registry.with(fmt::layer().compact()).try_init(),
    };
}
