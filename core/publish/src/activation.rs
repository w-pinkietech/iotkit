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
mod tests {
    use super::{
        ActivationRequest, ActivationResult, ActivationState, activation_state, apply_activation,
        cleanup_pre_activation_batch, install_edge_target,
    };
    use crate::store::TargetRow;
    use rusqlite::{Connection, params};

    const EDGE_ID: &str = "edge-node-01";
    const EPOCH: &str = "01JTESTEPOCH";

    fn initialize_identity(conn: &Connection) {
        conn.execute(
            "INSERT INTO ledger_meta(key, value) VALUES('edge_node_id', ?1)",
            [EDGE_ID],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ledger_meta(key, value) VALUES('epoch', ?1)",
            [EPOCH],
        )
        .unwrap();
    }

    fn target() -> TargetRow {
        TargetRow {
            target_id: "edge".into(),
            endpoint_url: "mqtts://broker.example.test:8883".into(),
            credential_token: "secret".into(),
            archive_responsible: true,
            schema_version: 1,
            cursor_epoch: None,
            cursor_pub_seq: 0,
        }
    }

    fn request(activation_id: &str) -> ActivationRequest {
        ActivationRequest {
            schema_version: 1,
            activation_id: activation_id.into(),
            edge_id: "edge-0123456789abcdef0123456789abcdef".into(),
            edge_node_id: EDGE_ID.into(),
            expected_ledger_epoch: EPOCH.into(),
            grant_revision: 1,
            issued_at: 1_720_000_000_000,
        }
    }

    fn seed_readings(conn: &Connection, count: i64) {
        conn.execute(
            "INSERT INTO devices(system_id, hardware_id, kind, state, created_at)
             VALUES(zeroblob(16), 'test-device', 'individual', 'active', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO series(system_id, measurement_key, created_at)
             VALUES(zeroblob(16), 'temperature', 1)",
            [],
        )
        .unwrap();
        for received_at in 1..=count {
            conn.execute(
                "INSERT INTO readings(
                     series_id, received_at, time_source, values_json
                 ) VALUES(1, ?1, 'edge', '[20.0]')",
                [received_at],
            )
            .unwrap();
        }
    }

    fn discovery_only() -> Connection {
        let conn = crate::tests_support::open();
        initialize_identity(&conn);
        assert_eq!(
            activation_state(&conn).unwrap(),
            ActivationState::Standalone
        );
        install_edge_target(&conn, &target(), 1).unwrap();
        assert_eq!(
            activation_state(&conn).unwrap(),
            ActivationState::DiscoveryOnly
        );
        conn
    }

    #[test]
    fn fresh_target_waits_for_activation_and_exact_duplicate_replays_result() {
        let conn = discovery_only();
        seed_readings(&conn, 2);
        let original = request("act-0123456789abcdef0123456789abcdef");

        let result = apply_activation(&conn, &original, 1_720_000_001_000).unwrap();

        assert_eq!(result.first_publication_seq, 1);
        assert_eq!(result.discard_through_reading_seq, 2);
        assert_eq!(result.status, "applied");
        assert_eq!(activation_state(&conn).unwrap(), ActivationState::Active);
        assert_eq!(
            apply_activation(&conn, &original, 1_720_000_009_999).unwrap(),
            result
        );
        assert!(
            apply_activation(
                &conn,
                &request("act-fedcba9876543210fedcba9876543210"),
                1_720_000_010_000
            )
            .is_err()
        );
        assert_eq!(
            crate::store::enqueue_measurement(&conn, EPOCH, 3, 1_720_000_010_001).unwrap(),
            1
        );
    }

