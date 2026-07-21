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
mod tests {
    use super::*;
    use iotkit_ingest_client::channel_for_test;
    use iotkit_input_adapter_host_api::{
        AdapterCompletion, AdapterDiagnostic, DiagnosticKind, SourceBoundIngest, runtime_channels,
    };

    #[tokio::test]
    async fn reference_adapter_proves_source_binding_and_two_subject_two_kind_support() {
        let reference = ReferenceAdapter::new();
        let items = reference.observations();
        assert_finite_items(&items);
        let (client, mut receiver) = channel_for_test(1);
        let ingest = SourceBoundIngest::new(reference.source.clone(), client);
        let enqueued = ingest.try_submit(items).unwrap();
        let envelope = receiver.recv().await.unwrap();
        assert_eq!(envelope.envelope_id, enqueued.envelope_id);
        assert_eq!(envelope.source, "input:reference:one");
        assert_eq!(
            envelope.items[0].subject_hint.as_deref(),
            Some("input:reference:one:subject:a")
        );
        assert_eq!(
            envelope.items[1].subject_hint.as_deref(),
            Some("input:reference:one:subject:b")
        );
        assert_ne!(
            envelope.items[0].measurement_key,
            envelope.items[1].measurement_key
        );
        assert_standard_registry_mapping(&envelope.items[..1], "temperature_c", Some("Cel"));
        assert_standard_registry_mapping(&envelope.items[1..], "contact_state", Some("1"));
    }

    #[tokio::test]
    async fn generic_runtime_channels_cover_activity_diagnostics_shutdown_and_completion() {
        let id = AdapterInstanceId::new("reference_lifecycle").unwrap();
        let (mut runtime, mut running) = runtime_channels(id, 1);
        runtime.activity.physical_decode();
        runtime.activity.queue_admission();
        assert!(running.activity.snapshot().last_physical_decode.is_some());
        assert!(running.activity.snapshot().last_queue_admission.is_some());

        runtime
            .diagnostics
            .try_emit(AdapterDiagnostic::new(
                DiagnosticKind::Transport,
                "password=must-not-escape",
            ))
            .unwrap();
        let diagnostic = running.diagnostics.recv().await.unwrap();
        assert_eq!(diagnostic.message, "[redacted]");

        running.shutdown.request();
        assert!(runtime.stop.changed().await);
        runtime
            .completion
            .complete(AdapterCompletion::RequestedStop);
        assert_eq!(
            running.completion.wait().await,
            AdapterCompletion::RequestedStop
        );
    }
}
