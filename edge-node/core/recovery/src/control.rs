use std::fmt;

use iotkit_core_ops::{
    Actor, ActorKind, DispatchRequest, DispatchResult, OpContext, OpDescriptor, OpError, Tier,
    dispatch,
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::RecoveryError;
#[cfg(test)]
use crate::{RecoveryStartupMode, startup_mode};

pub const APPLY_RECOVERY_ACTIVATION_OP: &str = "recovery.activation.apply";
pub const COMPLETE_RECOVERY_ACTIVATION_OP: &str = "recovery.activation.complete";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryControlError {
    Invalid,
}

impl fmt::Display for RecoveryControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Edge Node recovery control message is invalid")
    }
}

impl std::error::Error for RecoveryControlError {}

pub(crate) fn control_descriptors() -> Vec<OpDescriptor> {
    vec![
        OpDescriptor {
            name: APPLY_RECOVERY_ACTIVATION_OP,
            tier: Tier::Construction,
            bulk_escalates: false,
            changes_state: true,
            params_schema: private_schema,
            targets: |_| Vec::new(),
            preconditions: apply_preconditions,
            dry_run: |_, _| Ok(json!({"would": "apply_recovery_activation"})),
            execute: apply_execute,
            secret_execute: None,
        },
        OpDescriptor {
            name: COMPLETE_RECOVERY_ACTIVATION_OP,
            tier: Tier::Construction,
            bulk_escalates: false,
            changes_state: true,
            params_schema: private_schema,
            targets: |_| Vec::new(),
            preconditions: completion_preconditions,
            dry_run: |_, _| Ok(json!({"would": "complete_recovery_activation"})),
            execute: completion_execute,
            secret_execute: None,
        },
    ]
}

pub fn apply_recovery_activation(
    conn: &rusqlite::Connection,
    request: &RecoveryActivationRequest,
    applied_at: i64,
) -> Result<RecoveryActivationResult, RecoveryError> {
    request
        .validate()
        .map_err(|_| RecoveryError::RecoveryControlInvalid)?;
    if applied_at < 0 {
        return Err(RecoveryError::RecoveryControlInvalid);
    }
    let output = dispatch_recovery(
        conn,
        APPLY_RECOVERY_ACTIVATION_OP,
        json!({"request": request, "applied_at": applied_at}),
    )?;
    serde_json::from_value(output).map_err(|_| RecoveryError::RecoveryConflict)
}

pub fn complete_recovery_activation(
    conn: &rusqlite::Connection,
    completion: &RecoveryCompletion,
    observed_at: i64,
) -> Result<(), RecoveryError> {
    completion
        .validate()
        .map_err(|_| RecoveryError::RecoveryControlInvalid)?;
    if observed_at < 0 {
        return Err(RecoveryError::RecoveryControlInvalid);
    }
    dispatch_recovery(
        conn,
        COMPLETE_RECOVERY_ACTIVATION_OP,
        json!({"completion": completion, "observed_at": observed_at}),
    )?;
    Ok(())
}

pub fn recovery_activation_result(
    conn: &rusqlite::Connection,
) -> Result<Option<RecoveryActivationResult>, RecoveryError> {
    stored_result(conn).map_err(|_| RecoveryError::RecoveryConflict)
}

fn dispatch_recovery(
    conn: &rusqlite::Connection,
    operation: &str,
    state: Value,
) -> Result<Value, RecoveryError> {
    dispatch(
        conn,
        crate::recovery_descriptors(),
        DispatchRequest {
            op: operation.into(),
            params: json!({"private_recovery_state": state}),
            dry_run: false,
            actor: Actor {
                actor_id: "recovery-control".into(),
                actor_kind: ActorKind::LocalCli,
                tier_ceiling: Tier::Construction,
            },
            source: None,
            step_up_verified: true,
            clock_trust: None,
        },
    )
    .and_then(DispatchResult::into_public)
    .map_err(|_| RecoveryError::RecoveryConflict)
}

fn private_schema() -> Value {
    json!({"required": ["private_recovery_state"]})
}

