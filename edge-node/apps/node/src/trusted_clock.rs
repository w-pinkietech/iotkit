//! The collector's receive-time clock backed by the node's `ClockTrust`, so
//! that Observations carry `unix_epoch_ms` exactly while the device can vouch
//! for its wall clock, and `null` otherwise (contract section 4).

use std::sync::Arc;

use iotkit_core_collector::{FreshnessClock, FreshnessSnapshot};
use iotkit_core_ops::{ClockEvidence, ClockTrust};

pub(crate) struct ClockTrustFreshness {
    clock_trust: Arc<ClockTrust>,
}

impl ClockTrustFreshness {
    pub(crate) fn new(clock_trust: Arc<ClockTrust>) -> Self {
        Self { clock_trust }
    }
}

impl FreshnessClock for ClockTrustFreshness {
    fn snapshot(&self, conn: &rusqlite::Connection) -> Result<FreshnessSnapshot, String> {
        let received_at_ms = self.clock_trust.wall_time_ms();
        let trusted = matches!(
            self.clock_trust.refresh(conn),
            Ok(ClockEvidence::Trusted { .. })
        );
        Ok(FreshnessSnapshot {
            received_at_ms,
            trusted_wall_time_ms: trusted.then_some(received_at_ms),
        })
    }
}
