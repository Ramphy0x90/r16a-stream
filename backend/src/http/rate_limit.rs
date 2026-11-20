//! In-memory GCRA rate limits per replica, SPEC 16.
//!
//! With N API replicas the effective limit is N times the configured one. That is accepted for
//! a private service with a handful of users, the limits exist to blunt brute force and
//! runaway clients, not to meter usage.

use std::{hash::Hash, num::NonZeroU32, time::Duration};

use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter, clock::Clock};

use crate::error::AppError;

pub struct KeyedLimit<K: Hash + Eq + Clone> {
    inner: DefaultKeyedRateLimiter<K>,
}

impl<K: Hash + Eq + Clone> std::fmt::Debug for KeyedLimit<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyedLimit")
    }
}

impl<K: Hash + Eq + Clone> KeyedLimit<K> {
    /// `per_minute` requests per minute on average, with `burst` allowed at once.
    pub fn per_minute(per_minute: u32, burst: u32) -> Self {
        let rate = NonZeroU32::new(per_minute.max(1)).unwrap_or(NonZeroU32::MIN);
        let burst = NonZeroU32::new(burst.max(1)).unwrap_or(NonZeroU32::MIN);
        Self {
            inner: RateLimiter::keyed(Quota::per_minute(rate).allow_burst(burst)),
        }
    }

    pub fn check(&self, key: &K) -> Result<(), AppError> {
        self.inner.check_key(key).map_err(|not_until| {
            let wait = not_until.wait_time_from(governor::clock::DefaultClock::default().now());
            AppError::RateLimited {
                retry_after_secs: wait.as_secs().saturating_add(1),
            }
        })
    }

    /// Drops state for keys that are back at full capacity, so memory does not grow with every
    /// address ever seen. Called periodically by the API.
    pub fn shrink(&self) {
        self.inner.retain_recent();
        self.inner.shrink_to_fit();
    }
}

/// How often idle limiter entries are dropped.
pub const SHRINK_INTERVAL: Duration = Duration::from_secs(300);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_then_refusal_with_retry_after() {
        let l = KeyedLimit::per_minute(20, 3);
        for _ in 0..3 {
            l.check(&"a").expect("within burst");
        }
        match l.check(&"a") {
            Err(AppError::RateLimited { retry_after_secs }) => assert!((1..=4).contains(&retry_after_secs)),
            other => panic!("expected a rate limit, got {other:?}"),
        }
        l.check(&"b").expect("other keys are independent");
    }
}
