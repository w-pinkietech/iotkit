//! Edge Node と IoTKit Edge 間のMQTT custody wire contract。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_BATCH_RECORDS: usize = 256;
pub const MAX_BATCH_BYTES: usize = 1024 * 1024;
pub const MAX_DESCRIPTOR_BYTES: usize = 1024 * 1024;
/// Status heartbeat is intentionally small: it carries only bounded operational
/// evidence and never carries a local error, endpoint, payload, or credential.
pub const MAX_STATUS_BYTES: usize = 16 * 1024;
pub const MAX_STATUS_ADAPTERS: usize = 64;
pub const MAX_STATUS_ID_BYTES: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("decode contract JSON: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("invalid contract message: {0}")]
    Invalid(String),
}

fn invalid(message: impl Into<String>) -> ContractError {
    ContractError::Invalid(message.into())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordBatch {
    pub schema_version: u32,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub publication_id: String,
    pub cursor_start: i64,
    pub cursor_end: i64,
    pub records: Vec<Box<RawValue>>,
}

impl RecordBatch {
    pub fn decode(payload: &[u8]) -> Result<Self, ContractError> {
        if payload.len() > MAX_BATCH_BYTES {
            return Err(invalid("batch exceeds encoded byte limit"));
        }
        let batch: Self = serde_json::from_slice(payload)?;
        batch.validate()?;
        Ok(batch)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(invalid("unsupported schema_version"));
        }
        validate_topic_segment("edge_node_id", &self.edge_node_id)?;
        validate_identity("ledger_epoch", &self.ledger_epoch)?;
        if self.cursor_start < 1 || self.cursor_end < self.cursor_start {
            return Err(invalid("cursor range must be positive and non-empty"));
        }
        if self.cursor_end - self.cursor_start + 1 != self.records.len() as i64 {
            return Err(invalid("cursor range does not match record count"));
        }
        if self.records.len() > MAX_BATCH_RECORDS {
            return Err(invalid("batch exceeds record limit"));
        }
        if self.publication_id
            != publication_id(
                &self.edge_node_id,
                &self.ledger_epoch,
                self.cursor_start,
                self.cursor_end,
            )
        {
            return Err(invalid("publication_id does not match batch identity"));
        }
        for (index, record) in self.records.iter().enumerate() {
            validate_record(record, &self.ledger_epoch, self.cursor_start + index as i64)?;
        }
        if serde_json::to_vec(self)?.len() > MAX_BATCH_BYTES {
            return Err(invalid("batch exceeds encoded byte limit"));
        }
        Ok(())
    }

    pub fn validate_topic_edge_node(&self, edge_node_id: &str) -> Result<(), ContractError> {
        if self.edge_node_id != edge_node_id {
            return Err(invalid("topic/body edge_node_id mismatch"));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct RecordHeader {
    family: String,
    schema_version: u32,
    epoch: String,
    pub_seq: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasurementRecord {
    family: String,
    schema_version: u32,
    epoch: String,
    pub_seq: i64,
    series_key: String,
    values: Vec<f64>,
    event_time: i64,
    event_time_source: String,
    time_source: String,
    time_quality: String,
    received_at: i64,
    device_time: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnnotationRecord {
    family: String,
    schema_version: u32,
    epoch: String,
    pub_seq: i64,
    subtype: String,
    prior_epoch: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommissioningSmokeRecord {
    family: String,
    schema_version: u32,
    epoch: String,
    pub_seq: i64,
    test_id: String,
}

fn validate_record(raw: &RawValue, ledger_epoch: &str, pub_seq: i64) -> Result<(), ContractError> {
    let header: RecordHeader = serde_json::from_str(raw.get())?;
    if header.schema_version != SCHEMA_VERSION
        || header.epoch != ledger_epoch
        || header.pub_seq != pub_seq
    {
        return Err(invalid("record identity does not match batch"));
    }
    match header.family.as_str() {
        "measurement" => {
            let record: MeasurementRecord = serde_json::from_str(raw.get())?;
            if record.family != "measurement"
                || record.schema_version != SCHEMA_VERSION
                || record.epoch != ledger_epoch
                || record.pub_seq != pub_seq
                || record.series_key.is_empty()
                || record.values.is_empty()
                || record.values.iter().any(|value| !value.is_finite())
            {
                return Err(invalid("measurement fields are invalid"));
            }
            if !matches!(
                record.time_source.as_str(),
                "device_ntp" | "device_rtc" | "edge_node" | "edge_node_adjusted"
            ) || !matches!(
                record.time_quality.as_str(),
                "synced" | "holdover" | "unsynced"
            ) {
                return Err(invalid("measurement time metadata is invalid"));
            }
            let device_time = match &record.device_time {
                serde_json::Value::Null => None,
                serde_json::Value::Number(number) => number.as_i64(),
                _ => return Err(invalid("device_time must be an integer or null")),
            };
            let valid_time = match record.event_time_source.as_str() {
                "received_at" => record.event_time == record.received_at,
                "device" => {
                    matches!(record.time_source.as_str(), "device_ntp" | "device_rtc")
                        && device_time == Some(record.event_time)
                }
                "edge_node_adjusted" => {
                    record.time_source == "edge_node_adjusted"
                        && device_time == Some(record.event_time)
                }
                _ => false,
            };
            if !valid_time {
                return Err(invalid("measurement event time is inconsistent"));
            }
        }
        "annotation" => {
            let record: AnnotationRecord = serde_json::from_str(raw.get())?;
            if record.family != "annotation"
                || record.schema_version != SCHEMA_VERSION
                || record.epoch != ledger_epoch
                || record.pub_seq != pub_seq
                || record.subtype != "epoch_start"
                || record.prior_epoch.is_empty()
            {
                return Err(invalid("annotation fields are invalid"));
            }
        }
        "commissioning_smoke" => {
            let record: CommissioningSmokeRecord = serde_json::from_str(raw.get())?;
            let suffix = record.test_id.strip_prefix("smoke-").unwrap_or_default();
            if record.family != "commissioning_smoke"
                || record.schema_version != SCHEMA_VERSION
                || record.epoch != ledger_epoch
                || record.pub_seq != pub_seq
                || suffix.len() != 32
                || !suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err(invalid("commissioning smoke fields are invalid"));
            }
        }
        _ => return Err(invalid("record family is unsupported")),
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedThrough {
    pub schema_version: u32,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub publication_id: String,
    pub accepted_through: i64,
}

impl AcceptedThrough {
    pub fn decode(payload: &[u8]) -> Result<Self, ContractError> {
        let ack: Self = serde_json::from_slice(payload)?;
        ack.validate()?;
        Ok(ack)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != SCHEMA_VERSION || self.accepted_through < 1 {
            return Err(invalid("invalid accepted-through fields"));
        }
        validate_topic_segment("edge_node_id", &self.edge_node_id)?;
        validate_identity("ledger_epoch", &self.ledger_epoch)?;
        if self.publication_id.is_empty() {
            return Err(invalid("publication_id is missing"));
        }
        Ok(())
    }

    pub fn validate_for(
        &self,
        batch: &RecordBatch,
        prior_cursor: i64,
    ) -> Result<(), ContractError> {
        self.validate()?;
        batch.validate()?;
        if self.edge_node_id != batch.edge_node_id
            || self.ledger_epoch != batch.ledger_epoch
            || self.publication_id != batch.publication_id
            || self.accepted_through != batch.cursor_end
            || self.accepted_through <= prior_cursor
        {
            return Err(invalid("accepted-through does not match batch"));
        }
        Ok(())
    }
}

/// Latest-only operational evidence from an Edge Node.  It is deliberately
/// independent from the record custody cursor: MQTT receipt of this message
/// never accepts, advances, or purges a reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusHeartbeat {
    pub schema_version: u32,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub boot_id: String,
    pub status_seq: u64,
    pub collector_state: CollectorState,
    pub adapters: Vec<StatusAdapter>,
    pub accepted_through: i64,
    pub pending_publications: i64,
    pub storage_pressure: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectorState {
    Running,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusAdapter {
    pub adapter_id: String,
    pub state: AdapterState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterState {
    Running,
    Restarting,
    Exhausted,
    Stopped,
}

impl StatusHeartbeat {
    pub fn decode(payload: &[u8]) -> Result<Self, ContractError> {
        if payload.len() > MAX_STATUS_BYTES {
            return Err(invalid("status heartbeat exceeds encoded byte limit"));
        }
        let heartbeat: Self = serde_json::from_slice(payload)?;
        heartbeat.validate()?;
        Ok(heartbeat)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != SCHEMA_VERSION
            || self.status_seq == 0
            || self.status_seq > i64::MAX as u64
            || self.accepted_through < 0
            || self.pending_publications < 0
            || self.adapters.len() > MAX_STATUS_ADAPTERS
        {
            return Err(invalid("invalid status heartbeat boundary"));
        }
        validate_topic_segment("edge_node_id", &self.edge_node_id)?;
        validate_identity("ledger_epoch", &self.ledger_epoch)?;
        validate_status_id("boot_id", &self.boot_id)?;
        let mut adapter_ids = HashSet::new();
        for adapter in &self.adapters {
            validate_status_id("adapter_id", &adapter.adapter_id)?;
            if !adapter_ids.insert(adapter.adapter_id.as_str()) {
                return Err(invalid("status heartbeat has duplicate adapter_id"));
            }
        }
        if serde_json::to_vec(self)?.len() > MAX_STATUS_BYTES {
            return Err(invalid("status heartbeat exceeds encoded byte limit"));
        }
        Ok(())
    }

    pub fn validate_topic_edge_node(&self, edge_node_id: &str) -> Result<(), ContractError> {
        if self.edge_node_id != edge_node_id {
            return Err(invalid("status heartbeat topic/body edge_node_id mismatch"));
        }
        Ok(())
    }
}

pub fn publication_id(edge_node_id: &str, epoch: &str, start: i64, end: i64) -> String {
    format!("{edge_node_id}:{epoch}:{start}:{end}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationRequest {
    pub schema_version: u32,
    pub activation_id: String,
    pub edge_id: String,
    pub edge_node_id: String,
    pub expected_ledger_epoch: String,
    pub grant_revision: u64,
    pub issued_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationResult {
    pub schema_version: u32,
    pub activation_id: String,
    pub edge_id: String,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub status: String,
    pub discard_through_reading_seq: i64,
    pub first_publication_seq: i64,
    pub applied_at: i64,
}

impl ActivationRequest {
    pub fn decode(payload: &[u8]) -> Result<Self, ContractError> {
        let request: Self = serde_json::from_slice(payload)?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        validate_activation_common(
            self.schema_version,
            &self.activation_id,
            &self.edge_id,
            &self.edge_node_id,
        )?;
        validate_identity("expected_ledger_epoch", &self.expected_ledger_epoch)?;
        if self.grant_revision != 1 || self.issued_at < 0 {
            return Err(invalid("invalid activation request boundary"));
        }
        Ok(())
    }
}

impl ActivationResult {
    pub fn decode(payload: &[u8]) -> Result<Self, ContractError> {
        let result: Self = serde_json::from_slice(payload)?;
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        validate_activation_common(
            self.schema_version,
            &self.activation_id,
            &self.edge_id,
            &self.edge_node_id,
        )?;
        validate_identity("ledger_epoch", &self.ledger_epoch)?;
        if self.status != "applied"
            || self.discard_through_reading_seq < 0
            || self.first_publication_seq != 1
            || self.applied_at < 0
        {
            return Err(invalid("invalid activation result boundary"));
        }
        Ok(())
    }

    pub fn validate_topic_edge_node(&self, edge_node_id: &str) -> Result<(), ContractError> {
        if self.edge_node_id != edge_node_id {
            return Err(invalid(
                "activation result topic/body edge_node_id mismatch",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryActivationRequest {
    pub schema_version: u32,
    pub recovery_id: String,
    pub edge_id: String,
    pub edge_node_id: String,
    pub candidate_instance_id: String,
    pub backup_id: String,
    pub old_ledger_epoch: String,
    pub new_ledger_epoch: String,
    pub broker_credential_generation: i64,
    pub device_auth_generation: i64,
    pub snapshot_accepted_through: i64,
    pub snapshot_allocation_high_water: i64,
    pub snapshot_epoch_start_publication_seq: Option<i64>,
    pub edge_accepted_through: i64,
    pub grant_revision: u64,
    pub issued_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryActivationResult {
    pub schema_version: u32,
    pub recovery_id: String,
    pub edge_id: String,
    pub edge_node_id: String,
    pub candidate_instance_id: String,
    pub backup_id: String,
    pub old_ledger_epoch: String,
    pub new_ledger_epoch: String,
    pub broker_credential_generation: i64,
    pub device_auth_generation: i64,
    pub status: String,
    pub edge_accepted_through: i64,
    pub replayed_records: i64,
    pub first_new_publication_seq: i64,
    pub last_new_publication_seq: i64,
    pub applied_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryCompletion {
    pub schema_version: u32,
    pub recovery_id: String,
    pub edge_id: String,
    pub edge_node_id: String,
    pub candidate_instance_id: String,
    pub new_ledger_epoch: String,
    pub status: String,
    pub accepted_through: i64,
    pub committed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryCompletionAck {
    pub schema_version: u32,
    pub recovery_id: String,
    pub edge_id: String,
    pub edge_node_id: String,
    pub candidate_instance_id: String,
    pub new_ledger_epoch: String,
    pub status: String,
    pub acknowledged_at: i64,
}

impl RecoveryActivationRequest {
    pub fn decode(payload: &[u8]) -> Result<Self, ContractError> {
        let request: Self = serde_json::from_slice(payload)?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        validate_recovery_common(
            self.schema_version,
            &self.recovery_id,
            &self.edge_id,
            &self.edge_node_id,
            &self.candidate_instance_id,
        )?;
        validate_prefixed_hex("backup_id", &self.backup_id, "backup-")?;
        validate_identity("old_ledger_epoch", &self.old_ledger_epoch)?;
        validate_identity("new_ledger_epoch", &self.new_ledger_epoch)?;
        if self.old_ledger_epoch == self.new_ledger_epoch
            || self.broker_credential_generation < 1
            || self.device_auth_generation < 0
            || self.snapshot_accepted_through < 0
            || self.snapshot_allocation_high_water < self.snapshot_accepted_through
            || self
                .snapshot_epoch_start_publication_seq
                .is_some_and(|sequence| {
                    sequence < 1 || sequence > self.snapshot_allocation_high_water
                })
            || self.edge_accepted_through < self.snapshot_accepted_through
            || self.grant_revision != 1
            || self.issued_at < 0
        {
            return Err(invalid("invalid recovery activation request boundary"));
        }
        Ok(())
    }
}

impl RecoveryActivationResult {
    pub fn decode(payload: &[u8]) -> Result<Self, ContractError> {
        let result: Self = serde_json::from_slice(payload)?;
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        validate_recovery_common(
            self.schema_version,
            &self.recovery_id,
            &self.edge_id,
            &self.edge_node_id,
            &self.candidate_instance_id,
        )?;
        validate_prefixed_hex("backup_id", &self.backup_id, "backup-")?;
        validate_identity("old_ledger_epoch", &self.old_ledger_epoch)?;
        validate_identity("new_ledger_epoch", &self.new_ledger_epoch)?;
        if self.old_ledger_epoch == self.new_ledger_epoch
            || self.broker_credential_generation < 1
            || self.device_auth_generation < 0
            || self.status != "applied"
            || self.edge_accepted_through < 0
            || self.replayed_records < 0
            || self.first_new_publication_seq != 1
            || self.replayed_records.checked_add(1) != Some(self.last_new_publication_seq)
            || self.applied_at < 0
        {
            return Err(invalid("invalid recovery activation result boundary"));
        }
        Ok(())
    }

    pub fn validate_topic_edge_node(&self, edge_node_id: &str) -> Result<(), ContractError> {
        if self.edge_node_id != edge_node_id {
            return Err(invalid(
                "recovery activation result topic/body edge_node_id mismatch",
            ));
        }
        Ok(())
    }

    pub fn validate_for(&self, request: &RecoveryActivationRequest) -> Result<(), ContractError> {
        self.validate()?;
        if self.recovery_id != request.recovery_id
            || self.edge_id != request.edge_id
            || self.edge_node_id != request.edge_node_id
            || self.candidate_instance_id != request.candidate_instance_id
            || self.backup_id != request.backup_id
            || self.old_ledger_epoch != request.old_ledger_epoch
            || self.new_ledger_epoch != request.new_ledger_epoch
            || self.broker_credential_generation != request.broker_credential_generation
            || self.device_auth_generation != request.device_auth_generation
            || self.edge_accepted_through != request.edge_accepted_through
        {
            return Err(invalid("recovery activation result does not match request"));
        }
        Ok(())
    }
}

impl RecoveryCompletion {
    pub fn decode(payload: &[u8]) -> Result<Self, ContractError> {
        let completion: Self = serde_json::from_slice(payload)?;
        completion.validate()?;
        Ok(completion)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        validate_recovery_common(
            self.schema_version,
            &self.recovery_id,
            &self.edge_id,
            &self.edge_node_id,
            &self.candidate_instance_id,
        )?;
        validate_identity("new_ledger_epoch", &self.new_ledger_epoch)?;
        if self.status != "committed" || self.accepted_through != 0 || self.committed_at < 0 {
            return Err(invalid("invalid recovery completion boundary"));
        }
        Ok(())
    }

    pub fn validate_for(&self, request: &RecoveryActivationRequest) -> Result<(), ContractError> {
        self.validate()?;
        if self.recovery_id != request.recovery_id
            || self.edge_id != request.edge_id
            || self.edge_node_id != request.edge_node_id
            || self.candidate_instance_id != request.candidate_instance_id
            || self.new_ledger_epoch != request.new_ledger_epoch
        {
            return Err(invalid("recovery completion does not match request"));
        }
        Ok(())
    }
}

impl RecoveryCompletionAck {
    pub fn decode(payload: &[u8]) -> Result<Self, ContractError> {
        let acknowledgement: Self = serde_json::from_slice(payload)?;
        acknowledgement.validate()?;
        Ok(acknowledgement)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        validate_recovery_common(
            self.schema_version,
            &self.recovery_id,
            &self.edge_id,
            &self.edge_node_id,
            &self.candidate_instance_id,
        )?;
        validate_identity("new_ledger_epoch", &self.new_ledger_epoch)?;
        if self.status != "completion_stored" || self.acknowledged_at < 0 {
            return Err(invalid("invalid recovery completion acknowledgement"));
        }
        Ok(())
    }

    pub fn validate_topic_edge_node(&self, edge_node_id: &str) -> Result<(), ContractError> {
        if self.edge_node_id != edge_node_id {
            return Err(invalid(
                "recovery completion acknowledgement topic/body edge_node_id mismatch",
            ));
        }
        Ok(())
    }
}

fn validate_recovery_common(
    schema_version: u32,
    recovery_id: &str,
    edge_id: &str,
    edge_node_id: &str,
    candidate_instance_id: &str,
) -> Result<(), ContractError> {
    if schema_version != SCHEMA_VERSION {
        return Err(invalid("recovery schema_version must be 1"));
    }
    validate_prefixed_hex("recovery_id", recovery_id, "recovery-")?;
    validate_prefixed_hex("edge_id", edge_id, "edge-")?;
    validate_topic_segment("edge_node_id", edge_node_id)?;
    validate_prefixed_hex("candidate_instance_id", candidate_instance_id, "candidate-")
}

fn validate_activation_common(
    schema_version: u32,
    activation_id: &str,
    edge_id: &str,
    edge_node_id: &str,
) -> Result<(), ContractError> {
    if schema_version != SCHEMA_VERSION {
        return Err(invalid("activation schema_version must be 1"));
    }
    validate_prefixed_hex("activation_id", activation_id, "act-")?;
    validate_prefixed_hex("edge_id", edge_id, "edge-")?;
    validate_topic_segment("edge_node_id", edge_node_id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DescriptorDevice {
    pub system_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub fn decode(payload: &[u8]) -> Result<Self, ContractError> {
        if payload.len() > MAX_DESCRIPTOR_BYTES {
            return Err(invalid("descriptor exceeds encoded byte limit"));
        }
        let descriptor: Self = serde_json::from_slice(payload)?;
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != 2
            || !self.complete
            || self.descriptor_revision == 0
            || self.descriptor_revision > i64::MAX as u64
        {
            return Err(invalid(
                "only complete descriptor schema version 2 is supported",
            ));
        }
        validate_topic_segment("edge_node_id", &self.edge_node_id)?;
        validate_identity("ledger_epoch", &self.ledger_epoch)?;
        let mut devices = HashSet::new();
        for device in &self.devices {
            if !valid_uuid(&device.system_id)
                || !devices.insert(device.system_id.as_str())
                || !matches!(device.state.as_str(), "quarantined" | "active" | "retired")
                || device
                    .identifier
                    .as_deref()
                    .is_some_and(|value| !valid_display_text(value, 64, false))
                || device
                    .model_id
                    .as_deref()
                    .is_some_and(|value| !valid_model_id(value))
            {
                return Err(invalid("invalid or duplicate descriptor device"));
            }
        }
        let mut signals = HashSet::new();
        for signal in &self.signals {
            if !devices.contains(signal.system_id.as_str())
                || !signals.insert(signal.series_key.as_str())
                || !matches!(
                    signal.value_type.as_str(),
                    "float" | "int" | "bool" | "record"
                )
                || !valid_measurement_key(&signal.measurement_key)
                || signal.channel_index.is_some_and(|channel| channel < 0)
                || signal.variant.is_empty()
                || signal.variant.contains(':')
                || signal.variant.chars().any(char::is_control)
                || signal
                    .unit
                    .as_deref()
                    .is_some_and(|value| !valid_display_text(value, 128, true))
            {
                return Err(invalid("invalid or duplicate descriptor signal"));
            }
            let expected_channel = signal
                .channel_index
                .map_or_else(|| "na".into(), |channel| channel.to_string());
            let expected = format!(
                "{}:{}:{}:{}",
                signal.system_id, signal.measurement_key, expected_channel, signal.variant
            );
            if signal.series_key != expected {
                return Err(invalid("series_key does not match signal identity"));
            }
        }
        Ok(())
    }

    pub fn content_sha256(&self) -> Result<[u8; 32], ContractError> {
        self.validate()?;
        Ok(Sha256::digest(serde_json::to_vec(self)?).into())
    }

    pub fn validate_topic_edge_node(&self, edge_node_id: &str) -> Result<(), ContractError> {
        if self.edge_node_id != edge_node_id {
            return Err(invalid("descriptor topic/body edge_node_id mismatch"));
        }
        Ok(())
    }
}

fn validate_prefixed_hex(field: &str, value: &str, prefix: &str) -> Result<(), ContractError> {
    let suffix = value.strip_prefix(prefix).unwrap_or_default();
    if suffix.len() != 32
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid(format!("{field} is not prefixed lowercase hex")));
    }
    Ok(())
}

fn validate_topic_segment(field: &str, value: &str) -> Result<(), ContractError> {
    validate_identity(field, value)?;
    if value.contains(['/', '+', '#']) {
        return Err(invalid(format!("{field} is not a safe MQTT topic segment")));
    }
    Ok(())
}

fn validate_identity(field: &str, value: &str) -> Result<(), ContractError> {
    if value.is_empty()
        || value.len() > 255
        || value.contains(':')
        || value.chars().any(char::is_control)
    {
        return Err(invalid(format!("{field} is not a valid identity")));
    }
    Ok(())
}

fn validate_status_id(field: &str, value: &str) -> Result<(), ContractError> {
    if value.is_empty()
        || value.len() > MAX_STATUS_ID_BYTES
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_uppercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.')
        })
    {
        return Err(invalid(format!(
            "{field} is not a bounded opaque status identity"
        )));
    }
    Ok(())
}

fn valid_uuid(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    value.char_indices().all(|(index, character)| {
        if matches!(index, 8 | 13 | 18 | 23) {
            character == '-'
        } else {
            character.is_ascii_hexdigit() && !character.is_ascii_uppercase()
        }
    })
}

fn valid_measurement_key(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    value.split('.').all(|segment| {
        let mut bytes = segment.bytes();
        bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    })
}

fn valid_model_id(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    let mut bytes = value.bytes();
    if !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase()) {
        return false;
    }
    let mut after_separator = false;
    for byte in bytes {
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            after_separator = false;
        } else if matches!(byte, b'-' | b'_' | b'.') && !after_separator {
            after_separator = true;
        } else {
            return false;
        }
    }
    !after_separator
}

fn valid_display_text(value: &str, max_bytes: usize, allow_empty: bool) -> bool {
    value.len() <= max_bytes
        && (allow_empty || !value.is_empty())
        && !value.chars().any(char::is_control)
}
