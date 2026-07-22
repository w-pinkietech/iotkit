use std::net::{IpAddr, Ipv4Addr};

use crate::admission::test_support::ManualMonotonicClock;
use crate::{AdmissionConfig, AdmissionController};

#[test]
fn pre_auth_source_state_is_bounded_and_restart_is_conservative() {
    let clock = ManualMonotonicClock::new(0);
    let cardinality_config = AdmissionConfig::for_test()
        .with_pre_auth_source_capacity(2)
        .with_initial_auth_tokens(100);
    let admission = AdmissionController::new(cardinality_config, clock.clone()).unwrap();

    assert_eq!(admission.pre_auth_source_count(), 0);
    for octet in 1..=16 {
        let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, octet));
        let _ = admission.try_begin_auth(peer, false);
        assert!(admission.pre_auth_source_count() <= 2);
    }
    assert_eq!(
        admission.pre_auth_source_count(),
        2,
        "the test must actually fill the bounded peer map"
    );

    let restart_config = AdmissionConfig::for_test()
        .with_pre_auth_source_capacity(2)
        .with_initial_auth_tokens(0);
    let restarted = AdmissionController::new(restart_config, clock).unwrap();
    assert!(
        restarted
            .try_begin_auth(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 99)), false)
            .is_err(),
        "restart must not begin with a full authentication burst"
    );
}

#[test]
fn recently_validated_client_has_a_bounded_reserved_auth_lane() {
    let clock = ManualMonotonicClock::new(0);
    let admission = AdmissionController::new(
        AdmissionConfig::for_test()
            .with_auth_workers(1)
            .with_reserved_auth_workers(1),
        clock,
    )
    .unwrap();
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2));

    let _general = admission.try_begin_auth(peer, false).unwrap();
    admission.record_throttled_drop_for_test();
    let _reserved = admission.try_begin_auth(peer, true).unwrap();
    assert!(admission.try_begin_auth(peer, true).is_err());
}

#[test]
fn invalid_churn_cannot_exhaust_recently_validated_auth_work_slice() {
    let clock = ManualMonotonicClock::new(0);
    let admission = AdmissionController::new(AdmissionConfig::for_test(), clock).unwrap();
    for octet in 1..=100 {
        let peer = IpAddr::V4(Ipv4Addr::new(10, 0, 0, octet));
        drop(admission.try_begin_auth(peer, false).unwrap());
    }
    assert!(
        admission
            .try_begin_auth(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 1)), false)
            .is_err()
    );
    assert!(
        admission
            .try_begin_auth(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 2)), true)
            .is_ok()
    );
}

#[test]
fn thousands_of_drops_emit_one_start_and_one_recovery_after_hysteresis_cooldown() {
    let clock = ManualMonotonicClock::new(0);
    let admission = AdmissionController::new(
        AdmissionConfig::for_test()
            .with_auth_work_limit(10, 1)
            .with_initial_auth_tokens(0)
            .with_throttle_cooldown_ms(1_000),
        clock.clone(),
    )
    .unwrap();
    let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 44));

    for _ in 0..10_000 {
        assert!(admission.try_begin_auth(peer, false).is_err());
    }
    assert_eq!(admission.health_snapshot().throttled_drop_count, 10_000);
    assert_eq!(
        admission.pending_episode_events(),
        vec![crate::ThrottleEpisodeEvent::Started { episode_id: 1 }]
    );

    clock.advance_ms(999);
    let _ = admission.health_snapshot();
    assert_eq!(admission.pending_episode_events().len(), 1);
    clock.advance_ms(1);
    let health = admission.health_snapshot();
    assert!(!health.throttle_active);
    assert_eq!(
        admission.pending_episode_events(),
        vec![
            crate::ThrottleEpisodeEvent::Started { episode_id: 1 },
            crate::ThrottleEpisodeEvent::Recovered {
                episode_id: 1,
                drops: 10_000,
            },
        ]
    );
}

#[test]
fn aggregate_drop_and_queue_counters_saturate_without_identity_cardinality() {
    let clock = ManualMonotonicClock::new(0);
    let admission = AdmissionController::new(AdmissionConfig::for_test(), clock).unwrap();
    admission.seed_drop_count_for_test(u64::MAX - 1);
    admission.note_queue_depth(7);
    admission.note_queue_depth(3);
    admission.record_throttled_drop_for_test();
    admission.record_throttled_drop_for_test();

    let health = admission.health_snapshot();
    assert_eq!(health.throttled_drop_count, u64::MAX);
    assert_eq!(health.queue_high_water, 7);
    assert!(health.source_state_count <= 64);
    assert!(health.principal_state_count <= 64);
}

#[test]
fn post_decode_reconciliation_throttle_enters_the_same_aggregate_episode() {
    let clock = ManualMonotonicClock::new(10);
    let admission = AdmissionController::new(
        AdmissionConfig::for_test()
            .with_principal_flow_limit(
                4,
                crate::FlowClassLimit::new(10, 10),
                crate::FlowClassLimit::new(10, 10),
                crate::FlowClassLimit::new(10, 10),
            )
            .with_global_flow_limit(10, 10),
        clock.clone(),
    )
    .unwrap();
    let reservation = admission.reserve_principal("p1", "default", 1).unwrap();
    clock.advance_ms(5);
    assert!(reservation.reconcile_at(20, 1, 15).is_err());
    assert_eq!(admission.health_snapshot().throttled_drop_count, 1);
    assert_eq!(
        admission.pending_episode_events(),
        vec![crate::ThrottleEpisodeEvent::Started { episode_id: 1 }]
    );
}

