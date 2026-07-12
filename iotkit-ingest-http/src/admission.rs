use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

mod sealed {
    pub trait Sealed {}
}

pub trait MonotonicClock: sealed::Sealed + Clone + Send + Sync + 'static {
    fn now_ms(&self) -> u64;
}

#[derive(Clone)]
pub struct SystemMonotonicClock {
    start: Arc<std::time::Instant>,
}

impl Default for SystemMonotonicClock {
    fn default() -> Self {
        Self {
            start: Arc::new(std::time::Instant::now()),
        }
    }
}

impl sealed::Sealed for SystemMonotonicClock {}

impl MonotonicClock for SystemMonotonicClock {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct ManualMonotonicClock(Arc<AtomicU64>);

#[cfg(test)]
impl ManualMonotonicClock {
    pub(crate) fn new(now_ms: u64) -> Self {
        Self(Arc::new(AtomicU64::new(now_ms)))
    }

    pub(crate) fn advance_ms(&self, elapsed_ms: u64) {
        self.0.fetch_add(elapsed_ms, Ordering::SeqCst);
    }
}

#[cfg(test)]
impl sealed::Sealed for ManualMonotonicClock {}

#[cfg(test)]
impl MonotonicClock for ManualMonotonicClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone)]
pub struct AdmissionConfig {
    auth_workers: usize,
    reserved_auth_workers: usize,
    auth_rate_per_second: u64,
    auth_burst: u64,
    initial_auth_tokens: u64,
    reserved_auth_rate_per_second: u64,
    reserved_auth_burst: u64,
    initial_reserved_auth_tokens: u64,
    pre_auth_source_capacity: usize,
    pre_auth_source_ttl_ms: u64,
    pre_auth_failures_per_window: u32,
    principal_state_capacity: usize,
    low_flow: FlowClassLimit,
    default_flow: FlowClassLimit,
    high_flow: FlowClassLimit,
    global_rate_per_second: u64,
    global_burst: u64,
}

impl AdmissionConfig {
    #[cfg(test)]
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
        }
    }

    pub fn with_auth_workers(mut self, value: usize) -> Self {
        self.auth_workers = value;
        self
    }

    pub fn with_reserved_auth_workers(mut self, value: usize) -> Self {
        self.reserved_auth_workers = value;
        self
    }

    pub fn with_pre_auth_source_capacity(mut self, value: usize) -> Self {
        self.pre_auth_source_capacity = value;
        self
    }

    /// A named conservative-start semantic. Zero is deliberately allowed here:
    /// it means authentication work must wait for monotonic refill after restart.
    pub fn with_initial_auth_tokens(mut self, value: u64) -> Self {
        self.initial_auth_tokens = value;
        self
    }

    pub fn with_auth_work_limit(mut self, rate_per_second: u64, burst: u64) -> Self {
        self.auth_rate_per_second = rate_per_second;
        self.auth_burst = burst;
        self
    }

    pub fn with_reserved_auth_work_limit(
        mut self,
        rate_per_second: u64,
        burst: u64,
        initial_tokens: u64,
    ) -> Self {
        self.reserved_auth_rate_per_second = rate_per_second;
        self.reserved_auth_burst = burst;
        self.initial_reserved_auth_tokens = initial_tokens;
        self
    }

    pub fn with_pre_auth_source_limits(
        mut self,
        capacity: usize,
        ttl_ms: u64,
        failures_per_window: u32,
    ) -> Self {
        self.pre_auth_source_capacity = capacity;
        self.pre_auth_source_ttl_ms = ttl_ms;
        self.pre_auth_failures_per_window = failures_per_window;
        self
    }

    pub fn with_principal_flow_limit(
        mut self,
        state_capacity: usize,
        low: FlowClassLimit,
        default: FlowClassLimit,
        high: FlowClassLimit,
    ) -> Self {
        self.principal_state_capacity = state_capacity;
        self.low_flow = low;
        self.default_flow = default;
        self.high_flow = high;
        self
    }

    pub fn with_global_flow_limit(mut self, rate_per_second: u64, burst: u64) -> Self {
        self.global_rate_per_second = rate_per_second;
        self.global_burst = burst;
        self
    }

    fn validate(&self) -> Result<(), InvalidAdmissionConfig> {
        if self.auth_workers == 0
            || self.reserved_auth_workers == 0
            || self.auth_rate_per_second == 0
            || self.auth_burst == 0
            || self.initial_auth_tokens > self.auth_burst
            || self.reserved_auth_rate_per_second == 0
            || self.reserved_auth_burst == 0
            || self.initial_reserved_auth_tokens > self.reserved_auth_burst
            || self.pre_auth_source_capacity == 0
            || self.pre_auth_source_ttl_ms == 0
            || self.pre_auth_failures_per_window == 0
            || self.principal_state_capacity == 0
            || !self.low_flow.valid()
            || !self.default_flow.valid()
            || !self.high_flow.valid()
            || self.global_rate_per_second == 0
            || self.global_burst == 0
        {
            return Err(InvalidAdmissionConfig);
        }
        Ok(())
    }
}

