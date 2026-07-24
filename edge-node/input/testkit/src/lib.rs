//! Reusable conformance assertions and a test-only vendor-neutral reference adapter.

use iotkit_ingest_contract::{ReadingItem, TimeSource};
use iotkit_input_adapter_host_api::{
    AdapterCompletion, AdapterInstanceId, AdapterStartContext, AdapterTypeId, ConfiguredSource,
    InputAdapterTypeDescriptor, PhysicalTransportKind, RunningInputAdapter, UnexpectedExitReason,
    runtime_channels,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceAdapterConfig {
    pub diagnostic_capacity: usize,
}

impl ReferenceAdapter {
    pub fn new() -> Self {
        Self {
            instance_id: AdapterInstanceId::new("reference_one").unwrap(),
            source: ConfiguredSource::new("input:reference:one").unwrap(),
        }
    }

    pub fn descriptor() -> InputAdapterTypeDescriptor {
        InputAdapterTypeDescriptor {
            adapter_type_id: AdapterTypeId::new("reference-adapter").unwrap(),
            adapter_api_major: 1,
            config_schema_version: 1,
            implementation_version: env!("CARGO_PKG_VERSION"),
            display_name: "Reference Adapter",
            physical_transport_kind: PhysicalTransportKind::Other,
        }
    }

    pub fn parse_and_validate(
        config: ReferenceAdapterConfig,
    ) -> Result<ReferenceAdapterConfig, &'static str> {
        if config.diagnostic_capacity == 0 {
            return Err("diagnostic_capacity must be at least 1");
        }
        Ok(config)
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

    pub fn start(
        &self,
        context: AdapterStartContext,
        config: ReferenceAdapterConfig,
    ) -> Result<RunningInputAdapter, &'static str> {
        if context.instance_id != self.instance_id || context.configured_source != self.source {
            return Err("start context does not match the configured reference adapter");
        }
        let items = self.observations();
        let (mut runtime, running) =
            runtime_channels(context.instance_id.clone(), config.diagnostic_capacity);
        tokio::spawn(async move {
            runtime.activity.physical_decode();
            if context.ingest.try_submit(items).is_err() {
                runtime
                    .completion
                    .complete(AdapterCompletion::UnexpectedExit(
                        UnexpectedExitReason::ClientClosed,
                    ));
                return;
            }
            runtime.activity.queue_admission();
            let completion = if runtime.stop.changed().await {
                AdapterCompletion::RequestedStop
            } else {
                AdapterCompletion::UnexpectedExit(UnexpectedExitReason::InternalInvariant)
            };
            runtime.completion.complete(completion);
        });
        Ok(running)
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
