//! R20: アプリレベル監督。プロセスレベルはsystemdに委譲(責務台帳)。
use iotkit_core_types::AdapterId;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct RestartPolicy {
    pub max_restarts: u32,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            max_restarts: 5,
            base_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
        }
    }
}

/// アダプタごとの再起動回数を追跡し、指数バックオフの遅延を計算する。
pub struct RestartTracker {
    policy: RestartPolicy,
    counts: HashMap<AdapterId, u32>,
}

impl RestartTracker {
    pub fn new(policy: RestartPolicy) -> Self {
        Self { policy, counts: HashMap::new() }
    }

    /// 次の再起動までの待ち時間。予算超過ならNone(永続degraded)。
    pub fn next_delay(&mut self, id: &AdapterId) -> Option<Duration> {
        let count = self.counts.entry(id.clone()).or_insert(0);
        if *count >= self.policy.max_restarts {
            return None;
        }
        let delay = self
            .policy
            .base_backoff
            .saturating_mul(2u32.saturating_pow(*count))
            .min(self.policy.max_backoff);
        *count += 1;
        Some(delay)
    }

    /// 健全稼働を観測したら再起動カウンタをリセットする。
    pub fn note_healthy(&mut self, id: &AdapterId) {
        self.counts.remove(id);
    }
}

/// D1: グローバルpanicフックでbacktraceをログ(panic="abort"禁止はCargo.toml側で保証)
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(%info, %backtrace, "panic captured");
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    use iotkit_core_types::AdapterId;
    use std::time::Duration;

    #[test]
    fn backoff_grows_exponentially_with_cap_and_exhausts() {
        let policy = super::RestartPolicy {
            max_restarts: 3,
            base_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(4),
        };
        let mut t = super::RestartTracker::new(policy);
        let id = AdapterId::new("bravepi-mainboard:/dev/ttyAMA0");
        assert_eq!(t.next_delay(&id), Some(Duration::from_secs(1)));
        assert_eq!(t.next_delay(&id), Some(Duration::from_secs(2)));
        assert_eq!(t.next_delay(&id), Some(Duration::from_secs(4))); // cap
        assert_eq!(t.next_delay(&id), None); // exhausted → 永続degraded
    }

    #[test]
    fn healthy_note_resets_counter() {
        let mut t = super::RestartTracker::new(super::RestartPolicy::default());
        let id = AdapterId::new("a");
        t.next_delay(&id);
        t.note_healthy(&id);
        assert_eq!(t.next_delay(&id), Some(super::RestartPolicy::default().base_backoff));
    }

    #[test]
    fn workspace_does_not_use_panic_abort() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let toml = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(!toml.contains("panic = \"abort\""), "panic=abort breaks task supervision (D1)");
    }
}
