//! Opaque keyset cursors (SPEC 5.1).
//!
//! A cursor is base64url JSON of the last row's sort key and id. It is opaque to clients but not
//! secret, it only ever moves the caller through their own result set, and the query that reads
//! it still carries every filter and access rule. A cursor that does not parse is 400.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Serialize, de::DeserializeOwned};

use crate::error::AppError;

/// Longest cursor accepted, far above any real one.
const MAX_LEN: usize = 512;

pub fn encode<T: Serialize>(key: &T) -> Result<String, AppError> {
    serde_json::to_vec(key)
        .map(|b| URL_SAFE_NO_PAD.encode(b))
        .map_err(AppError::internal)
}

pub fn decode<T: DeserializeOwned>(raw: Option<&str>) -> Result<Option<T>, AppError> {
    let Some(raw) = raw.filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    if raw.len() > MAX_LEN {
        return Err(AppError::BadRequest("invalid-cursor"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| AppError::BadRequest("invalid-cursor"))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| AppError::BadRequest("invalid-cursor"))
}

/// Page size from `?limit=`, 1 to 60, default 30.
pub fn limit(raw: Option<i64>) -> Result<i64, AppError> {
    match raw {
        None => Ok(30),
        Some(n) if (1..=60).contains(&n) => Ok(n),
        Some(_) => Err(AppError::BadRequest("invalid-limit")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_garbage() {
        let c = encode(&("a", 3)).expect("encode");
        assert_eq!(
            decode::<(String, i32)>(Some(&c)).expect("decode"),
            Some(("a".into(), 3))
        );
        assert!(decode::<(String, i32)>(Some("!!")).is_err());
        assert!(decode::<(String, i32)>(Some(&"a".repeat(600))).is_err());
        assert_eq!(decode::<(String, i32)>(None).expect("none"), None);
        assert_eq!(limit(None).expect("default"), 30);
        assert!(limit(Some(0)).is_err() && limit(Some(61)).is_err());
    }
}
