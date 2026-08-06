use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const EGRESS_SCHEMA_VERSION: u32 = 1;
pub const MAX_BATCH_RECORDS: usize = 256;
pub const MAX_BATCH_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordBatch {
    pub schema_version: u32,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub publication_id: String,
    pub cursor_start: i64,
    pub cursor_end: i64,
    pub records: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedThrough {
    pub schema_version: u32,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub publication_id: String,
    pub accepted_through: i64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
enum EgressRecord {
    Measurement(MeasurementRecord),
    Annotation(AnnotationRecord),
    CommissioningSmoke(CommissioningSmokeRecord),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasurementRecord {
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
    device_time: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnnotationRecord {
    schema_version: u32,
    epoch: String,
    pub_seq: i64,
    subtype: String,
    prior_epoch: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommissioningSmokeRecord {
    schema_version: u32,
    epoch: String,
    pub_seq: i64,
    test_id: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid egress v1 message: {0}")]
pub struct WireError(String);

impl RecordBatch {
    pub fn validate(&self) -> Result<(), WireError> {
        if self.schema_version != EGRESS_SCHEMA_VERSION {
            return invalid("unsupported schema_version");
        }
        validate_topic_segment("edge_node_id", &self.edge_node_id)?;
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
            &self.edge_node_id,
            &self.ledger_epoch,
            self.cursor_start,
            self.cursor_end,
        );
        if self.publication_id != expected_publication_id {
            return invalid("publication_id does not match batch identity");
        }

        for (offset, record) in self.records.iter().enumerate() {
            let expected_seq = self.cursor_start + offset as i64;
            let record: EgressRecord = serde_json::from_value(record.clone())
                .map_err(|error| WireError(format!("record schema is invalid: {error}")))?;
            record.validate(&self.ledger_epoch, expected_seq)?;
        }

        let encoded = serde_json::to_vec(self)
            .map_err(|error| WireError(format!("batch encoding failed: {error}")))?;
        if encoded.len() > MAX_BATCH_BYTES {
            return invalid("batch exceeds encoded byte limit");
        }
        Ok(())
    }
}

impl EgressRecord {
    fn validate(&self, ledger_epoch: &str, expected_seq: i64) -> Result<(), WireError> {
        let (schema_version, epoch, pub_seq) = match self {
            Self::Measurement(record) => {
                let device_time =
                    if record.device_time.is_null() {
                        None
                    } else {
                        Some(record.device_time.as_i64().ok_or_else(|| {
                            WireError("measurement device_time is invalid".into())
                        })?)
                    };
                if record.series_key.is_empty()
                    || record.values.is_empty()
                    || record.values.iter().any(|value| !value.is_finite())
                    || !matches!(
                        record.time_source.as_str(),
                        "device_ntp" | "device_rtc" | "edge_node" | "edge_node_adjusted"
                    )
                    || !matches!(
                        record.time_quality.as_str(),
                        "synced" | "holdover" | "unsynced"
                    )
                {
                    return invalid("measurement fields are invalid");
                }
                match record.event_time_source.as_str() {
                    "received_at" if record.event_time == record.received_at => {}
                    "device"
                        if matches!(record.time_source.as_str(), "device_ntp" | "device_rtc")
                            && device_time == Some(record.event_time) => {}
                    "edge_node_adjusted"
                        if record.time_source == "edge_node_adjusted"
                            && device_time == Some(record.event_time) => {}
                    _ => return invalid("measurement event_time is inconsistent"),
                }
                (record.schema_version, &record.epoch, record.pub_seq)
            }
            Self::Annotation(record) => {
                if record.subtype != "epoch_start" || record.prior_epoch.is_empty() {
                    return invalid("annotation must be epoch_start with prior_epoch");
                }
                (record.schema_version, &record.epoch, record.pub_seq)
            }
            Self::CommissioningSmoke(record) => {
                if !valid_commissioning_smoke_test_id(&record.test_id) {
                    return invalid("commissioning_smoke test_id is invalid");
                }
                (record.schema_version, &record.epoch, record.pub_seq)
            }
        };
        if schema_version != EGRESS_SCHEMA_VERSION {
            return invalid("record schema_version mismatch");
        }
        if epoch != ledger_epoch {
            return invalid("record epoch mismatch");
        }
        if pub_seq != expected_seq {
            return invalid("record pub_seq is not contiguous");
        }
        Ok(())
    }
}

fn valid_commissioning_smoke_test_id(test_id: &str) -> bool {
    let Some(random) = test_id.strip_prefix("smoke-") else {
        return false;
    };
    random.len() == 32
        && random
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

impl AcceptedThrough {
    pub fn validate_for(&self, batch: &RecordBatch, prior_cursor: i64) -> Result<(), WireError> {
        batch.validate()?;
        if self.schema_version != EGRESS_SCHEMA_VERSION {
            return invalid("ack schema_version mismatch");
        }
        if self.edge_node_id != batch.edge_node_id {
            return invalid("ack edge_node_id mismatch");
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

    pub fn validate_stale_for(
        &self,
        batch: &RecordBatch,
        prior_cursor: i64,
    ) -> Result<(), WireError> {
        batch.validate()?;
        if self.schema_version != EGRESS_SCHEMA_VERSION {
            return invalid("ack schema_version mismatch");
        }
        if self.edge_node_id != batch.edge_node_id {
            return invalid("ack edge_node_id mismatch");
        }
        if self.ledger_epoch != batch.ledger_epoch {
            return invalid("ack ledger_epoch mismatch");
        }
        if self.accepted_through > prior_cursor {
            return invalid("stale ack advances the cursor");
        }

        let (_, end) = self.publication_id_range()?;
        if end != self.accepted_through {
            return invalid("stale ack publication_id range is invalid");
        }
        Ok(())
    }

    pub fn validate_prior_prefix_for(
        &self,
        batch: &RecordBatch,
        prior_cursor: i64,
    ) -> Result<(), WireError> {
        batch.validate()?;
        if self.schema_version != EGRESS_SCHEMA_VERSION {
            return invalid("ack schema_version mismatch");
        }
        if self.edge_node_id != batch.edge_node_id {
            return invalid("ack edge_node_id mismatch");
        }
        if self.ledger_epoch != batch.ledger_epoch {
            return invalid("ack ledger_epoch mismatch");
        }
        let expected_start = prior_cursor
            .checked_add(1)
            .ok_or_else(|| WireError("prior cursor overflow".into()))?;
        if batch.cursor_start != expected_start {
            return invalid("current batch does not start after the prior cursor");
        }

        let (start, end) = self.publication_id_range()?;
        if start != batch.cursor_start || end != self.accepted_through {
            return invalid("prior prefix ack publication_id range is invalid");
        }
        if self.accepted_through <= prior_cursor || self.accepted_through >= batch.cursor_end {
            return invalid("ack is not a strict prior prefix");
        }
        Ok(())
    }

    fn publication_id_range(&self) -> Result<(i64, i64), WireError> {
        let parts: Vec<_> = self.publication_id.split(':').collect();
        let [edge_node_id, ledger_epoch, start, end] = parts.as_slice() else {
            return invalid("ack publication_id is malformed");
        };
        if *edge_node_id != self.edge_node_id || *ledger_epoch != self.ledger_epoch {
            return invalid("ack publication_id identity mismatch");
        }
        let start = start
            .parse::<i64>()
            .map_err(|_| WireError("ack publication_id range is invalid".into()))?;
        let end = end
            .parse::<i64>()
            .map_err(|_| WireError("ack publication_id range is invalid".into()))?;
        if start < 1 || end < start {
            return invalid("ack publication_id range is invalid");
        }
        if self.publication_id != publication_id(&self.edge_node_id, &self.ledger_epoch, start, end)
        {
            return invalid("ack publication_id is not deterministic");
        }
        Ok((start, end))
    }
}

pub fn publication_id(
    edge_node_id: &str,
    ledger_epoch: &str,
    cursor_start: i64,
    cursor_end: i64,
) -> String {
    format!("{edge_node_id}:{ledger_epoch}:{cursor_start}:{cursor_end}")
}

pub(crate) fn validate_topic_segment(name: &str, value: &str) -> Result<(), WireError> {
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
