use iotkit_edge_custody_contract::{
    RecoveryActivationRequest, RecoveryActivationResult, RecoveryCompletion, RecoveryCompletionAck,
    SCHEMA_VERSION,
};
use sqlx::Row;

use super::{
    AuditActor, Storage, StorageError, StorageInner,
    auth::{insert_audit_postgres, insert_audit_sqlite},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCase {
    pub recovery_id: String,
    pub state: String,
    pub edge_node_id: String,
    pub backup_id: String,
    pub old_ledger_epoch: String,
    pub new_ledger_epoch: String,
    pub broker_fence_id: String,
    pub broker_credential_generation: i64,
    pub backup_created_at: i64,
    pub broker_fenced_at: i64,
    pub device_auth_generation: Option<i64>,
    pub candidate_instance_id: Option<String>,
    pub snapshot_accepted_through: i64,
    pub snapshot_allocation_high_water: i64,
    pub snapshot_epoch_start_publication_seq: Option<i64>,
    pub edge_accepted_through: i64,
    pub replayed_records: Option<i64>,
    pub last_new_publication_seq: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct RecoveryPrepare {
    pub recovery_id: String,
    pub edge_node_id: String,
    pub backup_id: String,
    pub old_ledger_epoch: String,
    pub new_ledger_epoch: String,
    pub broker_fence_id: String,
    pub broker_credential_generation: i64,
    pub backup_created_at: i64,
    pub broker_fenced_at: i64,
    pub snapshot_accepted_through: i64,
    pub snapshot_allocation_high_water: i64,
    pub snapshot_epoch_start_publication_seq: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCommand {
    pub recovery_id: String,
    pub kind: String,
    pub topic: String,
    pub payload_json: Vec<u8>,
    pub attempts: i64,
    pub last_attempt_at: Option<i64>,
}

impl Storage {
    pub async fn prepare_edge_node_recovery(
        &self,
        prepare: &RecoveryPrepare,
        now: i64,
    ) -> Result<RecoveryCase, StorageError> {
        validate_prepare(prepare, now)?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                match load_case_sqlite(&mut tx, &prepare.recovery_id).await {
                    Ok(existing) => {
                        if prepare_matches(&existing, prepare) {
                            tx.commit().await?;
                            return Ok(existing);
                        }
                        return Err(StorageError::RecoveryConflict);
                    }
                    Err(StorageError::Database(sqlx::Error::RowNotFound)) => {}
                    Err(error) => return Err(error),
                }
                let edge_id: String =
                    sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
                        .fetch_one(&mut *tx)
                        .await?;
                let row = sqlx::query(
                    "SELECT activation.state,activation.ledger_epoch,cursor.accepted_through
                     FROM edge_node_activations AS activation
                     JOIN accepted_cursors AS cursor
                       ON cursor.edge_node_id=activation.edge_node_id
                      AND cursor.ledger_epoch=activation.ledger_epoch
                     WHERE activation.edge_node_id=?",
                )
                .bind(&prepare.edge_node_id)
                .fetch_one(&mut *tx)
                .await?;
                validate_active_boundary(
                    row.try_get("state")?,
                    row.try_get("ledger_epoch")?,
                    row.try_get("accepted_through")?,
                    prepare,
                )?;
                let frozen = sqlx::query(
                    "UPDATE edge_node_activations
                     SET state='recovery_hold',revision=revision+1,updated_at=?
                     WHERE edge_node_id=? AND ledger_epoch=? AND state='active'",
                )
                .bind(now)
                .bind(&prepare.edge_node_id)
                .bind(&prepare.old_ledger_epoch)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                if frozen != 1 {
                    return Err(StorageError::RecoveryConflict);
                }
                sqlx::query(
                    "INSERT INTO edge_node_recovery_cases(
                         recovery_id,state,edge_node_id,backup_id,old_ledger_epoch,
                         new_ledger_epoch,broker_fence_id,broker_credential_generation,
                         backup_created_at,broker_fenced_at,
                         snapshot_accepted_through,snapshot_allocation_high_water,
                         snapshot_epoch_start_publication_seq,
                         edge_accepted_through,created_at,updated_at
                     ) VALUES(?1,'prepared',?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
                )
                .bind(&prepare.recovery_id)
                .bind(&prepare.edge_node_id)
                .bind(&prepare.backup_id)
                .bind(&prepare.old_ledger_epoch)
                .bind(&prepare.new_ledger_epoch)
                .bind(&prepare.broker_fence_id)
                .bind(prepare.broker_credential_generation)
                .bind(prepare.backup_created_at)
                .bind(prepare.broker_fenced_at)
                .bind(prepare.snapshot_accepted_through)
                .bind(prepare.snapshot_allocation_high_water)
                .bind(prepare.snapshot_epoch_start_publication_seq)
                .bind(row.try_get::<i64, _>("accepted_through")?)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                insert_audit_sqlite(
                    &mut tx,
                    &AuditActor::LocalCli,
                    now,
                    "edge_node_recovery_prepared",
                    &prepare.recovery_id,
                    serde_json::json!({
                        "edge_id": edge_id,
                        "edge_node_id": prepare.edge_node_id,
                        "backup_id": prepare.backup_id,
                        "old_ledger_epoch": prepare.old_ledger_epoch,
                        "new_ledger_epoch": prepare.new_ledger_epoch,
                        "broker_fence_id": prepare.broker_fence_id,
                        "broker_credential_generation": prepare.broker_credential_generation,
                        "backup_created_at": prepare.backup_created_at,
                        "broker_fenced_at": prepare.broker_fenced_at,
                        "snapshot_accepted_through": prepare.snapshot_accepted_through,
                        "edge_accepted_through": row.try_get::<i64, _>("accepted_through")?,
                    }),
                )
                .await?;
                let case = load_case_sqlite(&mut tx, &prepare.recovery_id).await?;
                tx.commit().await?;
                Ok(case)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                match load_case_postgres(&mut tx, &prepare.recovery_id).await {
                    Ok(existing) => {
                        if prepare_matches(&existing, prepare) {
                            tx.commit().await?;
                            return Ok(existing);
                        }
                        return Err(StorageError::RecoveryConflict);
                    }
                    Err(StorageError::Database(sqlx::Error::RowNotFound)) => {}
                    Err(error) => return Err(error),
                }
                let edge_id: String =
                    sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
                        .fetch_one(&mut *tx)
                        .await?;
                let row = sqlx::query(
                    "SELECT activation.state,activation.ledger_epoch,cursor.accepted_through
                     FROM edge_node_activations AS activation
                     JOIN accepted_cursors AS cursor
                       ON cursor.edge_node_id=activation.edge_node_id
                      AND cursor.ledger_epoch=activation.ledger_epoch
                     WHERE activation.edge_node_id=$1 FOR UPDATE",
                )
                .bind(&prepare.edge_node_id)
                .fetch_one(&mut *tx)
                .await?;
                validate_active_boundary(
                    row.try_get("state")?,
                    row.try_get("ledger_epoch")?,
                    row.try_get("accepted_through")?,
                    prepare,
                )?;
                let frozen = sqlx::query(
                    "UPDATE edge_node_activations
                     SET state='recovery_hold',revision=revision+1,updated_at=$1
                     WHERE edge_node_id=$2 AND ledger_epoch=$3 AND state='active'",
                )
                .bind(now)
                .bind(&prepare.edge_node_id)
                .bind(&prepare.old_ledger_epoch)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                if frozen != 1 {
                    return Err(StorageError::RecoveryConflict);
                }
                sqlx::query(
                    "INSERT INTO edge_node_recovery_cases(
                         recovery_id,state,edge_node_id,backup_id,old_ledger_epoch,
                         new_ledger_epoch,broker_fence_id,broker_credential_generation,
                         backup_created_at,broker_fenced_at,
                         snapshot_accepted_through,snapshot_allocation_high_water,
                         snapshot_epoch_start_publication_seq,
                         edge_accepted_through,created_at,updated_at
                     ) VALUES($1,'prepared',$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$14)",
                )
                .bind(&prepare.recovery_id)
                .bind(&prepare.edge_node_id)
                .bind(&prepare.backup_id)
                .bind(&prepare.old_ledger_epoch)
                .bind(&prepare.new_ledger_epoch)
                .bind(&prepare.broker_fence_id)
                .bind(prepare.broker_credential_generation)
                .bind(prepare.backup_created_at)
                .bind(prepare.broker_fenced_at)
                .bind(prepare.snapshot_accepted_through)
                .bind(prepare.snapshot_allocation_high_water)
                .bind(prepare.snapshot_epoch_start_publication_seq)
                .bind(row.try_get::<i64, _>("accepted_through")?)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                insert_audit_postgres(
                    &mut tx,
                    &AuditActor::LocalCli,
                    now,
                    "edge_node_recovery_prepared",
                    &prepare.recovery_id,
                    serde_json::json!({
                        "edge_id": edge_id,
                        "edge_node_id": prepare.edge_node_id,
                        "backup_id": prepare.backup_id,
                        "old_ledger_epoch": prepare.old_ledger_epoch,
                        "new_ledger_epoch": prepare.new_ledger_epoch,
                        "broker_fence_id": prepare.broker_fence_id,
                        "broker_credential_generation": prepare.broker_credential_generation,
                        "backup_created_at": prepare.backup_created_at,
                        "broker_fenced_at": prepare.broker_fenced_at,
                        "snapshot_accepted_through": prepare.snapshot_accepted_through,
                        "edge_accepted_through": row.try_get::<i64, _>("accepted_through")?,
                    }),
                )
                .await?;
                let case = load_case_postgres(&mut tx, &prepare.recovery_id).await?;
                tx.commit().await?;
                Ok(case)
            }
        }
    }

    pub async fn recovery_case(&self, recovery_id: &str) -> Result<RecoveryCase, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let case = load_case_sqlite(&mut tx, recovery_id).await?;
                tx.commit().await?;
                Ok(case)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let case = load_case_postgres(&mut tx, recovery_id).await?;
                tx.commit().await?;
                Ok(case)
            }
        }
    }

    pub async fn recovery_activation_request(
        &self,
        recovery_id: &str,
    ) -> Result<RecoveryActivationRequest, StorageError> {
        let bytes: Vec<u8> = match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                sqlx::query_scalar(
                    "SELECT request_json FROM edge_node_recovery_cases
                 WHERE recovery_id=? AND request_json IS NOT NULL",
                )
                .bind(recovery_id)
                .fetch_one(pool)
                .await?
            }
            StorageInner::Postgres { pool, .. } => {
                sqlx::query_scalar(
                    "SELECT request_json FROM edge_node_recovery_cases
                 WHERE recovery_id=$1 AND request_json IS NOT NULL",
                )
                .bind(recovery_id)
                .fetch_one(pool)
                .await?
            }
        };
        RecoveryActivationRequest::decode(&bytes).map_err(|_| StorageError::RecoveryConflict)
    }

    pub async fn authorize_edge_node_recovery(
        &self,
        request: &RecoveryActivationRequest,
        now: i64,
    ) -> Result<RecoveryCommand, StorageError> {
        request
            .validate()
            .map_err(|_| StorageError::RecoveryConflict)?;
        if now < 0 {
            return Err(StorageError::RecoveryConflict);
        }
        let payload = serde_json::to_vec(request).map_err(|_| StorageError::RecoveryConflict)?;
        let topic = format!(
            "iotkit/v1/edge-nodes/{}/recovery/request",
            request.edge_node_id
        );
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                authorize_sqlite(&mut tx, request, &topic, &payload, now).await?;
                let command = load_command_sqlite(&mut tx, &request.recovery_id, "request").await?;
                tx.commit().await?;
                Ok(command)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                authorize_postgres(&mut tx, request, &topic, &payload, now).await?;
                let command =
                    load_command_postgres(&mut tx, &request.recovery_id, "request").await?;
                tx.commit().await?;
                Ok(command)
            }
        }
    }

    pub async fn apply_edge_node_recovery_result(
        &self,
        result: &RecoveryActivationResult,
        now: i64,
    ) -> Result<RecoveryCompletion, StorageError> {
        result
            .validate()
            .map_err(|_| StorageError::RecoveryConflict)?;
        if now < 0 {
            return Err(StorageError::RecoveryConflict);
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                match apply_result_sqlite(&mut tx, result, now).await {
                    Ok(completion) => {
                        tx.commit().await?;
                        Ok(completion)
                    }
                    Err(StorageError::RecoveryConflict) => {
                        if recovery_case_belongs_to_node_sqlite(
                            &mut tx,
                            &result.recovery_id,
                            &result.edge_node_id,
                        )
                        .await?
                        {
                            hold_recovery_sqlite(&mut tx, &result.recovery_id, now).await?;
                        }
                        tx.commit().await?;
                        Err(StorageError::RecoveryConflict)
                    }
                    Err(error) => Err(error),
                }
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                match apply_result_postgres(&mut tx, result, now).await {
                    Ok(completion) => {
                        tx.commit().await?;
                        Ok(completion)
                    }
                    Err(StorageError::RecoveryConflict) => {
                        if recovery_case_belongs_to_node_postgres(
                            &mut tx,
                            &result.recovery_id,
                            &result.edge_node_id,
                        )
                        .await?
                        {
                            hold_recovery_postgres(&mut tx, &result.recovery_id, now).await?;
                        }
                        tx.commit().await?;
                        Err(StorageError::RecoveryConflict)
                    }
                    Err(error) => Err(error),
                }
            }
        }
    }

    pub async fn acknowledge_edge_node_recovery_completion(
        &self,
        acknowledgement: &RecoveryCompletionAck,
        now: i64,
    ) -> Result<(), StorageError> {
        acknowledgement
            .validate()
            .map_err(|_| StorageError::RecoveryConflict)?;
        if now < 0 {
            return Err(StorageError::RecoveryConflict);
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let case = load_case_sqlite(&mut tx, &acknowledgement.recovery_id)
                    .await
                    .map_err(normalize_missing_recovery)?;
                let edge_id: String =
                    sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
                        .fetch_one(&mut *tx)
                        .await?;
                if validate_completion_ack(&case, acknowledgement, &edge_id).is_err() {
                    if case.edge_node_id == acknowledgement.edge_node_id {
                        hold_recovery_sqlite(&mut tx, &case.recovery_id, now).await?;
                    }
                    tx.commit().await?;
                    return Err(StorageError::RecoveryConflict);
                }
                let changed = sqlx::query(
                    "UPDATE recovery_command_outbox SET completed_at=coalesce(completed_at,?)
                     WHERE recovery_id=? AND kind='completion'",
                )
                .bind(now)
                .bind(&acknowledgement.recovery_id)
                .execute(&mut *tx)
                .await?;
                if changed.rows_affected() != 1 {
                    return Err(StorageError::RecoveryConflict);
                }
                tx.commit().await?;
                Ok(())
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let case = load_case_postgres(&mut tx, &acknowledgement.recovery_id)
                    .await
                    .map_err(normalize_missing_recovery)?;
                let edge_id: String =
                    sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
                        .fetch_one(&mut *tx)
                        .await?;
                if validate_completion_ack(&case, acknowledgement, &edge_id).is_err() {
                    if case.edge_node_id == acknowledgement.edge_node_id {
                        hold_recovery_postgres(&mut tx, &case.recovery_id, now).await?;
                    }
                    tx.commit().await?;
                    return Err(StorageError::RecoveryConflict);
                }
                let changed = sqlx::query(
                    "UPDATE recovery_command_outbox SET completed_at=coalesce(completed_at,$1)
                     WHERE recovery_id=$2 AND kind='completion'",
                )
                .bind(now)
                .bind(&acknowledgement.recovery_id)
                .execute(&mut *tx)
                .await?;
                if changed.rows_affected() != 1 {
                    return Err(StorageError::RecoveryConflict);
                }
                tx.commit().await?;
                Ok(())
            }
        }
    }

    pub async fn recovery_completion_acknowledged(
        &self,
        recovery_id: &str,
    ) -> Result<bool, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let row: (Option<i64>, String) = sqlx::query_as(
                    "SELECT outbox.completed_at,recovery.state
                     FROM recovery_command_outbox outbox
                     JOIN edge_node_recovery_cases recovery
                       ON recovery.recovery_id=outbox.recovery_id
                     WHERE outbox.recovery_id=? AND outbox.kind='completion'",
                )
                .bind(recovery_id)
                .fetch_one(pool)
                .await
                .map_err(|error| normalize_missing_recovery(StorageError::Database(error)))?;
                Ok(row.1 == "completed" && row.0.is_some())
            }
            StorageInner::Postgres { pool, .. } => {
                let row: (Option<i64>, String) = sqlx::query_as(
                    "SELECT outbox.completed_at,recovery.state
                     FROM recovery_command_outbox outbox
                     JOIN edge_node_recovery_cases recovery
                       ON recovery.recovery_id=outbox.recovery_id
                     WHERE outbox.recovery_id=$1 AND outbox.kind='completion'",
                )
                .bind(recovery_id)
                .fetch_one(pool)
                .await
                .map_err(|error| normalize_missing_recovery(StorageError::Database(error)))?;
                Ok(row.1 == "completed" && row.0.is_some())
            }
        }
    }

    pub async fn pending_recovery_commands(
        &self,
        limit: i64,
    ) -> Result<Vec<RecoveryCommand>, StorageError> {
        if !(1..=1000).contains(&limit) {
            return Err(StorageError::RecoveryConflict);
        }
        let query = "SELECT recovery_id,kind,topic,payload_json,attempts,last_attempt_at
             FROM recovery_command_outbox WHERE completed_at IS NULL
             ORDER BY created_at,recovery_id,kind LIMIT ";
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let rows = sqlx::query(&format!("{query}?"))
                    .bind(limit)
                    .fetch_all(pool)
                    .await?;
                rows.into_iter().map(row_to_command).collect()
            }
            StorageInner::Postgres { pool, .. } => {
                let rows = sqlx::query(&format!("{query}$1"))
                    .bind(limit)
                    .fetch_all(pool)
                    .await?;
                rows.into_iter().map(row_to_command).collect()
            }
        }
    }

    pub async fn pending_recovery_commands_due(
        &self,
        limit: i64,
        now: i64,
    ) -> Result<Vec<RecoveryCommand>, StorageError> {
        if !(1..=1000).contains(&limit) || now < 0 {
            return Err(StorageError::RecoveryConflict);
        }
        let retry_before = now.saturating_sub(5_000);
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let rows = sqlx::query(
                    "SELECT recovery_id,kind,topic,payload_json,attempts,last_attempt_at
                     FROM recovery_command_outbox
                     WHERE completed_at IS NULL
                       AND (last_attempt_at IS NULL OR last_attempt_at<=?)
                     ORDER BY created_at,recovery_id,kind LIMIT ?",
                )
                .bind(retry_before)
                .bind(limit)
                .fetch_all(pool)
                .await?;
                rows.into_iter().map(row_to_command).collect()
            }
            StorageInner::Postgres { pool, .. } => {
                let rows = sqlx::query(
                    "SELECT recovery_id,kind,topic,payload_json,attempts,last_attempt_at
                     FROM recovery_command_outbox
                     WHERE completed_at IS NULL
                       AND (last_attempt_at IS NULL OR last_attempt_at<=$1)
                     ORDER BY created_at,recovery_id,kind LIMIT $2",
                )
                .bind(retry_before)
                .bind(limit)
                .fetch_all(pool)
                .await?;
                rows.into_iter().map(row_to_command).collect()
            }
        }
    }

    pub async fn mark_recovery_attempt(
        &self,
        recovery_id: &str,
        kind: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        if now < 0 || !matches!(kind, "request" | "completion") {
            return Err(StorageError::RecoveryConflict);
        }
        let affected = match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => sqlx::query(
                "UPDATE recovery_command_outbox
                 SET attempts=attempts+1,last_attempt_at=?
                 WHERE recovery_id=? AND kind=? AND completed_at IS NULL",
            )
            .bind(now)
            .bind(recovery_id)
            .bind(kind)
            .execute(pool)
            .await?
            .rows_affected(),
            StorageInner::Postgres { pool, .. } => sqlx::query(
                "UPDATE recovery_command_outbox
                 SET attempts=attempts+1,last_attempt_at=$1
                 WHERE recovery_id=$2 AND kind=$3 AND completed_at IS NULL",
            )
            .bind(now)
            .bind(recovery_id)
            .bind(kind)
            .execute(pool)
            .await?
            .rows_affected(),
        };
        if affected != 1 {
            return Err(StorageError::RecoveryConflict);
        }
        Ok(())
    }
}

