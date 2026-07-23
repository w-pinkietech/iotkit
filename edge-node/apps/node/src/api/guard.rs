use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub fn is_private_source(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => {
            let seg = ip.segments();
            ip.is_loopback() || (seg[0] & 0xfe00) == 0xfc00 || (seg[0] & 0xffc0) == 0xfe80
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryAfter {
    pub duration: Duration,
}

#[derive(Debug, Default)]
pub struct Throttle {
    inner: Mutex<ThrottleState>,
}

#[derive(Debug, Default)]
struct ThrottleState {
    sources: HashMap<IpAddr, SourceState>,
    global: VecDeque<Instant>,
}

#[derive(Debug, Clone, Copy)]
struct SourceState {
    failures: u32,
    blocked_until: Instant,
}

impl Throttle {
    pub fn check_and_record_source(&self, ip: IpAddr) -> Result<(), RetryAfter> {
        let now = Instant::now();
        let mut state = self.inner.lock().expect("throttle mutex poisoned");

        if let Some(source) = state.sources.get(&ip)
            && now < source.blocked_until
        {
            return Err(RetryAfter {
                duration: source.blocked_until - now,
            });
        }

        while state
            .global
            .front()
            .is_some_and(|seen| now.duration_since(*seen) >= Duration::from_secs(1))
        {
            state.global.pop_front();
        }
        if state.global.len() >= 10 {
            return Err(RetryAfter {
                duration: Duration::from_secs(1),
            });
        }
        state.global.push_back(now);
        Ok(())
    }

    pub fn record_failure(&self, ip: IpAddr) {
        let now = Instant::now();
        let mut state = self.inner.lock().expect("throttle mutex poisoned");
        let source = state.sources.entry(ip).or_insert(SourceState {
            failures: 0,
            blocked_until: now,
        });
        source.failures = source.failures.saturating_add(1);
        let secs = 2_u64.saturating_pow(source.failures).min(60);
        source.blocked_until = now + Duration::from_secs(secs);
    }

    pub fn record_success(&self, ip: IpAddr) {
        self.inner
            .lock()
            .expect("throttle mutex poisoned")
            .sources
            .remove(&ip);
    }
}
