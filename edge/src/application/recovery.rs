use crate::storage::{RecoveryCase, RecoveryPrepare, Storage, StorageError};
use iotkit_edge_custody_contract::RecoveryActivationRequest;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackupInspection {
    pub status: String,
    pub artifact_kind: String,
    pub format_version: u32,
    pub backup_id: String,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub created_at_ms: i64,
    pub accepted_cursor: i64,
    pub allocation_high_water: i64,
    pub epoch_start_publication_seq: Option<i64>,
    pub snapshot_mode: String,
    pub schema_version: u32,
    pub database_length: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerFenceReceipt {
    pub schema_version: u32,
    pub status: String,
    pub fence_id: String,
    pub edge_node_id: String,
    pub credential_generation: i64,
    pub fenced_at: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreReceipt {
    pub schema_version: u32,
    pub status: String,
    pub recovery_id: String,
    pub candidate_instance_id: String,
    pub backup_id: String,
    pub edge_id: String,
    pub edge_node_id: String,
    pub old_ledger_epoch: String,
    pub proposed_new_epoch: String,
    pub credential_generation: i64,
    pub device_auth_generation: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryHandoff {
    pub schema_version: u32,
    pub recovery_id: String,
    pub edge_id: String,
    pub edge_node_id: String,
    pub old_ledger_epoch: String,
    pub expected_backup_id: Option<String>,
    pub proposed_new_epoch: String,
    pub credential_generation: i64,
}

#[derive(Clone)]
pub struct RecoveryService {
    storage: Storage,
}

impl RecoveryService {
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    pub async fn prepare(
        &self,
        inspection: &BackupInspection,
        fence: &BrokerFenceReceipt,
        now: i64,
    ) -> Result<RecoveryHandoff, RecoveryApplicationError> {
        if inspection.status != "authenticated"
            || inspection.artifact_kind != "iotkit-node-backup"
            || inspection.format_version != 1
            || inspection.backup_id.is_empty()
            || inspection.edge_node_id.is_empty()
            || inspection.ledger_epoch.is_empty()
            || inspection.created_at_ms < 0
            || inspection.accepted_cursor < 0
            || inspection.allocation_high_water < inspection.accepted_cursor
            || inspection
                .epoch_start_publication_seq
                .is_some_and(|sequence| sequence < 1 || sequence > inspection.allocation_high_water)
            || inspection.snapshot_mode != "online"
            || !(23..=24).contains(&inspection.schema_version)
            || inspection.database_length == 0
            || fence.schema_version != 1
            || fence.status != "fenced"
            || fence.edge_node_id != inspection.edge_node_id
            || fence.credential_generation <= 0
            || fence.fenced_at < 0
            || fence.fenced_at > now
        {
            return Err(RecoveryApplicationError::InvalidEvidence);
        }
        let fence_suffix = fence
            .fence_id
            .strip_prefix("fence-")
            .filter(|suffix| {
                suffix.len() == 32
                    && suffix
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
            .ok_or(RecoveryApplicationError::InvalidEvidence)?;
        let recovery_id = format!("recovery-{fence_suffix}");
        let new_epoch = format!("epoch-{fence_suffix}");
        let edge_id = self.storage.edge_id().await?;
        let recovery_case = self
            .storage
            .prepare_edge_node_recovery(
                &RecoveryPrepare {
                    recovery_id: recovery_id.clone(),
                    edge_node_id: inspection.edge_node_id.clone(),
                    backup_id: inspection.backup_id.clone(),
                    old_ledger_epoch: inspection.ledger_epoch.clone(),
                    new_ledger_epoch: new_epoch.clone(),
                    broker_fence_id: fence.fence_id.clone(),
                    broker_credential_generation: fence.credential_generation,
                    backup_created_at: inspection.created_at_ms,
                    broker_fenced_at: fence.fenced_at,
                    snapshot_accepted_through: inspection.accepted_cursor,
                    snapshot_allocation_high_water: inspection.allocation_high_water,
                    snapshot_epoch_start_publication_seq: inspection.epoch_start_publication_seq,
                },
                now,
            )
            .await?;
        Ok(RecoveryHandoff {
            schema_version: 1,
            recovery_id: recovery_case.recovery_id,
            edge_id,
            edge_node_id: inspection.edge_node_id.clone(),
            old_ledger_epoch: inspection.ledger_epoch.clone(),
            expected_backup_id: Some(inspection.backup_id.clone()),
            proposed_new_epoch: recovery_case.new_ledger_epoch,
            credential_generation: fence.credential_generation,
        })
    }

    pub async fn authorize(
        &self,
        receipt: &RestoreReceipt,
        now: i64,
    ) -> Result<RecoveryActivationRequest, RecoveryApplicationError> {
        if receipt.schema_version != 2
            || receipt.status != "durably_fenced_candidate"
            || receipt.device_auth_generation < 0
        {
            return Err(RecoveryApplicationError::InvalidEvidence);
        }
        let case = self.storage.recovery_case(&receipt.recovery_id).await?;
        let edge_id = self.storage.edge_id().await?;
        if !matches!(case.state.as_str(), "prepared" | "authorized")
            || receipt.edge_id != edge_id
            || receipt.edge_node_id != case.edge_node_id
            || receipt.backup_id != case.backup_id
            || receipt.old_ledger_epoch != case.old_ledger_epoch
            || receipt.proposed_new_epoch != case.new_ledger_epoch
            || receipt.credential_generation != case.broker_credential_generation
        {
            return Err(RecoveryApplicationError::InvalidEvidence);
        }
        if case.state == "authorized" {
            if case.candidate_instance_id.as_deref() != Some(receipt.candidate_instance_id.as_str())
                || case.device_auth_generation != Some(receipt.device_auth_generation)
            {
                return Err(RecoveryApplicationError::InvalidEvidence);
            }
            return self
                .storage
                .recovery_activation_request(&receipt.recovery_id)
                .await
                .map_err(Into::into);
        }
        let request = RecoveryActivationRequest {
            schema_version: 1,
            recovery_id: case.recovery_id,
            edge_id,
            edge_node_id: case.edge_node_id,
            candidate_instance_id: receipt.candidate_instance_id.clone(),
            backup_id: case.backup_id,
            old_ledger_epoch: case.old_ledger_epoch,
            new_ledger_epoch: case.new_ledger_epoch,
            broker_credential_generation: case.broker_credential_generation,
            device_auth_generation: receipt.device_auth_generation,
            snapshot_accepted_through: case.snapshot_accepted_through,
            snapshot_allocation_high_water: case.snapshot_allocation_high_water,
            snapshot_epoch_start_publication_seq: case.snapshot_epoch_start_publication_seq,
            edge_accepted_through: case.edge_accepted_through,
            grant_revision: 1,
            issued_at: now,
        };
        self.storage
            .authorize_edge_node_recovery(&request, now)
            .await?;
        Ok(request)
    }

    pub async fn report(
        &self,
        recovery_id: &str,
    ) -> Result<RecoveryReport, RecoveryApplicationError> {
        let case = self.storage.recovery_case(recovery_id).await?;
        let new_epoch_accepted_through = if case.state == "completed" {
            Some(
                self.storage
                    .accepted_through(&case.edge_node_id, &case.new_ledger_epoch)
                    .await?,
            )
        } else {
            None
        };
        let completion_acknowledged = if case.state == "completed" {
            self.storage
                .recovery_completion_acknowledged(recovery_id)
                .await?
        } else {
            false
        };
        Ok(RecoveryReport::from_case(
            case,
            new_epoch_accepted_through,
            completion_acknowledged,
        ))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryReport {
    pub recovery_id: String,
    pub state: String,
    pub edge_node_id: String,
    pub backup_id: String,
    pub old_ledger_epoch: String,
    pub new_ledger_epoch: String,
    pub broker_credential_generation: i64,
    pub backup_created_at: i64,
    pub broker_fenced_at: i64,
    pub recovery_window_ms: Option<i64>,
    pub snapshot_accepted_through: i64,
    pub snapshot_allocation_high_water: i64,
    pub snapshot_epoch_start_publication_seq: Option<i64>,
    pub potential_unrecoverable_local_after_seq: Option<i64>,
    pub edge_accepted_through: i64,
    pub edge_only_post_backup_start: Option<i64>,
    pub edge_only_post_backup_end: Option<i64>,
    pub replayed_records: Option<i64>,
    pub last_new_publication_seq: Option<i64>,
    pub new_epoch_accepted_through: Option<i64>,
    pub completion_acknowledged: bool,
    pub cursor_converged: bool,
    pub remaining_gap_review_required: bool,
}

impl RecoveryReport {
    fn from_case(
        case: RecoveryCase,
        new_epoch_accepted_through: Option<i64>,
        completion_acknowledged: bool,
    ) -> Self {
        let edge_only = (case.edge_accepted_through > case.snapshot_allocation_high_water)
            .then_some((
                case.snapshot_allocation_high_water + 1,
                case.edge_accepted_through,
            ));
        let cursor_converged = case.state == "completed"
            && case.last_new_publication_seq.is_some_and(|expected| {
                new_epoch_accepted_through.is_some_and(|accepted| accepted >= expected)
            });
        // A lost old host cannot prove whether it allocated additional local
        // readings after the authenticated snapshot. Cursor convergence proves
        // replay custody, not the absence of that unknown tail.
        let remaining_gap_review_required = true;
        Self {
            recovery_id: case.recovery_id,
            state: case.state,
            edge_node_id: case.edge_node_id,
            backup_id: case.backup_id,
            old_ledger_epoch: case.old_ledger_epoch,
            new_ledger_epoch: case.new_ledger_epoch,
            broker_credential_generation: case.broker_credential_generation,
            backup_created_at: case.backup_created_at,
            broker_fenced_at: case.broker_fenced_at,
            // Node backup and Edge/Broker fence timestamps use independent
            // clocks, so their difference is not a trustworthy duration.
            recovery_window_ms: None,
            snapshot_accepted_through: case.snapshot_accepted_through,
            snapshot_allocation_high_water: case.snapshot_allocation_high_water,
            snapshot_epoch_start_publication_seq: case.snapshot_epoch_start_publication_seq,
            potential_unrecoverable_local_after_seq: case
                .snapshot_allocation_high_water
                .checked_add(1),
            edge_accepted_through: case.edge_accepted_through,
            edge_only_post_backup_start: edge_only.map(|range| range.0),
            edge_only_post_backup_end: edge_only.map(|range| range.1),
            replayed_records: case.replayed_records,
            last_new_publication_seq: case.last_new_publication_seq,
            new_epoch_accepted_through,
            completion_acknowledged,
            cursor_converged,
            remaining_gap_review_required,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RecoveryApplicationError {
    #[error("recovery evidence is invalid or conflicts with durable state")]
    InvalidEvidence,
    #[error(transparent)]
    Storage(#[from] StorageError),
}