    #[test]
    fn activation_rejects_any_prior_publication_allocation_or_cursor() {
        for poison in ["row", "sequence", "cursor"] {
            let conn = discovery_only();
            match poison {
                "row" => {
                    conn.execute(
                        "INSERT INTO publication_log(epoch, kind, created_at)
                         VALUES(?1, 'annotation', 1)",
                        [EPOCH],
                    )
                    .unwrap();
                }
                "sequence" => {
                    conn.execute(
                        "INSERT INTO publication_log(epoch, kind, created_at)
                         VALUES(?1, 'annotation', 1)",
                        [EPOCH],
                    )
                    .unwrap();
                    conn.execute("DELETE FROM publication_log", []).unwrap();
                }
                "cursor" => {
                    conn.execute("UPDATE target_registry SET cursor_pub_seq = 1", [])
                        .unwrap();
                }
                _ => unreachable!(),
            }

            let error =
                apply_activation(&conn, &request("act-0123456789abcdef0123456789abcdef"), 2)
                    .unwrap_err();
            assert!(
                error.to_string().contains("unused publication stream"),
                "{poison}: {error}"
            );
        }
    }

    #[test]
    fn activation_requires_the_initialized_edge_and_exact_epoch() {
        let conn = discovery_only();
        for mut invalid in [
            request("act-0123456789abcdef0123456789abcdef"),
            request("act-fedcba9876543210fedcba9876543210"),
        ] {
            if invalid.activation_id.starts_with("act-0") {
                invalid.edge_node_id = "edge-node-other".into();
            } else {
                invalid.expected_ledger_epoch = "01JOTHER".into();
            }
            assert!(apply_activation(&conn, &invalid, 2).is_err());
        }
        assert_eq!(
            activation_state(&conn).unwrap(),
            ActivationState::DiscoveryOnly
        );
    }

    #[test]
    fn cleanup_deletes_only_the_frozen_prefix_in_bounded_batches() {
        let conn = discovery_only();
        seed_readings(&conn, 3);
        apply_activation(&conn, &request("act-0123456789abcdef0123456789abcdef"), 10).unwrap();
        conn.execute(
            "INSERT INTO readings(series_id, received_at, time_source, values_json)
             VALUES(1, 4, 'edge', '[21.0]')",
            [],
        )
        .unwrap();

        assert_eq!(cleanup_pre_activation_batch(&conn, 2).unwrap(), 2);
        assert_eq!(cleanup_pre_activation_batch(&conn, 2).unwrap(), 1);
        assert_eq!(cleanup_pre_activation_batch(&conn, 2).unwrap(), 0);
        let remaining: Vec<i64> = conn
            .prepare("SELECT seq FROM readings ORDER BY seq")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(remaining, vec![4]);
    }

    #[test]
    fn request_decoder_is_strict_about_contract_fields() {
        let valid = serde_json::to_vec(&request("act-0123456789abcdef0123456789abcdef")).unwrap();
        assert_eq!(
            ActivationRequest::decode(&valid).unwrap(),
            request("act-0123456789abcdef0123456789abcdef")
        );

        for invalid in [
            br#"{"schema_version":2,"activation_id":"act-0123456789abcdef0123456789abcdef","edge_id":"edge-0123456789abcdef0123456789abcdef","edge_node_id":"edge-node-01","expected_ledger_epoch":"01JTESTEPOCH","grant_revision":1,"issued_at":1}"#.as_slice(),
            br#"{"schema_version":1,"activation_id":"act-0123456789ABCDEF0123456789ABCDEF","edge_id":"edge-0123456789abcdef0123456789abcdef","edge_node_id":"edge-node-01","expected_ledger_epoch":"01JTESTEPOCH","grant_revision":1,"issued_at":1}"#.as_slice(),
            br#"{"schema_version":1,"activation_id":"act-0123456789abcdef0123456789abcdef","edge_id":"edge-0123456789abcdef0123456789abcdef","edge_node_id":"edge-node-01","expected_ledger_epoch":"01JTESTEPOCH","grant_revision":1,"issued_at":1,"unknown":true}"#.as_slice(),
        ] {
            assert!(ActivationRequest::decode(invalid).is_err());
        }
    }

