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
    assert_eq!(ILLUMINANCE_MODEL_ID, "trial-sample-illuminance");
    assert_eq!(CONTACT_MODEL_ID, "trial-sample-contact");
    assert_ne!(ILLUMINANCE_MODEL_ID, "opt3001");
    assert_ne!(CONTACT_MODEL_ID, "contact");
    assert_eq!(ENABLE_ENV, "IOTKIT_ENABLE_TRIAL_SAMPLE");
}

#[test]
fn illuminance_triangle_changes_and_uses_declared_measurement() {
    let first = illuminance_reading("trial:sample", 1);
    let second = illuminance_reading("trial:sample", 2);
    assert_eq!(first.subject_hint.as_deref(), Some("trial:sample:sample"));
    assert_eq!(first.measurement_key, "illuminance_lux");
    assert_eq!(first.values.len(), 1);
    assert_ne!(first.values, second.values);
}

#[test]
fn contact_square_wave_toggles_high_and_low() {
    let half = DEFAULT_STATE_HALF_PERIOD_POLLS;
    let low = contact_reading("trial:sample", half);
    let high = contact_reading("trial:sample", half + 1);
    let high_again = contact_reading("trial:sample", half * 2);
    let low_again = contact_reading("trial:sample", half * 2 + 1);

    assert_eq!(low.subject_hint.as_deref(), Some("trial:sample:state"));
    assert_eq!(low.measurement_key, "contact_state");
    assert_eq!(low.values, vec![0.0]);
    assert_eq!(high.values, vec![1.0]);
    assert_eq!(high_again.values, vec![1.0]);
    assert_eq!(low_again.values, vec![0.0]);
}

#[test]
fn each_poll_emits_both_series() {
    let items = readings("trial:sample", 3);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].measurement_key, "illuminance_lux");
    assert_eq!(items[1].measurement_key, "contact_state");
    assert_eq!(
        items[0].subject_hint.as_deref(),
        Some("trial:sample:sample")
    );
    assert_eq!(items[1].subject_hint.as_deref(), Some("trial:sample:state"));
}

#[test]
fn inventory_lists_non_hardware_models_for_both_series() {
    let items = inventory_items("trial:sample");
    assert_eq!(
        items,
        [
            InventoryItem {
                hardware_id: "trial:sample:sample".into(),
                model_id: ILLUMINANCE_MODEL_ID.into(),
                label: ILLUMINANCE_LABEL.into(),
            },
            InventoryItem {
                hardware_id: "trial:sample:state".into(),
                model_id: CONTACT_MODEL_ID.into(),
                label: CONTACT_LABEL.into(),
            },
        ]
    );
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
