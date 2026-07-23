//! Shared conformance assertions for Output Adapter implementations.

use std::collections::BTreeSet;

use iotkit_output_adapter_api::{Observation, OutputAdapter};
use serde_json::value::RawValue;

pub struct ConformanceCase<'a> {
    pub config: &'a RawValue,
    pub observation: &'a Observation,
    pub expected_topic: &'a str,
    pub expected_qos: u8,
    pub expected_retain: bool,
    pub expected_payload: &'a str,
}

pub fn assert_adapter_conformance(
    adapter: &dyn OutputAdapter,
    cases: &[ConformanceCase<'_>],
) -> Result<(), String> {
    let descriptor = adapter.descriptor();
    if descriptor.id.is_empty()
        || descriptor.config_schema_version == 0
        || !descriptor
            .id
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_lowercase)
        || !descriptor.id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
    {
        return Err("invalid descriptor identity or schema version".into());
    }
    if descriptor.modes.is_empty() {
        return Err("descriptor must expose at least one mode".into());
    }
    let mut modes = BTreeSet::new();
    for mode in descriptor.modes {
        if mode.key.is_empty()
            || mode.display_name.is_empty()
            || mode.accepts.is_empty()
            || !modes.insert(mode.key)
        {
            return Err("descriptor contains an invalid or duplicate mode".into());
        }
    }

    for case in cases {
        adapter
            .validate_config(case.config, case.observation.kind())
            .map_err(|error| format!("configuration rejected: {error}"))?;
        let first = adapter
            .transform(case.config, case.observation)
            .map_err(|error| format!("transform failed: {error}"))?;
        let second = adapter
            .transform(case.config, case.observation)
            .map_err(|error| format!("second transform failed: {error}"))?;

        if first.topic() != case.expected_topic
            || first.qos() != case.expected_qos
            || first.retain() != case.expected_retain
            || first.payload().get() != case.expected_payload
        {
            return Err(format!(
                "publication mismatch: topic={} qos={} retain={} payload={}",
                first.topic(),
                first.qos(),
                first.retain(),
                first.payload().get()
            ));
        }
        if first.topic() != second.topic()
            || first.qos() != second.qos()
            || first.retain() != second.retain()
            || first.payload().get() != second.payload().get()
        {
            return Err("adapter output is not deterministic".into());
        }
    }
    Ok(())
}
