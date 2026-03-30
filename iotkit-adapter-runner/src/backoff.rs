/// Backoff calculator for reconnect delays (spec 3.5).
///
/// Formula: delay = clamp(base_ms * 2^min(attempt, 15) + jitter, 100ms, max_ms)
/// Jitter: +/-30% uniform random on the capped value.
pub(crate) struct Backoff {
    attempt: u32,
    base_ms: u64,
    max_ms: u64,
}

impl Backoff {
    pub fn new() -> Self {
        Self {
            attempt: 0,
            base_ms: 1000,
            max_ms: 30_000,
        }
    }

    /// Calculate the next delay and increment the attempt counter.
    pub fn next_delay(&mut self) -> std::time::Duration {
        let exp = self.attempt.min(15);
        let base_delay = self.base_ms.saturating_mul(1u64 << exp).min(self.max_ms);

        // +/-30% jitter
        let jitter_range = (base_delay as f64 * 0.3) as i64;
        let jitter = if jitter_range > 0 {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            rng.gen_range(-jitter_range..=jitter_range)
        } else {
            0
        };

        // Clamp: floor 100ms, ceiling max_ms (spec 3.5)
        let delay_ms = (base_delay as i64 + jitter).max(100).min(self.max_ms as i64) as u64;
        self.attempt = self.attempt.saturating_add(1);
        std::time::Duration::from_millis(delay_ms)
    }

    /// Reset attempt counter on successful ConnAck.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_0_delay_near_1s() {
        let mut b = Backoff::new();
        let d = b.next_delay();
        // 1000ms +/- 30% = [700, 1300]
        assert!(d.as_millis() >= 700, "delay too low: {:?}", d);
        assert!(d.as_millis() <= 1300, "delay too high: {:?}", d);
    }

    #[test]
    fn attempt_5_capped_at_30s() {
        let mut b = Backoff::new();
        for _ in 0..5 {
            b.next_delay();
        }
        // attempt 5: base = 1000 * 2^5 = 32000, capped at 30000
        let d = b.next_delay();
        assert!(d.as_millis() >= 100, "delay below floor: {:?}", d);
        assert!(d.as_millis() <= 30_000, "delay exceeds 30s max: {:?}", d);
    }

    #[test]
    fn delay_never_below_100ms() {
        let mut b = Backoff::new();
        for _ in 0..20 {
            let d = b.next_delay();
            assert!(d.as_millis() >= 100, "delay below floor: {:?}", d);
        }
    }

    #[test]
    fn reset_restarts_from_attempt_0() {
        let mut b = Backoff::new();
        for _ in 0..10 {
            b.next_delay();
        }
        b.reset();
        let d = b.next_delay();
        assert!(d.as_millis() >= 700, "after reset delay too low: {:?}", d);
        assert!(d.as_millis() <= 1300, "after reset delay too high: {:?}", d);
    }

    #[test]
    fn saturating_add_does_not_panic() {
        let mut b = Backoff::new();
        b.attempt = u32::MAX;
        let d = b.next_delay(); // should not panic
        assert!(d.as_millis() >= 100);
    }
}
