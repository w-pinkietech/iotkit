use std::time::{SystemTime, UNIX_EPOCH};

/// Largest configurable freshness interval (365 days).
///
/// This keeps receiver arithmetic and configuration operationally bounded while
/// allowing deployments substantially more latitude than the one-day default.
pub const MAX_FRESHNESS_LIMIT_MS: i64 = 365 * 24 * 60 * 60 * 1000;

/// Receiver-owned clock snapshot used for freshness decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreshnessSnapshot {
    /// Edge Node receive time for persistence and relative-age checks.
    pub received_at_ms: i64,
    /// Trusted wall time for absolute device timestamps, or `None` while untrusted.
    pub trusted_wall_time_ms: Option<i64>,
}

/// Collector boundary implemented by Task 5 using the shared `ClockTrust` owner.
/// Sender input cannot construct or override this receiver-owned evidence.
pub trait FreshnessClock: Send + Sync {
    /// Capture one clock state for the entire envelope.
    fn snapshot(&self, conn: &rusqlite::Connection) -> Result<FreshnessSnapshot, String>;
}

/// Default for existing in-process collection: receive time is available, but
/// absolute wall time is not claimed trusted.
pub struct UntrustedSystemClock;

impl FreshnessClock for UntrustedSystemClock {
    fn snapshot(&self, _conn: &rusqlite::Connection) -> Result<FreshnessSnapshot, String> {
        let received_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0);
        Ok(FreshnessSnapshot {
            received_at_ms,
            trusted_wall_time_ms: None,
        })
    }
}

/// Configured absolute and relative freshness bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreshnessLimits {
    /// Maximum accepted observation age.
    max_age_ms: i64,
    /// Maximum accepted future clock skew.
    max_future_skew_ms: i64,
}

/// Invalid receiver freshness configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidFreshnessLimits;

impl std::fmt::Display for InvalidFreshnessLimits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "freshness limits must be within 0..={MAX_FRESHNESS_LIMIT_MS} milliseconds"
        )
    }
}

impl std::error::Error for InvalidFreshnessLimits {}

impl FreshnessLimits {
    /// Validate receiver configuration before collector startup.
    pub fn new(max_age_ms: i64, max_future_skew_ms: i64) -> Result<Self, InvalidFreshnessLimits> {
        if !(0..=MAX_FRESHNESS_LIMIT_MS).contains(&max_age_ms)
            || !(0..=MAX_FRESHNESS_LIMIT_MS).contains(&max_future_skew_ms)
        {
            return Err(InvalidFreshnessLimits);
        }
        Ok(Self {
            max_age_ms,
            max_future_skew_ms,
        })
    }

    pub fn max_age_ms(self) -> i64 {
        self.max_age_ms
    }

    pub fn max_future_skew_ms(self) -> i64 {
        self.max_future_skew_ms
    }
}

impl Default for FreshnessLimits {
    fn default() -> Self {
        Self::new(24 * 60 * 60 * 1000, 5 * 60 * 1000).expect("default freshness limits are valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_reject_negative_and_effectively_unbounded_values() {
        assert!(FreshnessLimits::new(-1, 0).is_err());
        assert!(FreshnessLimits::new(0, -1).is_err());
        assert!(FreshnessLimits::new(MAX_FRESHNESS_LIMIT_MS + 1, 0).is_err());
        assert!(FreshnessLimits::new(0, MAX_FRESHNESS_LIMIT_MS + 1).is_err());
    }

    #[test]
    fn limits_accept_exact_finite_boundaries() {
        let zero = FreshnessLimits::new(0, 0).unwrap();
        assert_eq!(zero.max_age_ms(), 0);
        assert_eq!(zero.max_future_skew_ms(), 0);

        let maximum = FreshnessLimits::new(MAX_FRESHNESS_LIMIT_MS, MAX_FRESHNESS_LIMIT_MS).unwrap();
        assert_eq!(maximum.max_age_ms(), MAX_FRESHNESS_LIMIT_MS);
        assert_eq!(maximum.max_future_skew_ms(), MAX_FRESHNESS_LIMIT_MS);
    }
}
