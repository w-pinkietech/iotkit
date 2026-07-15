use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::PublishError;

pub const DESCRIPTOR_SCHEMA_VERSION: u32 = 1;
pub const MAX_DESCRIPTOR_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DescriptorSnapshot {
    pub schema_version: u32,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub descriptor_revision: u64,
    pub complete: bool,
    pub devices: Vec<DescriptorDevice>,
    pub signals: Vec<DescriptorSignal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DescriptorDevice {
    pub system_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DescriptorSignal {
    pub series_key: String,
    pub system_id: String,
    pub measurement_key: String,
    pub channel_index: Option<i32>,
    pub variant: String,
    pub unit: Option<String>,
    pub value_type: String,
}

impl DescriptorSnapshot {
    pub fn decode_bounded(payload: &[u8]) -> Result<Self, PublishError> {
        if payload.len() > MAX_DESCRIPTOR_BYTES {
            return Err(PublishError::Invalid(format!(
                "descriptor snapshot exceeds {MAX_DESCRIPTOR_BYTES} encoded bytes"
            )));
        }
        let snapshot: Self = serde_json::from_slice(payload).map_err(|error| {
            PublishError::Invalid(format!("descriptor decoding failed: {error}"))
        })?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn encode_bounded(&self) -> Result<Vec<u8>, PublishError> {
        self.validate()?;
        let payload = serde_json::to_vec(self).map_err(|error| {
            PublishError::Invalid(format!("descriptor encoding failed: {error}"))
        })?;
        if payload.len() > MAX_DESCRIPTOR_BYTES {
            return Err(PublishError::Invalid(format!(
                "descriptor snapshot exceeds {MAX_DESCRIPTOR_BYTES} encoded bytes"
            )));
        }
        Ok(payload)
    }

    fn validate(&self) -> Result<(), PublishError> {
        if self.schema_version != DESCRIPTOR_SCHEMA_VERSION || !self.complete {
            return invalid("only complete descriptor schema version 1 is supported");
        }
        crate::wire::validate_topic_segment("edge_node_id", &self.edge_node_id)
            .map_err(|error| PublishError::Invalid(error.to_string()))?;
        if self.ledger_epoch.is_empty() || self.ledger_epoch.contains(':') {
            return invalid("ledger_epoch is empty or contains ':'");
        }
        if self.descriptor_revision < 1 {
            return invalid("descriptor_revision must be positive");
        }

        let mut device_ids = HashSet::with_capacity(self.devices.len());
        for device in &self.devices {
            validate_system_id(&device.system_id)?;
            if !device_ids.insert(device.system_id.as_str()) {
                return invalid("duplicate descriptor device system_id");
            }
            if !matches!(device.state.as_str(), "quarantined" | "active" | "retired") {
                return invalid("unsupported descriptor device state");
            }
            if let Some(identifier) = &device.identifier
                && (identifier.is_empty()
                    || identifier.len() > 64
                    || identifier.chars().any(char::is_control))
            {
                return invalid("invalid descriptor device identifier");
            }
        }

        let mut series_keys = HashSet::with_capacity(self.signals.len());
        for signal in &self.signals {
            validate_system_id(&signal.system_id)?;
            if !device_ids.contains(signal.system_id.as_str()) {
                return invalid("descriptor signal references an unknown device");
            }
            if signal.measurement_key.is_empty()
                || signal.measurement_key.contains(':')
                || signal.variant.is_empty()
                || signal.variant.contains(':')
                || signal.channel_index.is_some_and(|channel| channel < 0)
            {
                return invalid("invalid descriptor signal identity");
            }
            let channel = signal
                .channel_index
                .map(|value| value.to_string())
                .unwrap_or_else(|| "na".into());
            let expected = format!(
                "{}:{}:{}:{}",
                signal.system_id, signal.measurement_key, channel, signal.variant
            );
            if signal.series_key != expected {
                return invalid("descriptor series_key does not match signal identity");
            }
            if !series_keys.insert(signal.series_key.as_str()) {
                return invalid("duplicate descriptor signal series_key");
            }
            if !matches!(
                signal.value_type.as_str(),
                "float" | "int" | "bool" | "record"
            ) {
                return invalid("unsupported descriptor signal value_type");
            }
            if let Some(unit) = &signal.unit
                && (unit.len() > 128 || unit.chars().any(char::is_control))
            {
                return invalid("invalid descriptor signal unit");
            }
        }
        Ok(())
    }
}

fn validate_system_id(value: &str) -> Result<(), PublishError> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| PublishError::Invalid("descriptor system_id is not a UUID".into()))
}

fn invalid<T>(message: &str) -> Result<T, PublishError> {
    Err(PublishError::Invalid(message.into()))
}
