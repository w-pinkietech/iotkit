use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const EGRESS_SCHEMA_VERSION: u32 = 1;
pub const MAX_BATCH_RECORDS: usize = 256;
pub const MAX_BATCH_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordBatch {
    pub schema_version: u32,
    pub gateway_identity: String,
    pub ledger_epoch: String,
    pub publication_id: String,
    pub cursor_start: i64,
    pub cursor_end: i64,
    pub records: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedThrough {
    pub schema_version: u32,
    pub gateway_identity: String,
    pub ledger_epoch: String,
    pub publication_id: String,
    pub accepted_through: i64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid egress v1 message: {0}")]
pub struct WireError(String);

impl RecordBatch {
    pub fn validate(&self) -> Result<(), WireError> {
        if self.schema_version != EGRESS_SCHEMA_VERSION {
            return invalid("unsupported schema_version");
        }
        validate_topic_segment("gateway_identity", &self.gateway_identity)?;
        validate_identity_component("ledger_epoch", &self.ledger_epoch)?;
        if self.cursor_start < 1 || self.cursor_end < self.cursor_start {
            return invalid("cursor range must be positive and non-empty");
        }
        let expected_count = self
            .cursor_end
            .checked_sub(self.cursor_start)
            .and_then(|span| span.checked_add(1))
            .and_then(|count| usize::try_from(count).ok())
            .ok_or_else(|| WireError("cursor range overflow".into()))?;
        if expected_count != self.records.len() {
            return invalid("cursor range does not match record count");
        }
        if self.records.len() > MAX_BATCH_RECORDS {
            return invalid("batch exceeds record limit");
        }
        let expected_publication_id = publication_id(
            &self.gateway_identity,
            &self.ledger_epoch,
            self.cursor_start,
            self.cursor_end,
        );
        if self.publication_id != expected_publication_id {
            return invalid("publication_id does not match batch identity");
        }

        for (offset, record) in self.records.iter().enumerate() {
            let object = record
                .as_object()
                .ok_or_else(|| WireError("record must be an object".into()))?;
            let expected_seq = self.cursor_start + offset as i64;
            if object.get("schema_version").and_then(Value::as_u64)
                != Some(EGRESS_SCHEMA_VERSION as u64)
            {
                return invalid("record schema_version mismatch");
            }
            if object.get("epoch").and_then(Value::as_str) != Some(self.ledger_epoch.as_str()) {
                return invalid("record epoch mismatch");
            }
            if object.get("pub_seq").and_then(Value::as_i64) != Some(expected_seq) {
                return invalid("record pub_seq is not contiguous");
            }
        }

        let encoded = serde_json::to_vec(self)
            .map_err(|error| WireError(format!("batch encoding failed: {error}")))?;
        if encoded.len() > MAX_BATCH_BYTES {
            return invalid("batch exceeds encoded byte limit");
        }
        Ok(())
    }
}

impl AcceptedThrough {
    pub fn validate_for(&self, batch: &RecordBatch, prior_cursor: i64) -> Result<(), WireError> {
        batch.validate()?;
        if self.schema_version != EGRESS_SCHEMA_VERSION {
            return invalid("ack schema_version mismatch");
        }
        if self.gateway_identity != batch.gateway_identity {
            return invalid("ack gateway_identity mismatch");
        }
        if self.ledger_epoch != batch.ledger_epoch {
            return invalid("ack ledger_epoch mismatch");
        }
        if self.publication_id != batch.publication_id {
            return invalid("ack publication_id mismatch");
        }
        if self.accepted_through != batch.cursor_end {
            return invalid("ack must accept the complete initial-window batch");
        }
        if self.accepted_through <= prior_cursor {
            return invalid("ack does not advance the cursor");
        }
        Ok(())
    }
}

pub fn publication_id(
    gateway_identity: &str,
    ledger_epoch: &str,
    cursor_start: i64,
    cursor_end: i64,
) -> String {
    format!("{gateway_identity}:{ledger_epoch}:{cursor_start}:{cursor_end}")
}

fn validate_topic_segment(name: &str, value: &str) -> Result<(), WireError> {
    validate_identity_component(name, value)?;
    if value.contains(['/', '+', '#']) {
        return invalid(&format!("{name} is not a safe MQTT topic segment"));
    }
    Ok(())
}

fn validate_identity_component(name: &str, value: &str) -> Result<(), WireError> {
    if value.is_empty() || value.contains(':') {
        return invalid(&format!("{name} is empty or contains ':'"));
    }
    Ok(())
}

fn invalid<T>(message: &str) -> Result<T, WireError> {
    Err(WireError(message.into()))
}