fn validate_prepare(prepare: &RecoveryPrepare, now: i64) -> Result<(), StorageError> {
    if now < 0
        || prepare.recovery_id.is_empty()
        || prepare.edge_node_id.is_empty()
        || prepare.backup_id.is_empty()
        || prepare.old_ledger_epoch.is_empty()
        || prepare.new_ledger_epoch.is_empty()
        || prepare.old_ledger_epoch == prepare.new_ledger_epoch
        || prepare.broker_fence_id.is_empty()
        || prepare.broker_credential_generation <= 0
        || prepare.backup_created_at < 0
        || prepare.broker_fenced_at > now
        || prepare.snapshot_accepted_through < 0
        || prepare.snapshot_allocation_high_water < prepare.snapshot_accepted_through
        || prepare
            .snapshot_epoch_start_publication_seq
            .is_some_and(|sequence| {
                sequence < 1 || sequence > prepare.snapshot_allocation_high_water
            })
    {
        return Err(StorageError::RecoveryConflict);
    }
    Ok(())
}

fn prepare_matches(case: &RecoveryCase, prepare: &RecoveryPrepare) -> bool {
    case.state == "prepared"
        && case.recovery_id == prepare.recovery_id
        && case.edge_node_id == prepare.edge_node_id
        && case.backup_id == prepare.backup_id
        && case.old_ledger_epoch == prepare.old_ledger_epoch
        && case.new_ledger_epoch == prepare.new_ledger_epoch
        && case.broker_fence_id == prepare.broker_fence_id
        && case.broker_credential_generation == prepare.broker_credential_generation
        && case.backup_created_at == prepare.backup_created_at
        && case.broker_fenced_at == prepare.broker_fenced_at
        && case.snapshot_accepted_through == prepare.snapshot_accepted_through
        && case.snapshot_allocation_high_water == prepare.snapshot_allocation_high_water
        && case.snapshot_epoch_start_publication_seq == prepare.snapshot_epoch_start_publication_seq
}