    #[test]
    fn shared_activation_fixtures_match_the_edge_contract() {
        let valid_request = ActivationRequest::decode(include_bytes!(
            "../../../testdata/egress/v1/activation-request.json"
        ))
        .unwrap();
        let valid_result = ActivationResult::decode(include_bytes!(
            "../../../testdata/egress/v1/activation-result.json"
        ))
        .unwrap();
        assert_eq!(valid_request.edge_node_id, valid_result.edge_node_id);
        assert_eq!(
            valid_request.expected_ledger_epoch,
            valid_result.ledger_epoch
        );
        for invalid in [
            include_bytes!("../../../testdata/egress/v1/activation-request-malformed-id.json")
                .as_slice(),
            include_bytes!("../../../testdata/egress/v1/activation-request-unknown-field.json")
                .as_slice(),
        ] {
            assert!(ActivationRequest::decode(invalid).is_err());
        }
        assert!(
            ActivationResult::decode(include_bytes!(
                "../../../testdata/egress/v1/activation-result-first-seq-2.json"
            ))
            .is_err()
        );

        let conn = discovery_only();
        for contextual_mismatch in [
            include_bytes!("../../../testdata/egress/v1/activation-request-wrong-edge-node.json")
                .as_slice(),
            include_bytes!("../../../testdata/egress/v1/activation-request-wrong-epoch.json")
                .as_slice(),
        ] {
            let request = ActivationRequest::decode(contextual_mismatch).unwrap();
            assert!(apply_activation(&conn, &request, 1_720_000_001_000).is_err());
        }
        let original = ActivationRequest::decode(include_bytes!(
            "../../../testdata/egress/v1/activation-request.json"
        ))
        .unwrap();
        apply_activation(&conn, &original, 1_720_000_001_000).unwrap();
        let conflicting = ActivationRequest::decode(include_bytes!(
            "../../../testdata/egress/v1/activation-request-conflicting-id.json"
        ))
        .unwrap();
        assert!(apply_activation(&conn, &conflicting, 1_720_000_002_000).is_err());
    }

    #[test]
    fn existing_target_migrates_as_active() {
        let conn = Connection::open_in_memory().unwrap();
        let mut before_activation = Vec::new();
        before_activation.extend_from_slice(iotkit_core_storage::MIGRATIONS);
        before_activation.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
        before_activation.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
        before_activation.push(crate::MIGRATIONS[0]);
        before_activation.sort_by_key(|migration| migration.version);
        iotkit_core_storage::run_migrations(&conn, &before_activation).unwrap();
        conn.execute(
            "INSERT INTO target_registry(
                 target_id, endpoint_url, credential_token, archive_responsible,
                 schema_version, cursor_pub_seq, created_at
             ) VALUES('edge', 'mqtts://broker', 'secret', 1, 1, 0, 1)",
            [],
        )
        .unwrap();

        let mut after_activation = before_activation;
        after_activation.push(crate::MIGRATIONS[1]);
        after_activation.sort_by_key(|migration| migration.version);
        iotkit_core_storage::run_migrations(&conn, &after_activation).unwrap();

        assert_eq!(activation_state(&conn).unwrap(), ActivationState::Active);
    }

    #[test]
    fn target_install_rejects_a_used_standalone_outbox() {
        let conn = crate::tests_support::open();
        initialize_identity(&conn);
        conn.execute(
            "INSERT INTO publication_log(epoch, kind, created_at)
             VALUES(?1, 'annotation', 1)",
            params![EPOCH],
        )
        .unwrap();

        let error = install_edge_target(&conn, &target(), 2).unwrap_err();

        assert!(error.to_string().contains("standalone outbox adoption"));
        assert_eq!(crate::store::target_count(&conn).unwrap(), 0);
        assert_eq!(
            activation_state(&conn).unwrap(),
            ActivationState::Standalone
        );
    }
}