impl Default for AdmissionConfig {
    fn default() -> Self {
        Self {
            auth_workers: 2,
            reserved_auth_workers: 1,
            auth_rate_per_second: 16,
            auth_burst: 32,
            initial_auth_tokens: 1,
            reserved_auth_rate_per_second: 8,
            reserved_auth_burst: 8,
            initial_reserved_auth_tokens: 1,
            pre_auth_source_capacity: 1024,
            pre_auth_source_ttl_ms: 60_000,
            pre_auth_failures_per_window: 8,
            principal_state_capacity: 64,
            low_flow: FlowClassLimit::new(1_000_000, 1_000_000),
            default_flow: FlowClassLimit::new(1_000_000, 1_000_000),
            high_flow: FlowClassLimit::new(1_000_000, 1_000_000),
            global_rate_per_second: 4_000_000,
            global_burst: 4_000_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidAdmissionConfig;

impl std::fmt::Display for InvalidAdmissionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("admission limits must be finite and positive")
    }
}

impl std::error::Error for InvalidAdmissionConfig {}

struct TokenBucket {
    tokens_milli: u128,
    last_ms: u64,
}

#[derive(Clone, Copy)]
struct SourceState {
    failures: u32,
    expires_at_ms: u64,
}

struct AdmissionState {
    auth: TokenBucket,
    reserved_auth: TokenBucket,
    sources: HashMap<IpAddr, SourceState>,
    global_flow: TokenBucket,
    principals: HashMap<String, TokenBucket>,
}

pub struct AdmissionController<C: MonotonicClock = SystemMonotonicClock> {
    config: AdmissionConfig,
    clock: C,
    state: Arc<Mutex<AdmissionState>>,
    general_workers: Arc<Semaphore>,
    reserved_workers: Arc<Semaphore>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AdmissionSnapshot {
    pub reserved_auth_tokens_milli: u128,
    pub reserved_auth_workers_available: usize,
}

impl<C: MonotonicClock> AdmissionController<C> {
    pub fn new(config: AdmissionConfig, clock: C) -> Result<Self, InvalidAdmissionConfig> {
        config.validate()?;
        let now = clock.now_ms();
        Ok(Self {
            state: Arc::new(Mutex::new(AdmissionState {
                auth: TokenBucket {
                    tokens_milli: u128::from(config.initial_auth_tokens) * 1000,
                    last_ms: now,
                },
                reserved_auth: TokenBucket {
                    tokens_milli: u128::from(config.initial_reserved_auth_tokens) * 1000,
                    last_ms: now,
                },
                sources: HashMap::with_capacity(config.pre_auth_source_capacity),
                global_flow: TokenBucket {
                    tokens_milli: u128::from(config.global_burst) * 1000,
                    last_ms: now,
                },
                principals: HashMap::with_capacity(config.principal_state_capacity),
            })),
            general_workers: Arc::new(Semaphore::new(config.auth_workers)),
            reserved_workers: Arc::new(Semaphore::new(config.reserved_auth_workers)),
            config,
            clock,
        })
    }

    #[cfg(test)]
    pub(crate) fn pre_auth_source_count(&self) -> usize {
        self.state
            .lock()
            .expect("admission mutex poisoned")
            .sources
            .len()
    }