fn validate_active_boundary(
    state: String,
    epoch: String,
    edge_accepted: i64,
    prepare: &RecoveryPrepare,
) -> Result<(), StorageError> {
    if state != "active"
        || epoch != prepare.old_ledger_epoch
        || edge_accepted < prepare.snapshot_accepted_through
    {
        return Err(StorageError::RecoveryConflict);
    }
    Ok(())
}

fn row_to_case<R>(row: R) -> Result<RecoveryCase, StorageError>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    for<'a> String: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> Option<String>: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> i64: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> Option<i64>: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
{
    Ok(RecoveryCase {
        recovery_id: row.try_get("recovery_id")?,
        state: row.try_get("state")?,
        edge_node_id: row.try_get("edge_node_id")?,
        backup_id: row.try_get("backup_id")?,
        old_ledger_epoch: row.try_get("old_ledger_epoch")?,
        new_ledger_epoch: row.try_get("new_ledger_epoch")?,
        broker_fence_id: row.try_get("broker_fence_id")?,
        broker_credential_generation: row.try_get("broker_credential_generation")?,
        backup_created_at: row.try_get("backup_created_at")?,
        broker_fenced_at: row.try_get("broker_fenced_at")?,
        device_auth_generation: row.try_get("device_auth_generation")?,
        candidate_instance_id: row.try_get("candidate_instance_id")?,
        snapshot_accepted_through: row.try_get("snapshot_accepted_through")?,
        snapshot_allocation_high_water: row.try_get("snapshot_allocation_high_water")?,
        snapshot_epoch_start_publication_seq: row
            .try_get("snapshot_epoch_start_publication_seq")?,
        edge_accepted_through: row.try_get("edge_accepted_through")?,
        replayed_records: row.try_get("replayed_records")?,
        last_new_publication_seq: row.try_get("last_new_publication_seq")?,
    })
}

