//! Reusable conformance assertions and a test-only vendor-neutral reference adapter.

use iotkit_ingest_contract::{ReadingItem, TimeSource};
use iotkit_input_adapter_host_api::{
    AdapterInstanceId, ConfiguredSource, InputAdapterTypeDescriptor,
};

pub fn assert_descriptor_v1(descriptor: &InputAdapterTypeDescriptor) {
    assert_eq!(descriptor.adapter_api_major, 1);
    assert_eq!(descriptor.config_schema_version, 1);
    assert!(!descriptor.display_name.is_empty());
    assert!(!descriptor.implementation_version.is_empty());
}

pub fn assert_finite_items(items: &[ReadingItem]) {
    assert!(!items.is_empty());
    for item in items {
        assert!(
            item.subject_hint
                .as_ref()
                .is_some_and(|value| !value.is_empty())
        );
        assert!(!item.measurement_key.is_empty());
        assert!(!item.values.is_empty());
        assert!(item.values.iter().all(|value| value.is_finite()));
    }
}

pub fn assert_standard_registry_mapping(
    items: &[ReadingItem],
    expected_key: &str,
    expected_unit_ucum: Option<&str>,
) {
    assert_finite_items(items);
    let entry = iotkit_core_registry::standard_catalog()
        .find(expected_key)
        .unwrap_or_else(|| panic!("{expected_key} is absent from the standard registry"));
    assert_eq!(entry.unit_ucum.as_deref(), expected_unit_ucum);
    assert!(
        items
            .iter()
            .all(|item| item.measurement_key == expected_key),
        "emitted items must use the expected canonical measurement key"
    );
}

/// Test-only adapter model proving the contract does not require BravePI vocabulary.
pub struct ReferenceAdapter {
    pub instance_id: AdapterInstanceId,
    pub source: ConfiguredSource,
}

impl ReferenceAdapter {
    pub fn new() -> Self {
        Self {
            instance_id: AdapterInstanceId::new("reference_one").unwrap(),
            source: ConfiguredSource::new("input:reference:one").unwrap(),
        }
    }

    pub fn observations(&self) -> Vec<ReadingItem> {
        vec![
            ReadingItem {
                subject_hint: Some(format!("{}:subject:a", self.source)),
                measurement_key: "temperature_c".into(),
                channel_index: None,
                series_variant: None,
                values: vec![20.5],
                device_time_ms: None,
                time_source: TimeSource::EdgeNode,
                age_ms: None,
                rssi: None,
                battery_pct: None,
            },
            ReadingItem {
                subject_hint: Some(format!("{}:subject:b", self.source)),
                measurement_key: "contact_state".into(),
                channel_index: None,
                series_variant: None,
                values: vec![1.0],
                device_time_ms: None,
                time_source: TimeSource::EdgeNode,
                age_ms: None,
                rssi: None,
                battery_pct: None,
            },
        ]
    }
}

impl Default for ReferenceAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../tests/unit/lib_tests.rs"]
mod tests;
