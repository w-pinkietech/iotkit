use super::*;

fn raw(adapter_type: &str) -> RawInputAdapterInstance {
    RawInputAdapterInstance {
        adapter_type: adapter_type.into(),
        enabled: Some(true),
        config_schema_version: 1,
        source: "input:test:line_a".into(),
        port: None,
        bus_path: None,
        poll_interval_ms: None,
        devices: None,
    }
}

#[test]
fn built_in_catalog_is_unique_and_uses_host_api_v1() {
    validate_catalog().unwrap();
    let ids: Vec<_> = catalog()
        .into_iter()
        .map(|factory| ((factory.descriptor)()).adapter_type_id.as_str().to_owned())
        .collect();
    assert_eq!(ids, ["bravepi-mainboard", "rpi-local", "trial-sample"]);
}

#[test]
fn trial_sample_factory_has_inventory_and_rejects_hardware_fields() {
    let mut sample = raw("trial-sample");
    sample.poll_interval_ms = Some(1_000);
    let prepared = resolve_instance("trial_sample".into(), sample)
        .unwrap()
        .unwrap();
    assert_eq!(
        prepared.positional_inventory(),
        [PositionalInventoryItem {
            hardware_id: "input:test:line_a:sample".into(),
            model_id: "opt3001".into(),
            label: "Trial illuminance sensor".into(),
        }]
    );

    let mut invalid = raw("trial-sample");
    invalid.poll_interval_ms = Some(1_000);
    invalid.bus_path = Some("/dev/i2c-1".into());
    assert!(
        resolve_instance("trial_sample".into(), invalid)
            .unwrap_err()
            .contains("trial-sample-only")
    );
}

#[test]
fn rpi_factory_validates_driver_limits_before_returning_prepared_instance() {
    let mut raw = raw("rpi-local");
    raw.bus_path = Some("/dev/i2c-1".into());
    raw.poll_interval_ms = Some(50);
    let error = resolve_instance("line_a".into(), raw).unwrap_err();
    assert!(error.contains("poll_interval_ms"));
}

#[test]
fn rpi_inventory_and_runtime_share_the_same_validated_targets() {
    let mut raw = raw("rpi-local");
    raw.bus_path = Some("/dev/i2c-1".into());
    raw.poll_interval_ms = Some(1_000);
    let prepared = resolve_instance("line_a".into(), raw).unwrap().unwrap();
    let inventory = prepared.positional_inventory();
    assert_eq!(inventory.len(), 2);
    assert_eq!(inventory[0].hardware_id, "input:test:line_a:i2c:0x60");
    assert_eq!(inventory[1].hardware_id, "input:test:line_a:i2c:0x44");
}