fn row_to_command<R>(row: R) -> Result<RecoveryCommand, StorageError>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    for<'a> String: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> Vec<u8>: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> i64: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> Option<i64>: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
{
    Ok(RecoveryCommand {
        recovery_id: row.try_get("recovery_id")?,
        kind: row.try_get("kind")?,
        topic: row.try_get("topic")?,
        payload_json: row.try_get("payload_json")?,
        attempts: row.try_get("attempts")?,
        last_attempt_at: row.try_get("last_attempt_at")?,
    })
}

macro_rules! recovery_select {
    () => {
        "SELECT recovery_id,state,edge_node_id,backup_id,old_ledger_epoch,
                new_ledger_epoch,broker_fence_id,broker_credential_generation,
                backup_created_at,broker_fenced_at,device_auth_generation,
                candidate_instance_id,snapshot_accepted_through,
                snapshot_allocation_high_water,edge_accepted_through,
                snapshot_epoch_start_publication_seq,
                replayed_records,last_new_publication_seq
         FROM edge_node_recovery_cases WHERE recovery_id="
    };
}

async fn load_case_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    recovery_id: &str,
) -> Result<RecoveryCase, StorageError> {
    row_to_case(
        sqlx::query(&format!("{}?", recovery_select!()))
            .bind(recovery_id)
            .fetch_one(&mut **tx)
            .await?,
    )
}

