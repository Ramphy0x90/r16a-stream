//! Rules every secret value must pass before the process starts.

use std::collections::HashSet;

use super::{ConfigError, Env};

/// Words that only appear in shipped templates. A secret containing one was never replaced.
const PLACEHOLDER_MARKERS: [&str; 7] = [
    "change_me",
    "change-me",
    "changeme",
    "placeholder",
    "replace",
    "example",
    "xxxx",
];

/// Garage key ids published in `deploy/compose`. They only exist in local containers, and
/// accepting them in production would mean someone copied the dev compose file.
/// Session key published in `backend/.env.example`, the dev compose file and the tests. Sealing
/// production sessions with it would make every sealed refresh token readable by anyone.
pub const DEV_SESSION_KEYS: [&str; 1] = ["BwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyAhIiMkJSY="];

pub const DEV_GARAGE_KEY_IDS: [&str; 2] = ["GKe781357014cf1870ce72ffb3", "GKb21e6a01028af364e9ab7821"];

/// Validates one secret value. `min_len` is the shortest value that can still be random.
pub fn check_secret(name: &'static str, value: &str, env: Env, min_len: usize) -> Result<(), ConfigError> {
    if value.is_empty() {
        return Err(ConfigError::Missing { name });
    }
    let lower = value.to_ascii_lowercase();
    if PLACEHOLDER_MARKERS.iter().any(|m| lower.contains(m)) {
        return Err(ConfigError::invalid(name, "still holds a template placeholder"));
    }
    if lower.contains("dev-only") && !env.allows_dev_values() {
        return Err(ConfigError::invalid(
            name,
            "holds a published development value outside development",
        ));
    }
    if value.chars().count() < min_len {
        return Err(ConfigError::invalid(
            name,
            format!("must be at least {min_len} characters"),
        ));
    }
    let distinct = value.chars().collect::<HashSet<_>>().len();
    if distinct < 8.min(min_len) {
        return Err(ConfigError::invalid(name, "is too repetitive to be random"));
    }
    Ok(())
}

pub fn looks_like_garage_key_id(id: &str) -> bool {
    id.len() == 26 && id.starts_with("GK") && id[2..].chars().all(|c| c.is_ascii_hexdigit())
}
