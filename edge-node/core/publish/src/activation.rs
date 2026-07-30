use crate::PublishError;
use crate::store::TargetRow;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

pub const ACTIVATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationState {
    Standalone,
    DiscoveryOnly,
    Active,
}

impl ActivationState {
    fn from_db(value: &str) -> Result<Self, PublishError> {
        match value {
            "standalone" => Ok(Self::Standalone),
            "discovery_only" => Ok(Self::DiscoveryOnly),
            "active" => Ok(Self::Active),
            other => Err(PublishError::Invalid(format!(
                "unknown Edge Node activation state {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn decode(payload: &[u8]) -> Result<Self, PublishError> {
        let request: Self = serde_json::from_slice(payload)
            .map_err(|error| PublishError::Invalid(format!("activation request JSON: {error}")))?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), PublishError> {
        if self.schema_version != ACTIVATION_SCHEMA_VERSION {
            return invalid("activation request schema_version must be 1");
        }
        validate_prefixed_hex("activation_id", &self.activation_id, "act-")?;
        validate_prefixed_hex("edge_id", &self.edge_id, "edge-")?;
        validate_topic_segment("edge_node_id", &self.edge_node_id)?;
        validate_identity("expected_ledger_epoch", &self.expected_ledger_epoch)?;
        if self.grant_revision != 1 {
            return invalid("activation grant_revision must be 1");
        }
        if self.issued_at < 0 {
            return invalid("activation issued_at must be non-negative");
        }
        Ok(())
    }
}

impl ActivationResult {
    pub fn decode(payload: &[u8]) -> Result<Self, PublishError> {
        let result: Self = serde_json::from_slice(payload)
            .map_err(|error| PublishError::Invalid(format!("activation result JSON: {error}")))?;
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), PublishError> {
        if self.schema_version != ACTIVATION_SCHEMA_VERSION {
            return invalid("activation result schema_version must be 1");
        }
        validate_prefixed_hex("activation_id", &self.activation_id, "act-")?;
        validate_prefixed_hex("edge_id", &self.edge_id, "edge-")?;
        validate_topic_segment("edge_node_id", &self.edge_node_id)?;
        validate_identity("ledger_epoch", &self.ledger_epoch)?;
        if self.status != "applied" {
            return invalid("activation result status must be applied");
        }
        if self.discard_through_reading_seq < 0 {
            return invalid("discard boundary must be non-negative");
        }
        if self.first_publication_seq != 1 {
            return invalid("first_publication_seq must be 1");
        }
        if self.applied_at < 0 {
            return invalid("activation applied_at must be non-negative");
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, PublishError> {
        self.validate()?;
        serde_json::to_vec(self)
            .map_err(|error| PublishError::Invalid(format!("activation result JSON: {error}")))
    }
}

pub fn activation_state(conn: &Connection) -> Result<ActivationState, PublishError> {
    let value: String = conn.query_row(
        "SELECT state FROM edge_node_activation WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    ActivationState::from_db(&value)
}

pub fn publication_admitted(conn: &Connection) -> Result<bool, PublishError> {
    Ok(activation_state(conn)? != ActivationState::DiscoveryOnly)
}

pub fn install_edge_target(
    conn: &Connection,
    target: &TargetRow,
    now_ms: i64,
) -> Result<(), PublishError> {
    if target.cursor_pub_seq != 0 || target.cursor_epoch.is_some() {
        return invalid("new IoTKit Edge target cursor must be unused");
    }
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if activation_state(&tx)? != ActivationState::Standalone {
        return invalid("IoTKit Edge target may only be installed on a standalone Edge Node");
    }
    let existing_targets: i64 =
        tx.query_row("SELECT count(*) FROM target_registry", [], |row| row.get(0))?;
    let publication_rows: i64 =
        tx.query_row("SELECT count(*) FROM publication_log", [], |row| row.get(0))?;
    let allocation_sequence = publication_allocation_sequence(&tx)?;
    if existing_targets != 0 || publication_rows != 0 || allocation_sequence != 0 {
        return invalid(
            "standalone outbox adoption is unsupported; Edge Node activation requires an unused publication stream",
        );
    }
    tx.execute(
        "INSERT INTO target_registry(
             target_id, endpoint_url, credential_token, archive_responsible,
             schema_version, cursor_epoch, cursor_pub_seq, created_at
         ) VALUES(?1, ?2, ?3, ?4, ?5, NULL, 0, ?6)",
        params![
            target.target_id,
            target.endpoint_url,
            target.credential_token,
            target.archive_responsible,
            target.schema_version,
            now_ms
        ],
    )?;
    tx.execute(
        "UPDATE edge_node_activation
         SET state = 'discovery_only'
         WHERE singleton = 1 AND state = 'standalone'",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn apply_activation(
    conn: &Connection,
    request: &ActivationRequest,
    now_ms: i64,
) -> Result<ActivationResult, PublishError> {
    request.validate()?;
    if now_ms < 0 {
        return invalid("activation applied_at must be non-negative");
    }
    let request_json = serde_json::to_string(request)
        .map_err(|error| PublishError::Invalid(format!("activation request JSON: {error}")))?;
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let identity = iotkit_core_ledger::load_edge_node_identity(&tx)
        .map_err(|error| PublishError::Ledger(error.to_string()))?;
    if request.edge_node_id != identity.edge_node_id {
        return invalid("activation request edge_node_id does not match this Edge Node");
    }
    if request.expected_ledger_epoch != identity.ledger_epoch {
        return invalid("activation request ledger epoch does not match this Edge Node");
    }

    let (state, stored_request, stored_result): (String, Option<String>, Option<String>) = tx
        .query_row(
            "SELECT state, request_json, result_json
             FROM edge_node_activation WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    match ActivationState::from_db(&state)? {
        ActivationState::Active => {
            if stored_request.as_deref() != Some(request_json.as_str()) {
                return invalid("Edge Node is already active under a conflicting activation");
            }
            let result = ActivationResult::decode(
                stored_result
                    .as_deref()
                    .ok_or_else(|| {
                        PublishError::Invalid(
                            "active Edge Node has no stored activation result".into(),
                        )
                    })?
                    .as_bytes(),
            )?;
            tx.commit()?;
            return Ok(result);
        }
        ActivationState::Standalone => {
            return invalid("Edge Node has no IoTKit Edge target and cannot be activated");
        }
        ActivationState::DiscoveryOnly => {}
    }

    let target_count: i64 =
        tx.query_row("SELECT count(*) FROM target_registry", [], |row| row.get(0))?;
    let bad_cursor: i64 = tx.query_row(
        "SELECT count(*) FROM target_registry
         WHERE cursor_pub_seq != 0 OR cursor_epoch IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    let publication_rows: i64 =
        tx.query_row("SELECT count(*) FROM publication_log", [], |row| row.get(0))?;
    let allocation_sequence = publication_allocation_sequence(&tx)?;
    if target_count != 1 || bad_cursor != 0 || publication_rows != 0 || allocation_sequence != 0 {
        return invalid("activation requires an unused publication stream");
    }

    let discard_through_reading_seq: i64 =
        tx.query_row("SELECT COALESCE(MAX(seq), 0) FROM readings", [], |row| {
            row.get(0)
        })?;
    let result = ActivationResult {
        schema_version: ACTIVATION_SCHEMA_VERSION,
        activation_id: request.activation_id.clone(),
        edge_id: request.edge_id.clone(),
        edge_node_id: identity.edge_node_id,
        ledger_epoch: identity.ledger_epoch,
        status: "applied".into(),
        discard_through_reading_seq,
        first_publication_seq: 1,
        applied_at: now_ms,
    };
    let result_json = serde_json::to_string(&result)
        .map_err(|error| PublishError::Invalid(format!("activation result JSON: {error}")))?;
    tx.execute(
        "UPDATE edge_node_activation
         SET state = 'active',
             edge_id = ?1,
             activation_id = ?2,
             ledger_epoch = ?3,
             discard_through_reading_seq = ?4,
             cleanup_through_reading_seq = 0,
             request_json = ?5,
             result_json = ?6,
             activated_at = ?7
         WHERE singleton = 1 AND state = 'discovery_only'",
        params![
            result.edge_id,
            result.activation_id,
            result.ledger_epoch,
            result.discard_through_reading_seq,
            request_json,
            result_json,
            now_ms
        ],
    )?;
    tx.commit()?;
    Ok(result)
}

pub fn cleanup_pre_activation_batch(conn: &Connection, limit: u32) -> Result<u64, PublishError> {
    if limit == 0 {
        return Ok(0);
    }
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let row: Option<(i64, i64)> = tx
        .query_row(
            "SELECT discard_through_reading_seq, cleanup_through_reading_seq
             FROM edge_node_activation
             WHERE singleton = 1 AND state = 'active'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((boundary, progress)) = row else {
        tx.commit()?;
        return Ok(0);
    };
    let batch_end: Option<i64> = tx.query_row(
        "SELECT MAX(seq) FROM (
             SELECT seq FROM readings
             WHERE seq > ?1 AND seq <= ?2
             ORDER BY seq
             LIMIT ?3
         )",
        params![progress, boundary, limit],
        |row| row.get(0),
    )?;
    let Some(batch_end) = batch_end else {
        if progress < boundary {
            tx.execute(
                "UPDATE edge_node_activation
                 SET cleanup_through_reading_seq = ?1
                 WHERE singleton = 1",
                [boundary],
            )?;
        }
        tx.commit()?;
        return Ok(0);
    };
    let changed = tx.execute(
        "DELETE FROM readings WHERE seq > ?1 AND seq <= ?2",
        params![progress, batch_end],
    )?;
    tx.execute(
        "UPDATE edge_node_activation
         SET cleanup_through_reading_seq = ?1
         WHERE singleton = 1",
        [batch_end],
    )?;
    tx.commit()?;
    Ok(changed as u64)
}

fn publication_allocation_sequence(conn: &Connection) -> Result<i64, PublishError> {
    conn.query_row(
        "SELECT seq FROM sqlite_sequence WHERE name = 'publication_log'",
        [],
        |row| row.get(0),
    )
    .optional()
    .map(|value| value.unwrap_or(0))
    .map_err(PublishError::from)
}

/// Returns the durable publication sequence allocation high-water mark.
pub fn publication_allocation_high_water(conn: &Connection) -> Result<i64, PublishError> {
    publication_allocation_sequence(conn)
}

fn validate_prefixed_hex(field: &str, value: &str, prefix: &str) -> Result<(), PublishError> {
    let Some(random) = value.strip_prefix(prefix) else {
        return invalid(&format!("{field} must start with {prefix}"));
    };
    if random.len() != 32
        || !random
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return invalid(&format!(
            "{field} must contain 128-bit lowercase hexadecimal"
        ));
    }
    Ok(())
}

fn validate_topic_segment(field: &str, value: &str) -> Result<(), PublishError> {
    validate_identity(field, value)?;
    if value.contains(['/', '+', '#']) {
        return invalid(&format!("{field} is not a safe MQTT topic segment"));
    }
    Ok(())
}

fn validate_identity(field: &str, value: &str) -> Result<(), PublishError> {
    if value.is_empty()
        || value.len() > 255
        || value.contains(':')
        || value.chars().any(char::is_control)
    {
        return invalid(&format!("{field} is not a valid identity"));
    }
    Ok(())
}

fn invalid<T>(message: &str) -> Result<T, PublishError> {
    Err(PublishError::Invalid(message.into()))
}

#[cfg(test)]
#[path = "../tests/unit/activation_tests.rs"]
mod tests;