async fn load_case_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    recovery_id: &str,
) -> Result<RecoveryCase, StorageError> {
    row_to_case(
        sqlx::query(&format!("{}$1 FOR UPDATE", recovery_select!()))
            .bind(recovery_id)
            .fetch_one(&mut **tx)
            .await?,
    )
}

async fn load_command_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    recovery_id: &str,
    kind: &str,
) -> Result<RecoveryCommand, StorageError> {
    row_to_command(
        sqlx::query(
            "SELECT recovery_id,kind,topic,payload_json,attempts,last_attempt_at
             FROM recovery_command_outbox WHERE recovery_id=? AND kind=?",
        )
        .bind(recovery_id)
        .bind(kind)
        .fetch_one(&mut **tx)
        .await?,
    )
}

async fn load_command_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    recovery_id: &str,
    kind: &str,
) -> Result<RecoveryCommand, StorageError> {
    row_to_command(
        sqlx::query(
            "SELECT recovery_id,kind,topic,payload_json,attempts,last_attempt_at
             FROM recovery_command_outbox WHERE recovery_id=$1 AND kind=$2",
        )
        .bind(recovery_id)
        .bind(kind)
        .fetch_one(&mut **tx)
        .await?,
    )
}

fn request_matches(case: &RecoveryCase, request: &RecoveryActivationRequest) -> bool {
    case.state == "prepared"
        && case.recovery_id == request.recovery_id
        && case.edge_node_id == request.edge_node_id
        && case.backup_id == request.backup_id
        && case.old_ledger_epoch == request.old_ledger_epoch
        && case.new_ledger_epoch == request.new_ledger_epoch
        && case.broker_credential_generation == request.broker_credential_generation
        && case.snapshot_accepted_through == request.snapshot_accepted_through
        && case.snapshot_allocation_high_water == request.snapshot_allocation_high_water
        && case.snapshot_epoch_start_publication_seq == request.snapshot_epoch_start_publication_seq
        && case.edge_accepted_through == request.edge_accepted_through
}