fn private_state<T: for<'de> Deserialize<'de>>(context: &OpContext<'_>) -> Result<T, OpError> {
    serde_json::from_value(
        context
            .params
            .get("private_recovery_state")
            .cloned()
            .ok_or_else(|| OpError::Validation("private_recovery_state".into()))?,
    )
    .map_err(|_| OpError::Validation("private_recovery_state".into()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyState {
    request: RecoveryActivationRequest,
    applied_at: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionState {
    completion: RecoveryCompletion,
    observed_at: i64,
}

fn apply_preconditions(tx: &Transaction<'_>, context: &OpContext<'_>) -> Result<(), OpError> {
    let state: ApplyState = private_state(context)?;
    state
        .request
        .validate()
        .map_err(|_| OpError::Validation("recovery_request".into()))?;
    if state.applied_at < 0 {
        return Err(OpError::Validation("recovery_request".into()));
    }
    if let Some(stored) = stored_request(tx)? {
        return if stored == state.request {
            Ok(())
        } else {
            Err(OpError::PreconditionFailed("recovery_conflict".into()))
        };
    }

    let candidate: (
        String,
        String,
        Option<String>,
        String,
        String,
        String,
        String,
        i64,
    ) = tx
        .query_row(
            "SELECT recovery_id,candidate_instance_id,backup_id,edge_id,edge_node_id,
                    old_ledger_epoch,proposed_new_epoch,credential_generation
             FROM edge_node_recovery_candidate WHERE singleton=1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .map_err(|_| OpError::PreconditionFailed("recovery_candidate".into()))?;
    if candidate.0 != state.request.recovery_id
        || candidate.1 != state.request.candidate_instance_id
        || candidate.2.as_deref() != Some(state.request.backup_id.as_str())
        || candidate.3 != state.request.edge_id
        || candidate.4 != state.request.edge_node_id
        || candidate.5 != state.request.old_ledger_epoch
        || candidate.6 != state.request.new_ledger_epoch
        || candidate.7 != state.request.broker_credential_generation
    {
        return Err(OpError::PreconditionFailed("recovery_candidate".into()));
    }
    let identity = iotkit_core_ledger::load_edge_node_identity(tx)
        .map_err(|_| OpError::PreconditionFailed("recovery_identity".into()))?;
    let device_generation = iotkit_core_ops::device_auth_generation(tx)
        .map_err(|_| OpError::PreconditionFailed("recovery_generation".into()))?;
    let target = iotkit_core_publish::store::target_get(tx)
        .map_err(|_| OpError::PreconditionFailed("recovery_cursor".into()))?
        .ok_or_else(|| OpError::PreconditionFailed("recovery_cursor".into()))?;
    let allocation_high_water =
        iotkit_core_publish::activation::publication_allocation_high_water(tx)
            .map_err(|_| OpError::PreconditionFailed("recovery_cursor".into()))?;
    let epoch_start_publication_seq: Option<i64> = tx
        .query_row(
            "SELECT pub_seq FROM publication_log
             WHERE epoch=?1 AND kind='annotation' AND subtype='epoch_start'",
            [&state.request.old_ledger_epoch],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| OpError::PreconditionFailed("recovery_cursor".into()))?;
    let activation: (String, Option<String>, Option<String>) = tx
        .query_row(
            "SELECT state,edge_id,ledger_epoch FROM edge_node_activation WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| OpError::PreconditionFailed("recovery_activation".into()))?;
    if identity.edge_node_id != state.request.edge_node_id
        || identity.ledger_epoch != state.request.old_ledger_epoch
        || device_generation != state.request.device_auth_generation
        || target.cursor_epoch.as_deref() != Some(state.request.old_ledger_epoch.as_str())
        || target.cursor_pub_seq != state.request.snapshot_accepted_through
        || allocation_high_water != state.request.snapshot_allocation_high_water
        || epoch_start_publication_seq != state.request.snapshot_epoch_start_publication_seq
        || activation.0 != "active"
        || activation.1.as_deref() != Some(state.request.edge_id.as_str())
        || activation.2.as_deref() != Some(state.request.old_ledger_epoch.as_str())
    {
        return Err(OpError::PreconditionFailed("recovery_boundary".into()));
    }
    Ok(())
}

fn apply_execute(tx: &Transaction<'_>, context: &OpContext<'_>) -> Result<Value, OpError> {
    let state: ApplyState = private_state(context)?;
    if let Some(result) = stored_result(tx)? {
        return serde_json::to_value(result)
            .map_err(|_| OpError::Internal("recovery_result".into()));
    }
    iotkit_core_ledger::install_recovery_epoch(
        tx,
        &state.request.old_ledger_epoch,
        &state.request.new_ledger_epoch,
    )
    .map_err(|_| OpError::PreconditionFailed("recovery_epoch".into()))?;
    let rebuilt = iotkit_core_publish::store::rebuild_recovery_outbox(
        tx,
        &state.request.old_ledger_epoch,
        &state.request.new_ledger_epoch,
        state.request.edge_accepted_through,
        state.applied_at,
    )
    .map_err(|_| OpError::PreconditionFailed("recovery_outbox".into()))?;
    let result = RecoveryActivationResult {
        schema_version: 1,
        recovery_id: state.request.recovery_id.clone(),
        edge_id: state.request.edge_id.clone(),
        edge_node_id: state.request.edge_node_id.clone(),
        candidate_instance_id: state.request.candidate_instance_id.clone(),
        backup_id: state.request.backup_id.clone(),
        old_ledger_epoch: state.request.old_ledger_epoch.clone(),
        new_ledger_epoch: state.request.new_ledger_epoch.clone(),
        broker_credential_generation: state.request.broker_credential_generation,
        device_auth_generation: state.request.device_auth_generation,
        status: "applied".into(),
        edge_accepted_through: state.request.edge_accepted_through,
        replayed_records: rebuilt.replayed_records,
        first_new_publication_seq: 1,
        last_new_publication_seq: rebuilt.last_new_publication_seq,
        applied_at: state.applied_at,
    };
    let request_json = serde_json::to_string(&state.request)
        .map_err(|_| OpError::Internal("recovery_request".into()))?;
    let result_json =
        serde_json::to_string(&result).map_err(|_| OpError::Internal("recovery_result".into()))?;
    let changed = tx.execute(
        "UPDATE edge_node_activation
         SET ledger_epoch=?1,activation_id=?2,request_json=?3,result_json=?4
         WHERE singleton=1 AND state='active' AND ledger_epoch=?5",
        params![
            state.request.new_ledger_epoch,
            state.request.recovery_id,
            request_json,
            result_json,
            state.request.old_ledger_epoch
        ],
    )?;
    if changed != 1 {
        return Err(OpError::PreconditionFailed("recovery_activation".into()));
    }
    tx.execute(
        "INSERT INTO edge_node_recovery_activation(
             singleton,state,request_json,result_json,applied_at_ms
         ) VALUES(1,'applied',?1,?2,?3)",
        params![request_json, result_json, state.applied_at],
    )?;
    serde_json::to_value(result).map_err(|_| OpError::Internal("recovery_result".into()))
}

fn completion_preconditions(tx: &Transaction<'_>, context: &OpContext<'_>) -> Result<(), OpError> {
    let state: CompletionState = private_state(context)?;
    state
        .completion
        .validate()
        .map_err(|_| OpError::Validation("recovery_completion".into()))?;
    let request = stored_request(tx)?
        .ok_or_else(|| OpError::PreconditionFailed("recovery_activation".into()))?;
    state
        .completion
        .validate_for(&request)
        .map_err(|_| OpError::PreconditionFailed("recovery_conflict".into()))?;
    let (activation_state, stored_completion): (String, Option<String>) = tx.query_row(
        "SELECT state,completion_json FROM edge_node_recovery_activation WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if activation_state == "completed" {
        let exact = stored_completion
            .as_deref()
            .and_then(|value| serde_json::from_str::<RecoveryCompletion>(value).ok())
            .is_some_and(|completion| completion == state.completion);
        return if exact {
            Ok(())
        } else {
            Err(OpError::PreconditionFailed("recovery_conflict".into()))
        };
    }
    if activation_state != "applied" {
        return Err(OpError::PreconditionFailed("recovery_activation".into()));
    }
    Ok(())
}

fn completion_execute(tx: &Transaction<'_>, context: &OpContext<'_>) -> Result<Value, OpError> {
    let state: CompletionState = private_state(context)?;
    if state.observed_at < 0 {
        return Err(OpError::Validation("recovery_completion".into()));
    }
    let current: String = tx.query_row(
        "SELECT state FROM edge_node_recovery_activation WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    if current == "completed" {
        return Ok(json!({"status": "completed"}));
    }
    let completion_json = serde_json::to_string(&state.completion)
        .map_err(|_| OpError::Internal("recovery_completion".into()))?;
    let changed = tx.execute(
        "UPDATE edge_node_recovery_activation
         SET state='completed',completion_json=?1,completed_at_ms=?2
         WHERE singleton=1 AND state='applied'",
        params![completion_json, state.observed_at],
    )?;
    if changed != 1 {
        return Err(OpError::PreconditionFailed("recovery_activation".into()));
    }
    Ok(json!({"status": "completed"}))
}

pub(crate) fn stored_request(
    conn: &rusqlite::Connection,
) -> Result<Option<RecoveryActivationRequest>, OpError> {
    conn.query_row(
        "SELECT request_json FROM edge_node_recovery_activation WHERE singleton=1",
        [],
        |row| row.get::<_, String>(0),
    )
    .optional()?
    .map(|value| {
        RecoveryActivationRequest::decode(value.as_bytes())
            .map_err(|_| OpError::PreconditionFailed("recovery_state".into()))
    })
    .transpose()
}

pub(crate) fn stored_result(
    conn: &rusqlite::Connection,
) -> Result<Option<RecoveryActivationResult>, OpError> {
    conn.query_row(
        "SELECT result_json FROM edge_node_recovery_activation WHERE singleton=1",
        [],
        |row| row.get::<_, String>(0),
    )
    .optional()?
    .map(|value| {
        RecoveryActivationResult::decode(value.as_bytes())
            .map_err(|_| OpError::PreconditionFailed("recovery_state".into()))
    })
    .transpose()
}

impl RecoveryActivationRequest {
    pub fn decode(payload: &[u8]) -> Result<Self, RecoveryControlError> {
        let request: Self =
            serde_json::from_slice(payload).map_err(|_| RecoveryControlError::Invalid)?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), RecoveryControlError> {
        validate_common(
            self.schema_version,
            &self.recovery_id,
            &self.edge_id,
            &self.edge_node_id,
            &self.candidate_instance_id,
        )?;
        prefixed_hex(&self.backup_id, "backup-")?;
        identity(&self.old_ledger_epoch)?;
        identity(&self.new_ledger_epoch)?;
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
            return Err(RecoveryControlError::Invalid);
        }
        Ok(())
    }
}

impl RecoveryActivationResult {
    pub fn decode(payload: &[u8]) -> Result<Self, RecoveryControlError> {
        let result: Self =
            serde_json::from_slice(payload).map_err(|_| RecoveryControlError::Invalid)?;
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), RecoveryControlError> {
        validate_common(
            self.schema_version,
            &self.recovery_id,
            &self.edge_id,
            &self.edge_node_id,
            &self.candidate_instance_id,
        )?;
        prefixed_hex(&self.backup_id, "backup-")?;
        identity(&self.old_ledger_epoch)?;
        identity(&self.new_ledger_epoch)?;
        if self.old_ledger_epoch == self.new_ledger_epoch
            || self.broker_credential_generation < 1
            || self.device_auth_generation < 0
            || self.status != "applied"
            || self.edge_accepted_through < 0
            || self.replayed_records < 0
            || self.first_new_publication_seq != 1
            || self.last_new_publication_seq != self.replayed_records + 1
            || self.applied_at < 0
        {
            return Err(RecoveryControlError::Invalid);
        }
        Ok(())
    }

    pub fn validate_for(
        &self,
        request: &RecoveryActivationRequest,
    ) -> Result<(), RecoveryControlError> {
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
            return Err(RecoveryControlError::Invalid);
        }
        Ok(())
    }
}

impl RecoveryCompletion {
    pub fn decode(payload: &[u8]) -> Result<Self, RecoveryControlError> {
        let completion: Self =
            serde_json::from_slice(payload).map_err(|_| RecoveryControlError::Invalid)?;
        completion.validate()?;
        Ok(completion)
    }

    pub fn validate(&self) -> Result<(), RecoveryControlError> {
        validate_common(
            self.schema_version,
            &self.recovery_id,
            &self.edge_id,
            &self.edge_node_id,
            &self.candidate_instance_id,
        )?;
        identity(&self.new_ledger_epoch)?;
        if self.status != "committed" || self.accepted_through != 0 || self.committed_at < 0 {
            return Err(RecoveryControlError::Invalid);
        }
        Ok(())
    }

    pub fn validate_for(
        &self,
        request: &RecoveryActivationRequest,
    ) -> Result<(), RecoveryControlError> {
        self.validate()?;
        if self.recovery_id != request.recovery_id
            || self.edge_id != request.edge_id
            || self.edge_node_id != request.edge_node_id
            || self.candidate_instance_id != request.candidate_instance_id
            || self.new_ledger_epoch != request.new_ledger_epoch
        {
            return Err(RecoveryControlError::Invalid);
        }
        Ok(())
    }
}

impl RecoveryCompletionAck {
    pub fn for_completion(
        completion: &RecoveryCompletion,
        acknowledged_at: i64,
    ) -> Result<Self, RecoveryControlError> {
        completion.validate()?;
        let acknowledgement = Self {
            schema_version: 1,
            recovery_id: completion.recovery_id.clone(),
            edge_id: completion.edge_id.clone(),
            edge_node_id: completion.edge_node_id.clone(),
            candidate_instance_id: completion.candidate_instance_id.clone(),
            new_ledger_epoch: completion.new_ledger_epoch.clone(),
            status: "completion_stored".into(),
            acknowledged_at,
        };
        acknowledgement.validate()?;
        Ok(acknowledgement)
    }

    pub fn decode(payload: &[u8]) -> Result<Self, RecoveryControlError> {
        let acknowledgement: Self =
            serde_json::from_slice(payload).map_err(|_| RecoveryControlError::Invalid)?;
        acknowledgement.validate()?;
        Ok(acknowledgement)
    }

    pub fn validate(&self) -> Result<(), RecoveryControlError> {
        validate_common(
            self.schema_version,
            &self.recovery_id,
            &self.edge_id,
            &self.edge_node_id,
            &self.candidate_instance_id,
        )?;
        identity(&self.new_ledger_epoch)?;
        if self.status != "completion_stored" || self.acknowledged_at < 0 {
            return Err(RecoveryControlError::Invalid);
        }
        Ok(())
    }
}

fn validate_common(
    schema_version: u32,
    recovery_id: &str,
    edge_id: &str,
    edge_node_id: &str,
    candidate_instance_id: &str,
) -> Result<(), RecoveryControlError> {
    if schema_version != 1 {
        return Err(RecoveryControlError::Invalid);
    }
    prefixed_hex(recovery_id, "recovery-")?;
    prefixed_hex(edge_id, "edge-")?;
    topic_segment(edge_node_id)?;
    prefixed_hex(candidate_instance_id, "candidate-")
}

fn prefixed_hex(value: &str, prefix: &str) -> Result<(), RecoveryControlError> {
    let Some(random) = value.strip_prefix(prefix) else {
        return Err(RecoveryControlError::Invalid);
    };
    if random.len() != 32
        || !random
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(RecoveryControlError::Invalid);
    }
    Ok(())
}

fn topic_segment(value: &str) -> Result<(), RecoveryControlError> {
    identity(value)?;
    if value.contains(['/', '+', '#']) {
        return Err(RecoveryControlError::Invalid);
    }
    Ok(())
}

fn identity(value: &str) -> Result<(), RecoveryControlError> {
    if value.is_empty()
        || value.len() > 255
        || value.contains(':')
        || value.chars().any(char::is_control)
    {
        return Err(RecoveryControlError::Invalid);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/control_tests.rs"]
mod tests;