    #[cfg(test)]
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

    pub fn try_begin_auth(
        &self,
        peer: IpAddr,
        recently_validated: bool,
    ) -> Result<AuthPermit, AdmissionDenied> {
        let now = self.clock.now_ms();
        {
            let mut state = self.state.lock().expect("admission mutex poisoned");
            state.sources.retain(|_, source| source.expires_at_ms > now);
            if let Some(source) = state.sources.get(&peer)
                && source.failures >= self.config.pre_auth_failures_per_window
            {
                return Err(AdmissionDenied::Throttled);
            }
            refill(&mut state.auth, &self.config, now);
            refill_rate(
                &mut state.reserved_auth,
                self.config.reserved_auth_rate_per_second,
                self.config.reserved_auth_burst,
                now,
            );
            if state.auth.tokens_milli >= 1000 {
                state.auth.tokens_milli -= 1000;
            } else if recently_validated && state.reserved_auth.tokens_milli >= 1000 {
                state.reserved_auth.tokens_milli -= 1000;
            } else {
                return Err(AdmissionDenied::Throttled);
            }
            if state.sources.len() < self.config.pre_auth_source_capacity {
                state.sources.entry(peer).or_insert(SourceState {
                    failures: 0,
                    expires_at_ms: now.saturating_add(self.config.pre_auth_source_ttl_ms),
                });
            }
        }

        let permit = self
            .general_workers
            .clone()
            .try_acquire_owned()
            .or_else(|_| {
                if recently_validated {
                    self.reserved_workers.clone().try_acquire_owned()
                } else {
                    Err(tokio::sync::TryAcquireError::NoPermits)
                }
            });
        permit
            .map(|permit| AuthPermit { _permit: permit })
            .map_err(|_| AdmissionDenied::Busy)
    }

    pub fn record_auth_failure(&self, peer: IpAddr) {
        let now = self.clock.now_ms();
        let mut state = self.state.lock().expect("admission mutex poisoned");
        if let Some(source) = state.sources.get_mut(&peer) {
            source.failures = source.failures.saturating_add(1);
            source.expires_at_ms = now.saturating_add(self.config.pre_auth_source_ttl_ms);
        }
    }

    pub fn reserve_principal(
        &self,
        principal_id: &str,
        flow_class: &str,
        maximum_cost: u64,
    ) -> Result<PrincipalReservation, AdmissionDenied> {
        if maximum_cost == 0 {
            return Err(AdmissionDenied::Throttled);
        }
        let now = self.clock.now_ms();
        let flow = match flow_class {
            "low" => self.config.low_flow,
            "default" => self.config.default_flow,
            "high" => self.config.high_flow,
            _ => return Err(AdmissionDenied::Throttled),
        };
        let mut state = self.state.lock().expect("admission mutex poisoned");
        refill_rate(
            &mut state.global_flow,
            self.config.global_rate_per_second,
            self.config.global_burst,
            now,
        );
        if !state.principals.contains_key(principal_id)
            && state.principals.len() >= self.config.principal_state_capacity
        {
            return Err(AdmissionDenied::Throttled);
        }
        {
            let principal =
                state
                    .principals
                    .entry(principal_id.to_owned())
                    .or_insert(TokenBucket {
                        tokens_milli: u128::from(flow.burst) * 1000,
                        last_ms: now,
                    });
            refill_rate(principal, flow.rate_per_second, flow.burst, now);
        }
        let cost = u128::from(maximum_cost) * 1000;
        if state.global_flow.tokens_milli < cost
            || state
                .principals
                .get(principal_id)
                .is_none_or(|principal| principal.tokens_milli < cost)
        {
            return Err(AdmissionDenied::Throttled);
        }
        state.global_flow.tokens_milli -= cost;
        state
            .principals
            .get_mut(principal_id)
            .expect("principal inserted above")
            .tokens_milli -= cost;
        Ok(PrincipalReservation {
            state: Arc::clone(&self.state),
            principal_id: principal_id.to_owned(),
            reserved_milli: cost,
            consumed_milli: 1000,
            global_capacity_milli: u128::from(self.config.global_burst) * 1000,
            principal_capacity_milli: u128::from(flow.burst) * 1000,
            settled: false,
        })
    }
}

fn refill(bucket: &mut TokenBucket, config: &AdmissionConfig, now: u64) {
    let elapsed = now.saturating_sub(bucket.last_ms);
    bucket.last_ms = now;
    let added = u128::from(elapsed).saturating_mul(u128::from(config.auth_rate_per_second));
    let capacity = u128::from(config.auth_burst) * 1000;
    bucket.tokens_milli = bucket.tokens_milli.saturating_add(added).min(capacity);
}

fn refill_rate(bucket: &mut TokenBucket, rate: u64, burst: u64, now: u64) {
    let elapsed = now.saturating_sub(bucket.last_ms);
    bucket.last_ms = now;
    let added = u128::from(elapsed).saturating_mul(u128::from(rate));
    bucket.tokens_milli = bucket
        .tokens_milli
        .saturating_add(added)
        .min(u128::from(burst) * 1000);
}

pub struct PrincipalReservation {
    state: Arc<Mutex<AdmissionState>>,
    principal_id: String,
    reserved_milli: u128,
    consumed_milli: u128,
    global_capacity_milli: u128,
    principal_capacity_milli: u128,
    settled: bool,
}

impl PrincipalReservation {
    /// Record irreversible body work as chunks are consumed. Values saturate at the reservation.
    pub fn note_consumed_bytes(&mut self, bytes: usize) {
        self.consumed_milli = self
            .consumed_milli
            .saturating_add((bytes as u128).saturating_mul(1000))
            .min(self.reserved_milli);
    }