async fn authorize_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    request: &RecoveryActivationRequest,
    topic: &str,
    payload: &[u8],
    now: i64,
) -> Result<(), StorageError> {
    let edge_id: String = sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
        .fetch_one(&mut **tx)
        .await?;
    if request.edge_id != edge_id {
        return Err(StorageError::RecoveryConflict);
    }
    let case = load_case_sqlite(tx, &request.recovery_id).await?;
    if case.state == "authorized" {
        let stored: Vec<u8> = sqlx::query_scalar(
            "SELECT request_json FROM edge_node_recovery_cases WHERE recovery_id=?",
        )
        .bind(&request.recovery_id)
        .fetch_one(&mut **tx)
        .await?;
        return if stored == payload {
            Ok(())
        } else {
            Err(StorageError::RecoveryConflict)
        };
    }
    if !request_matches(&case, request) {
        return Err(StorageError::RecoveryConflict);
    }
    let changed = sqlx::query(
        "UPDATE edge_node_recovery_cases SET state='authorized',
         device_auth_generation=?,candidate_instance_id=?,request_json=?,updated_at=?
         WHERE recovery_id=? AND state='prepared'",
    )
    .bind(request.device_auth_generation)
    .bind(&request.candidate_instance_id)
    .bind(payload)
    .bind(now)
    .bind(&request.recovery_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(StorageError::RecoveryConflict);
    }
    sqlx::query(
        "INSERT INTO recovery_command_outbox(
             recovery_id,kind,topic,payload_json,created_at
         ) VALUES(?,'request',?,?,?)",
    )
    .bind(&request.recovery_id)
    .bind(topic)
    .bind(payload)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn authorize_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request: &RecoveryActivationRequest,
    topic: &str,
    payload: &[u8],
    now: i64,
) -> Result<(), StorageError> {
    let edge_id: String = sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
        .fetch_one(&mut **tx)
        .await?;
    if request.edge_id != edge_id {
        return Err(StorageError::RecoveryConflict);
    }
    let case = load_case_postgres(tx, &request.recovery_id).await?;
    if case.state == "authorized" {
        let stored: Vec<u8> = sqlx::query_scalar(
            "SELECT request_json FROM edge_node_recovery_cases WHERE recovery_id=$1",
        )
        .bind(&request.recovery_id)
        .fetch_one(&mut **tx)
        .await?;
        return if stored == payload {
            Ok(())
        } else {
            Err(StorageError::RecoveryConflict)
        };
    }
    if !request_matches(&case, request) {
        return Err(StorageError::RecoveryConflict);
    }
    let changed = sqlx::query(
        "UPDATE edge_node_recovery_cases SET state='authorized',
         device_auth_generation=$1,candidate_instance_id=$2,request_json=$3,updated_at=$4
         WHERE recovery_id=$5 AND state='prepared'",
    )
    .bind(request.device_auth_generation)
    .bind(&request.candidate_instance_id)
    .bind(payload)
    .bind(now)
    .bind(&request.recovery_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(StorageError::RecoveryConflict);
    }
    sqlx::query(
        "INSERT INTO recovery_command_outbox(
             recovery_id,kind,topic,payload_json,created_at
         ) VALUES($1,'request',$2,$3,$4)",
    )
    .bind(&request.recovery_id)
    .bind(topic)
    .bind(payload)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn apply_result_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    result: &RecoveryActivationResult,
    now: i64,
) -> Result<RecoveryCompletion, StorageError> {
    let case = match load_case_sqlite(tx, &result.recovery_id).await {
        Err(StorageError::Database(sqlx::Error::RowNotFound)) => {
            return Err(StorageError::RecoveryConflict);
        }
        other => other?,
    };
    let edge_id: String = sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
        .fetch_one(&mut **tx)
        .await?;
    apply_result_common(&case, result, &edge_id)?;
    if case.state == "completed" {
        let stored: Vec<u8> = sqlx::query_scalar(
            "SELECT result_json FROM edge_node_recovery_cases WHERE recovery_id=?",
        )
        .bind(&result.recovery_id)
        .fetch_one(&mut **tx)
        .await?;
        if stored != serde_json::to_vec(result).map_err(|_| StorageError::RecoveryConflict)? {
            return Err(StorageError::RecoveryConflict);
        }
        return stored_completion_sqlite(tx, &result.recovery_id).await;
    }
    let completion = commit_result_sqlite(tx, &case, result, now).await?;
    Ok(completion)
}

async fn apply_result_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    result: &RecoveryActivationResult,
    now: i64,
) -> Result<RecoveryCompletion, StorageError> {
    let case = match load_case_postgres(tx, &result.recovery_id).await {
        Err(StorageError::Database(sqlx::Error::RowNotFound)) => {
            return Err(StorageError::RecoveryConflict);
        }
        other => other?,
    };
    let edge_id: String = sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
        .fetch_one(&mut **tx)
        .await?;
    apply_result_common(&case, result, &edge_id)?;
    if case.state == "completed" {
        let stored: Vec<u8> = sqlx::query_scalar(
            "SELECT result_json FROM edge_node_recovery_cases WHERE recovery_id=$1",
        )
        .bind(&result.recovery_id)
        .fetch_one(&mut **tx)
        .await?;
        if stored != serde_json::to_vec(result).map_err(|_| StorageError::RecoveryConflict)? {
            return Err(StorageError::RecoveryConflict);
        }
        return stored_completion_postgres(tx, &result.recovery_id).await;
    }
    commit_result_postgres(tx, &case, result, now).await
}