#[test]
fn recovery_snapshot_ack_preserves_a_concurrent_new_episode_in_order() {
    let clock = ManualMonotonicClock::new(0);
    let admission = AdmissionController::new(
        AdmissionConfig::for_test()
            .with_auth_work_limit(10, 1)
            .with_initial_auth_tokens(0)
            .with_throttle_cooldown_ms(1_000),
        clock.clone(),
    )
    .unwrap();
    admission.record_throttled_drop_for_test();
    clock.advance_ms(1_000);
    assert!(!admission.health_snapshot().throttle_active);
    let first_episode = admission.pending_episode_events();
    assert_eq!(first_episode.len(), 2);

    admission.record_throttled_drop_for_test();
    assert!(admission.acknowledge_episode_events(&first_episode));
    let remaining = admission.pending_episode_events();
    assert_eq!(remaining.len(), 1);
    assert!(matches!(
        remaining[0],
        crate::ThrottleEpisodeEvent::Started { episode_id: 2 }
    ));
}

#[test]
fn failed_persistence_keeps_old_episode_and_never_overwrites_new_start() {
    let clock = ManualMonotonicClock::new(0);
    let admission = AdmissionController::new(
        AdmissionConfig::for_test()
            .with_auth_work_limit(10, 1)
            .with_initial_auth_tokens(0)
            .with_throttle_cooldown_ms(1_000),
        clock.clone(),
    )
    .unwrap();
    admission.record_throttled_drop_for_test();
    clock.advance_ms(1_000);
    let _ = admission.health_snapshot();
    let failed_batch = admission.pending_episode_events();
    // Persistence failed: deliberately do not acknowledge failed_batch.
    admission.record_throttled_drop_for_test();

    let retry = admission.pending_episode_events();
    assert_eq!(retry.len(), 3);
    assert_eq!(&retry[..failed_batch.len()], failed_batch.as_slice());
    assert!(matches!(
        retry[2],
        crate::ThrottleEpisodeEvent::Started { episode_id: 2 }
    ));
    assert!(admission.acknowledge_episode_events(&retry));
    assert!(admission.pending_episode_events().is_empty());
}

#[test]
fn aggregate_capacity_pressure_prevents_premature_recovery() {
    let clock = ManualMonotonicClock::new(0);
    let admission = AdmissionController::new(
        AdmissionConfig::for_test().with_throttle_cooldown_ms(1_000),
        clock.clone(),
    )
    .unwrap();
    admission.record_throttled_drop_for_test();
    admission.note_current_capacity_pressure(7, 8, 7, 8, 7, 8);
    clock.advance_ms(1_000);

    let pressured = admission.health_snapshot();
    assert!(pressured.throttle_active);
    assert_eq!(pressured.queue_current, 7);
    assert!(pressured.queue_pressure_percent >= 87);
    assert!(pressured.request_pressure_percent >= 87);
    assert!(pressured.connection_pressure_percent >= 87);

    admission.note_current_capacity_pressure(0, 8, 0, 8, 0, 8);
    let recovered = admission.health_snapshot();
    assert!(!recovered.throttle_active);
}

#[test]
fn principal_pressure_refills_on_clock_and_then_allows_recovery() {
    let clock = ManualMonotonicClock::new(0);
    let flow = crate::FlowClassLimit::new(10, 10);
    let admission = AdmissionController::new(
        AdmissionConfig::for_test()
            .with_principal_flow_limit(4, flow, flow, flow)
            .with_throttle_cooldown_ms(1_000),
        clock.clone(),
    )
    .unwrap();
    let mut reservation = admission.reserve_principal("p1", "default", 8).unwrap();
    reservation.note_consumed_bytes(8);
    reservation.reconcile(0, 0).unwrap();
    admission.record_throttled_drop_for_test();

    clock.advance_ms(1_000);
    let recovered = admission.health_snapshot();
    assert!(recovered.principal_pressure_percent < 50);
    assert!(!recovered.throttle_active);
}

#[test]
fn unacknowledged_episode_state_remains_bounded_without_overwrite() {
    let clock = ManualMonotonicClock::new(0);
    let admission = AdmissionController::new(
        AdmissionConfig::for_test().with_throttle_cooldown_ms(1_000),
        clock.clone(),
    )
    .unwrap();
    admission.record_throttled_drop_for_test();
    clock.advance_ms(1_000);
    let _ = admission.health_snapshot();
    admission.record_throttled_drop_for_test();
    for _ in 0..10_000 {
        clock.advance_ms(1_000);
        let _ = admission.health_snapshot();
        admission.record_throttled_drop_for_test();
        assert!(admission.pending_episode_events().len() <= 3);
    }
    assert_eq!(admission.pending_episode_events().len(), 3);
}