    /// Finish two-stage charging. The fixed request unit, decoded bytes, and item work remain
    /// consumed; only unused conservative reservation is refunded.
    pub fn reconcile(mut self, decoded_bytes: usize, items: usize) -> Result<(), AdmissionDenied> {
        let actual = 1_u128
            .saturating_add(decoded_bytes as u128)
            .saturating_add((items as u128).saturating_mul(256))
            .saturating_mul(1000);
        if actual > self.reserved_milli {
            let additional = actual - self.reserved_milli;
            let mut state = self.state.lock().expect("admission mutex poisoned");
            let principal_has = state
                .principals
                .get(&self.principal_id)
                .is_some_and(|principal| principal.tokens_milli >= additional);
            if state.global_flow.tokens_milli < additional || !principal_has {
                self.consumed_milli = self.reserved_milli;
                self.settled = true;
                return Err(AdmissionDenied::Throttled);
            }
            state.global_flow.tokens_milli -= additional;
            state
                .principals
                .get_mut(&self.principal_id)
                .expect("reservation principal exists")
                .tokens_milli -= additional;
            self.reserved_milli = actual;
        }
        self.consumed_milli = self.consumed_milli.max(actual);
        self.refund_unused();
        self.settled = true;
        Ok(())
    }

    fn refund_unused(&mut self) {
        let refund = self.reserved_milli.saturating_sub(self.consumed_milli);
        if refund == 0 {
            return;
        }
        let mut state = self.state.lock().expect("admission mutex poisoned");
        state.global_flow.tokens_milli = state
            .global_flow
            .tokens_milli
            .saturating_add(refund)
            .min(self.global_capacity_milli);
        if let Some(principal) = state.principals.get_mut(&self.principal_id) {
            principal.tokens_milli = principal
                .tokens_milli
                .saturating_add(refund)
                .min(self.principal_capacity_milli);
        }
        self.reserved_milli = self.consumed_milli;
    }
}

impl Drop for PrincipalReservation {
    fn drop(&mut self) {
        if !self.settled {
            self.refund_unused();
        }
    }
}

pub struct AuthPermit {
    _permit: OwnedSemaphorePermit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionDenied {
    Throttled,
    Busy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowClassLimit {
    rate_per_second: u64,
    burst: u64,
}

impl FlowClassLimit {
    pub const fn new(rate_per_second: u64, burst: u64) -> Self {
        Self {
            rate_per_second,
            burst,
        }
    }

    const fn valid(self) -> bool {
        self.rate_per_second > 0 && self.burst > 0
    }
}