fn apply_result_common(
    case: &RecoveryCase,
    result: &RecoveryActivationResult,
    edge_id: &str,
) -> Result<(), StorageError> {
    let unaccepted_epoch_start = case
        .snapshot_epoch_start_publication_seq
        .is_some_and(|sequence| sequence > case.edge_accepted_through);
    let expected_replayed_records = case
        .snapshot_allocation_high_water
        .saturating_sub(case.edge_accepted_through)
        .saturating_sub(i64::from(unaccepted_epoch_start));
    let expected_last_new_publication_seq = expected_replayed_records
        .checked_add(1)
        .ok_or(StorageError::RecoveryConflict)?;
    if !matches!(case.state.as_str(), "authorized" | "completed")
        || case.recovery_id != result.recovery_id
        || edge_id != result.edge_id
        || case.edge_node_id != result.edge_node_id
        || case.backup_id != result.backup_id
        || case.old_ledger_epoch != result.old_ledger_epoch
        || case.new_ledger_epoch != result.new_ledger_epoch
        || case.broker_credential_generation != result.broker_credential_generation
        || case.device_auth_generation != Some(result.device_auth_generation)
        || case.candidate_instance_id.as_deref() != Some(&result.candidate_instance_id)
        || case.edge_accepted_through != result.edge_accepted_through
        || result.replayed_records != expected_replayed_records
        || result.last_new_publication_seq != expected_last_new_publication_seq
    {
        return Err(StorageError::RecoveryConflict);
    }
    Ok(())
}

fn normalize_missing_recovery(error: StorageError) -> StorageError {
    match error {
        StorageError::Database(sqlx::Error::RowNotFound) => StorageError::RecoveryConflict,
        other => other,
    }
}

fn validate_completion_ack(
    case: &RecoveryCase,
    acknowledgement: &RecoveryCompletionAck,
    edge_id: &str,
) -> Result<(), StorageError> {
    if case.state != "completed"
        || case.recovery_id != acknowledgement.recovery_id
        || case.edge_node_id != acknowledgement.edge_node_id
        || case.new_ledger_epoch != acknowledgement.new_ledger_epoch
        || edge_id != acknowledgement.edge_id
        || case.candidate_instance_id.as_deref()
            != Some(acknowledgement.candidate_instance_id.as_str())
    {
        return Err(StorageError::RecoveryConflict);
    }
    Ok(())
}

fn completion(case: &RecoveryCase, now: i64) -> RecoveryCompletion {
    RecoveryCompletion {
        schema_version: SCHEMA_VERSION,
        recovery_id: case.recovery_id.clone(),
        edge_id: String::new(),
        edge_node_id: case.edge_node_id.clone(),
        candidate_instance_id: case.candidate_instance_id.clone().unwrap_or_default(),
        new_ledger_epoch: case.new_ledger_epoch.clone(),
        status: "committed".into(),
        accepted_through: 0,
        committed_at: now,
    }
}

async fn commit_result_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    case: &RecoveryCase,
    result: &RecoveryActivationResult,
    now: i64,
) -> Result<RecoveryCompletion, StorageError> {
    let edge_id: String = sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
        .fetch_one(&mut **tx)
        .await?;
    let mut complete = completion(case, now);
    complete.edge_id = edge_id;
    complete
        .validate()
        .map_err(|_| StorageError::RecoveryConflict)?;
    let result_json = serde_json::to_vec(result).map_err(|_| StorageError::RecoveryConflict)?;
    let completion_json =
        serde_json::to_vec(&complete).map_err(|_| StorageError::RecoveryConflict)?;
    let changed = sqlx::query(
        "UPDATE edge_node_activations SET ledger_epoch=?,state='active',
         activation_id=NULL,revision=revision+1,updated_at=?
         WHERE edge_node_id=? AND ledger_epoch=? AND state='recovery_hold'",
    )
    .bind(&case.new_ledger_epoch)
    .bind(now)
    .bind(&case.edge_node_id)
    .bind(&case.old_ledger_epoch)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(StorageError::RecoveryConflict);
    }
    sqlx::query(
        "INSERT INTO accepted_cursors(edge_node_id,ledger_epoch,accepted_through,updated_at)
         VALUES(?,?,0,?)",
    )
    .bind(&case.edge_node_id)
    .bind(&case.new_ledger_epoch)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE edge_node_recovery_cases SET state='completed',result_json=?,
         completion_json=?,replayed_records=?,last_new_publication_seq=?,
         updated_at=?,completed_at=? WHERE recovery_id=? AND state='authorized'",
    )
    .bind(&result_json)
    .bind(&completion_json)
    .bind(result.replayed_records)
    .bind(result.last_new_publication_seq)
    .bind(now)
    .bind(now)
    .bind(&case.recovery_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE recovery_command_outbox SET completed_at=?
         WHERE recovery_id=? AND kind='request'",
    )
    .bind(now)
    .bind(&case.recovery_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO recovery_command_outbox(
             recovery_id,kind,topic,payload_json,created_at
         ) VALUES(?,'completion',?,?,?)",
    )
    .bind(&case.recovery_id)
    .bind(format!(
        "iotkit/v1/edge-nodes/{}/recovery/completion",
        case.edge_node_id
    ))
    .bind(&completion_json)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(complete)
}

