use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

#[derive(Clone, Default)]
pub(crate) struct ManualMonotonicClock(Arc<AtomicU64>);

impl ManualMonotonicClock {
    pub(crate) fn new(now_ms: u64) -> Self {
        Self(Arc::new(AtomicU64::new(now_ms)))
    }

    pub(crate) fn advance_ms(&self, elapsed_ms: u64) {
        self.0.fetch_add(elapsed_ms, Ordering::SeqCst);
    }
}

impl sealed::Sealed for ManualMonotonicClock {}

impl MonotonicClock for ManualMonotonicClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

impl AdmissionConfig {
    pub(crate) fn for_test() -> Self {
        Self {
            auth_workers: 2,
            reserved_auth_workers: 1,
            auth_rate_per_second: 100,
            auth_burst: 100,
            initial_auth_tokens: 100,
            reserved_auth_rate_per_second: 8,
            reserved_auth_burst: 8,
            initial_reserved_auth_tokens: 8,
            pre_auth_source_capacity: 64,
            pre_auth_source_ttl_ms: 60_000,
            pre_auth_failures_per_window: 8,
            principal_state_capacity: 64,
            low_flow: FlowClassLimit::new(1_000_000, 1_000_000),
            default_flow: FlowClassLimit::new(1_000_000, 1_000_000),
            high_flow: FlowClassLimit::new(1_000_000, 1_000_000),
            global_rate_per_second: 4_000_000,
            global_burst: 4_000_000,
            throttle_cooldown_ms: 1_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AdmissionSnapshot {
    pub reserved_auth_tokens_milli: u128,
    pub reserved_auth_workers_available: usize,
}

impl<C: MonotonicClock> AdmissionController<C> {
    pub(crate) fn pre_auth_source_count(&self) -> usize {
        self.state
            .lock()
            .expect("admission mutex poisoned")
            .sources
            .len()
    }

    pub(crate) fn snapshot(&self) -> AdmissionSnapshot {
        AdmissionSnapshot {
            reserved_auth_tokens_milli: self
                .state
                .lock()
                .expect("admission mutex poisoned")
                .reserved_auth
                .tokens_milli,
            reserved_auth_workers_available: self.reserved_workers.available_permits(),
        }
    }

    pub(crate) fn seed_drop_count_for_test(&self, value: u64) {
        self.state
            .lock()
            .expect("admission mutex poisoned")
            .throttled_drop_count = value;
    }

    pub(crate) fn record_throttled_drop_for_test(&self) {
        self.record_drop(self.clock.now_ms());
    }
}
