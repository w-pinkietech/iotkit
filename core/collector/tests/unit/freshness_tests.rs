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