async fn commit_result_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    case: &RecoveryCase,
    result: &RecoveryActivationResult,
    now: i64,
) -> Result<RecoveryCompletion, StorageError> {
    let edge_id: String = sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
        .fetch_one(&mut **tx)
        .await?;
    let mut complete = completion(case, now);
    complete.edge_id = edge_id;
    complete
        .validate()
        .map_err(|_| StorageError::RecoveryConflict)?;
    let result_json = serde_json::to_vec(result).map_err(|_| StorageError::RecoveryConflict)?;
    let completion_json =
        serde_json::to_vec(&complete).map_err(|_| StorageError::RecoveryConflict)?;
    let changed = sqlx::query(
        "UPDATE edge_node_activations SET ledger_epoch=$1,state='active',
         activation_id=NULL,revision=revision+1,updated_at=$2
         WHERE edge_node_id=$3 AND ledger_epoch=$4 AND state='recovery_hold'",
    )
    .bind(&case.new_ledger_epoch)
    .bind(now)
    .bind(&case.edge_node_id)
    .bind(&case.old_ledger_epoch)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(StorageError::RecoveryConflict);
    }
    sqlx::query(
        "INSERT INTO accepted_cursors(edge_node_id,ledger_epoch,accepted_through,updated_at)
         VALUES($1,$2,0,$3)",
    )
    .bind(&case.edge_node_id)
    .bind(&case.new_ledger_epoch)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE edge_node_recovery_cases SET state='completed',result_json=$1,
         completion_json=$2,replayed_records=$3,last_new_publication_seq=$4,
         updated_at=$5,completed_at=$5
         WHERE recovery_id=$6 AND state='authorized'",
    )
    .bind(&result_json)
    .bind(&completion_json)
    .bind(result.replayed_records)
    .bind(result.last_new_publication_seq)
    .bind(now)
    .bind(&case.recovery_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE recovery_command_outbox SET completed_at=$1
         WHERE recovery_id=$2 AND kind='request'",
    )
    .bind(now)
    .bind(&case.recovery_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO recovery_command_outbox(
             recovery_id,kind,topic,payload_json,created_at
         ) VALUES($1,'completion',$2,$3,$4)",
    )
    .bind(&case.recovery_id)
    .bind(format!(
        "iotkit/v1/edge-nodes/{}/recovery/completion",
        case.edge_node_id
    ))
    .bind(&completion_json)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(complete)
}

async fn stored_completion_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    recovery_id: &str,
) -> Result<RecoveryCompletion, StorageError> {
    let bytes: Vec<u8> = sqlx::query_scalar(
        "SELECT completion_json FROM edge_node_recovery_cases WHERE recovery_id=?",
    )
    .bind(recovery_id)
    .fetch_one(&mut **tx)
    .await?;
    RecoveryCompletion::decode(&bytes).map_err(|_| StorageError::RecoveryConflict)
}

async fn stored_completion_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    recovery_id: &str,
) -> Result<RecoveryCompletion, StorageError> {
    let bytes: Vec<u8> = sqlx::query_scalar(
        "SELECT completion_json FROM edge_node_recovery_cases WHERE recovery_id=$1",
    )
    .bind(recovery_id)
    .fetch_one(&mut **tx)
    .await?;
    RecoveryCompletion::decode(&bytes).map_err(|_| StorageError::RecoveryConflict)
}

async fn recovery_case_belongs_to_node_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    recovery_id: &str,
    edge_node_id: &str,
) -> Result<bool, StorageError> {
    let stored: Option<String> =
        sqlx::query_scalar("SELECT edge_node_id FROM edge_node_recovery_cases WHERE recovery_id=?")
            .bind(recovery_id)
            .fetch_optional(&mut **tx)
            .await?;
    Ok(stored.as_deref() == Some(edge_node_id))
}

async fn recovery_case_belongs_to_node_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    recovery_id: &str,
    edge_node_id: &str,
) -> Result<bool, StorageError> {
    let stored: Option<String> = sqlx::query_scalar(
        "SELECT edge_node_id FROM edge_node_recovery_cases WHERE recovery_id=$1",
    )
    .bind(recovery_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(stored.as_deref() == Some(edge_node_id))
}

async fn hold_recovery_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    recovery_id: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE edge_node_recovery_cases SET state='recovery_hold',updated_at=?
         WHERE recovery_id=? AND state IN ('prepared','authorized','completed')",
    )
    .bind(now)
    .bind(recovery_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE recovery_command_outbox SET completed_at=?
         WHERE recovery_id=? AND completed_at IS NULL",
    )
    .bind(now)
    .bind(recovery_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE edge_node_activations SET state='recovery_hold',updated_at=?
         WHERE edge_node_id=(
             SELECT edge_node_id FROM edge_node_recovery_cases WHERE recovery_id=?
         )
         AND ledger_epoch IN (
             SELECT old_ledger_epoch FROM edge_node_recovery_cases WHERE recovery_id=?
             UNION
             SELECT new_ledger_epoch FROM edge_node_recovery_cases WHERE recovery_id=?
         )",
    )
    .bind(now)
    .bind(recovery_id)
    .bind(recovery_id)
    .bind(recovery_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn hold_recovery_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    recovery_id: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE edge_node_recovery_cases SET state='recovery_hold',updated_at=$1
         WHERE recovery_id=$2 AND state IN ('prepared','authorized','completed')",
    )
    .bind(now)
    .bind(recovery_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE recovery_command_outbox SET completed_at=$1
         WHERE recovery_id=$2 AND completed_at IS NULL",
    )
    .bind(now)
    .bind(recovery_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE edge_node_activations SET state='recovery_hold',updated_at=$1
         WHERE edge_node_id=(
             SELECT edge_node_id FROM edge_node_recovery_cases WHERE recovery_id=$2
         )
         AND ledger_epoch IN (
             SELECT old_ledger_epoch FROM edge_node_recovery_cases WHERE recovery_id=$2
             UNION
             SELECT new_ledger_epoch FROM edge_node_recovery_cases WHERE recovery_id=$2
         )",
    )
    .bind(now)
    .bind(recovery_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
