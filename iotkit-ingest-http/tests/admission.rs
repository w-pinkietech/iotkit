use std::net::{IpAddr, Ipv4Addr};

use crate::{AdmissionConfig, AdmissionController, ManualMonotonicClock};

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
    assert!(admission.try_begin_auth(peer, false).is_err());
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
