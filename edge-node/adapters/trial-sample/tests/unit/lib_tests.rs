use super::*;

#[test]
fn descriptor_is_explicitly_a_trial_source() {
    let descriptor = descriptor();
    assert_eq!(descriptor.adapter_type_id.as_str(), "trial-sample");
    assert_eq!(descriptor.config_schema_version, 1);
    assert_eq!(
        descriptor.physical_transport_kind,
        PhysicalTransportKind::Other
    );
}

#[test]
fn readings_change_and_use_the_declared_measurement() {
    let first = reading("trial:sample", 1);
    let second = reading("trial:sample", 2);
    assert_eq!(first.subject_hint.as_deref(), Some("trial:sample:sample"));
    assert_eq!(first.measurement_key, "illuminance_lux");
    assert_ne!(first.values, second.values);
}

#[test]
fn polling_interval_is_bounded() {
    assert!(
        validate(TrialSampleConfig {
            poll_interval_ms: 249
        })
        .is_err()
    );
    assert!(
        validate(TrialSampleConfig {
            poll_interval_ms: 1_000
        })
        .is_ok()
    );
    assert!(
        validate(TrialSampleConfig {
            poll_interval_ms: 60_001
        })
        .is_err()
    );
}
