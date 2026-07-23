use iotkit_output_adapter_api::{
    AdapterError, IdentityScope, Observation, ObservationKind, ObservationValue, ProfileRequest,
};
use serde::Deserialize;
use serde_json::{Map, Value, value::RawValue};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::{
    application::{
        output_profiles::{ExportProfile, OutputBinding, ProfileState},
        semantics::{SemanticObservation, SemanticRule, SemanticRuleDraft},
    },
    composition::{OutputAdapterRegistration, registered_output_adapters},
    semantics::{EvaluationState, RuleSpec, SemanticKind, evaluate_rule},
};

use super::{Storage, StorageError, StorageInner};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedOutput {
    pub export_id: String,
    pub route_id: String,
    pub topic: String,
    pub qos: u8,
    pub retain: bool,
    pub payload: Vec<u8>,
    pub attempts: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMark {
    Published,
    ClaimLost,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProjectedOne {
    pub receipt: bool,
    pub observation: bool,
    pub publications: usize,
}

#[derive(Debug, Deserialize)]
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
    device_time: Option<i64>,
}

impl Storage {
    pub async fn create_semantic_rule(
        &self,
        draft: SemanticRuleDraft,
        now: i64,
    ) -> Result<SemanticRule, StorageError> {
        validate_rule_draft(&draft, now)?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let rule = create_rule_sqlite(&mut tx, draft, now).await?;
                tx.commit().await?;
                Ok(rule)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let rule = create_rule_postgres(&mut tx, draft, now).await?;
                tx.commit().await?;
                Ok(rule)
            }
        }
    }

    pub async fn revise_semantic_rule(
        &self,
        rule_id: &str,
        display_name: &str,
        spec: RuleSpec,
        now: i64,
    ) -> Result<SemanticRule, StorageError> {
        if rule_id.is_empty() || display_name.is_empty() || now < 0 {
            return Err(StorageError::InvalidSemantic(
                "rule, display name, and timestamp are required".into(),
            ));
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let rule = revise_rule_sqlite(&mut tx, rule_id, display_name, spec, now).await?;
                tx.commit().await?;
                Ok(rule)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let rule =
                    revise_rule_postgres(&mut tx, rule_id, display_name, spec, now).await?;
                tx.commit().await?;
                Ok(rule)
            }
        }
    }

    pub async fn update_semantic_calibration(
        &self,
        signal_ref: &str,
        scale: f64,
        offset: f64,
        now: i64,
    ) -> Result<i64, StorageError> {
        if signal_ref.is_empty() || now < 0 {
            return Err(StorageError::InvalidSemantic(
                "signal and timestamp are required".into(),
            ));
        }
        let revision = match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let revision: Option<i64> = sqlx::query_scalar(
                    "UPDATE semantic_signals SET calibration_revision=calibration_revision+1, \
                     scale=?, calibration_offset=? WHERE signal_ref=? RETURNING calibration_revision",
                )
                .bind(scale)
                .bind(offset)
                .bind(signal_ref)
                .fetch_optional(&mut *tx)
                .await?;
                let revision = revision.ok_or(StorageError::SemanticNotFound)?;
                sqlx::query(
                    "INSERT INTO semantic_calibration_revisions(\
                     signal_ref,revision,scale,calibration_offset,created_at) VALUES(?,?,?,?,?)",
                )
                .bind(signal_ref)
                .bind(revision)
                .bind(scale)
                .bind(offset)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "INSERT INTO semantic_calibration_starts(\
                     signal_ref,revision,ledger_epoch,start_after_pub_seq) \
                     SELECT ?,?,cursor.ledger_epoch,cursor.accepted_through \
                     FROM semantic_signals AS signal JOIN accepted_cursors AS cursor \
                     ON cursor.edge_node_id=signal.edge_node_id WHERE signal.signal_ref=?",
                )
                .bind(signal_ref)
                .bind(revision)
                .bind(signal_ref)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                Some(revision)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let edge_node_id: Option<String> = sqlx::query_scalar(
                    "SELECT edge_node_id FROM semantic_signals WHERE signal_ref=$1",
                )
                .bind(signal_ref)
                .fetch_optional(&mut *tx)
                .await?;
                let edge_node_id = edge_node_id.ok_or(StorageError::SemanticNotFound)?;
                lock_edge_cursors_postgres(&mut tx, &edge_node_id).await?;
                lock_signal_runtimes_postgres(&mut tx, signal_ref).await?;
                let revision: Option<i64> = sqlx::query_scalar(
                    "UPDATE semantic_signals SET calibration_revision=calibration_revision+1, \
                     scale=$1, calibration_offset=$2 WHERE signal_ref=$3 RETURNING calibration_revision",
                )
                .bind(scale)
                .bind(offset)
                .bind(signal_ref)
                .fetch_optional(&mut *tx)
                .await?;
                let revision = revision.ok_or(StorageError::SemanticNotFound)?;
                sqlx::query(
                    "INSERT INTO semantic_calibration_revisions(\
                     signal_ref,revision,scale,calibration_offset,created_at) VALUES($1,$2,$3,$4,$5)",
                )
                .bind(signal_ref)
                .bind(revision)
                .bind(scale)
                .bind(offset)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "INSERT INTO semantic_calibration_starts(\
                     signal_ref,revision,ledger_epoch,start_after_pub_seq) \
                     SELECT $1,$2,cursor.ledger_epoch,cursor.accepted_through \
                     FROM semantic_signals AS signal JOIN accepted_cursors AS cursor \
                     ON cursor.edge_node_id=signal.edge_node_id WHERE signal.signal_ref=$3",
                )
                .bind(signal_ref)
                .bind(revision)
                .bind(signal_ref)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                Some(revision)
            }
        };
        revision.ok_or(StorageError::SemanticNotFound)
    }

    pub async fn retire_semantic_rule(
        &self,
        rule_id: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        if rule_id.is_empty() || now < 0 {
            return Err(StorageError::InvalidSemantic(
                "rule and timestamp are required".into(),
            ));
        }
        let affected = match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                sqlx::query(
                    "INSERT OR IGNORE INTO semantic_rule_ends(rule_id,ledger_epoch,end_at_pub_seq) \
                     SELECT ?,cursor.ledger_epoch,cursor.accepted_through \
                     FROM semantic_rules AS rule JOIN semantic_signals AS signal \
                     ON signal.signal_ref=rule.signal_ref JOIN accepted_cursors AS cursor \
                     ON cursor.edge_node_id=signal.edge_node_id WHERE rule.rule_id=?",
                )
                .bind(rule_id)
                .bind(rule_id)
                .execute(&mut *tx)
                .await?;
                let result = sqlx::query(
                    "UPDATE semantic_rules SET active=0,retired_at=? \
                     WHERE rule_id=? AND active=1",
                )
                .bind(now)
                .bind(rule_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE output_bindings SET state='draining',revision=revision+1 \
                     WHERE rule_id=? AND state='active'",
                )
                .bind(rule_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE output_routes SET lifecycle_state='draining' \
                     WHERE rule_id=? AND active=1",
                )
                .bind(rule_id)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                result.rows_affected()
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let edge_node_id: Option<String> = sqlx::query_scalar(
                    "SELECT signal.edge_node_id FROM semantic_rules AS rule \
                     JOIN semantic_signals AS signal ON signal.signal_ref=rule.signal_ref \
                     WHERE rule.rule_id=$1 AND rule.active=TRUE",
                )
                .bind(rule_id)
                .fetch_optional(&mut *tx)
                .await?;
                let edge_node_id = edge_node_id.ok_or(StorageError::SemanticNotFound)?;
                lock_edge_cursors_postgres(&mut tx, &edge_node_id).await?;
                lock_rule_runtime_postgres(&mut tx, rule_id).await?;
                sqlx::query(
                    "INSERT INTO semantic_rule_ends(rule_id,ledger_epoch,end_at_pub_seq) \
                     SELECT $1,cursor.ledger_epoch,cursor.accepted_through \
                     FROM semantic_rules AS rule JOIN semantic_signals AS signal \
                     ON signal.signal_ref=rule.signal_ref JOIN accepted_cursors AS cursor \
                     ON cursor.edge_node_id=signal.edge_node_id WHERE rule.rule_id=$1 \
                     ON CONFLICT(rule_id,ledger_epoch) DO NOTHING",
                )
                .bind(rule_id)
                .execute(&mut *tx)
                .await?;
                let result = sqlx::query(
                    "UPDATE semantic_rules SET active=FALSE,retired_at=$1 \
                     WHERE rule_id=$2 AND active=TRUE",
                )
                .bind(now)
                .bind(rule_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE output_bindings SET state='draining',revision=revision+1 \
                     WHERE rule_id=$1 AND state='active'",
                )
                .bind(rule_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE output_routes SET lifecycle_state='draining' \
                     WHERE rule_id=$1 AND active=TRUE",
                )
                .bind(rule_id)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                result.rows_affected()
            }
        };
        if affected == 1 {
            Ok(())
        } else {
            Err(StorageError::SemanticNotFound)
        }
    }

    pub async fn reset_semantic_counter(
        &self,
        rule_id: &str,
        now: i64,
    ) -> Result<String, StorageError> {
        let reset_id = prefixed_uuid("reset_");
        let observation_id = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("semantic-v3-reset:{reset_id}").as_bytes(),
        )
        .to_string();
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                apply_reset_sqlite(
                    &mut tx,
                    rule_id,
                    &reset_id,
                    &observation_id,
                    now,
                )
                .await?;
                tx.commit().await?;
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                apply_reset_postgres(
                    &mut tx,
                    rule_id,
                    &reset_id,
                    &observation_id,
                    now,
                )
                .await?;
                tx.commit().await?;
            }
        }
        Ok(reset_id)
    }

    pub async fn semantic_observations(
        &self,
        rule_id: &str,
    ) -> Result<Vec<SemanticObservation>, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let rows = sqlx::query(
                    "SELECT observation_id,rule_id,series_id,sequence,kind,value_json,reading,\
                     observed_at FROM semantic_observations WHERE rule_id=? \
                     ORDER BY observation_row_id",
                )
                .bind(rule_id)
                .fetch_all(pool)
                .await?;
                rows.into_iter().map(sqlite_row_to_observation).collect()
            }
            StorageInner::Postgres { pool, .. } => {
                let rows = sqlx::query(
                    "SELECT observation_id,rule_id,series_id,sequence,kind,\
                     value_json::text AS value_json,reading,observed_at \
                     FROM semantic_observations WHERE rule_id=$1 ORDER BY observation_row_id",
                )
                .bind(rule_id)
                .fetch_all(pool)
                .await?;
                rows.into_iter().map(postgres_row_to_observation).collect()
            }
        }
    }

    pub async fn activate_output_profile(
        &self,
        display_name: &str,
        registration: &'static OutputAdapterRegistration,
        values: Map<String, Value>,
        now: i64,
    ) -> Result<ExportProfile, StorageError> {
        if now < 0 {
            return Err(StorageError::InvalidOutput(
                "profile timestamp must be non-negative".into(),
            ));
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let result = activate_profile_sqlite(
                    &mut tx,
                    display_name,
                    registration,
                    values,
                    now,
                )
                .await?;
                tx.commit().await?;
                Ok(result)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let result = activate_profile_postgres(
                    &mut tx,
                    display_name,
                    registration,
                    values,
                    now,
                )
                .await?;
                tx.commit().await?;
                Ok(result)
            }
        }
    }

    pub async fn configure_output_binding(
        &self,
        binding_id: &str,
        mode: &str,
        values: Map<String, Value>,
        adapters: &'static [OutputAdapterRegistration],
        now: i64,
    ) -> Result<OutputBinding, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let result =
                    configure_binding_sqlite(&mut tx, binding_id, mode, values, adapters, now)
                        .await?;
                tx.commit().await?;
                Ok(result)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let result =
                    configure_binding_postgres(&mut tx, binding_id, mode, values, adapters, now)
                        .await?;
                tx.commit().await?;
                Ok(result)
            }
        }
    }

    pub async fn confirm_output_binding(
        &self,
        binding_id: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                confirm_binding_sqlite(&mut tx, binding_id, now).await?;
                tx.commit().await?;
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                confirm_binding_postgres(&mut tx, binding_id, now).await?;
                tx.commit().await?;
            }
        }
        Ok(())
    }

    pub async fn stop_output_profile(
        &self,
        profile_id: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                stop_profile_sqlite(&mut tx, profile_id, now).await?;
                tx.commit().await?;
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                stop_profile_postgres(&mut tx, profile_id, now).await?;
                tx.commit().await?;
            }
        }
        Ok(())
    }

    pub async fn list_output_profiles(&self) -> Result<Vec<ExportProfile>, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => list_profiles_sqlite(pool).await,
            StorageInner::Postgres { pool, .. } => list_profiles_postgres(pool).await,
        }
    }

    pub async fn pending_output_count(&self) -> Result<i64, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => Ok(sqlx::query_scalar(
                "SELECT COUNT(*) FROM output_outbox WHERE published_at IS NULL",
            )
            .fetch_one(pool)
            .await?),
            StorageInner::Postgres { pool, .. } => Ok(sqlx::query_scalar(
                "SELECT COUNT(*) FROM output_outbox WHERE published_at IS NULL",
            )
            .fetch_one(pool)
            .await?),
        }
    }

    pub async fn claim_output(
        &self,
        claim_token: &str,
        now: i64,
        lease_ms: i64,
    ) -> Result<Option<ClaimedOutput>, StorageError> {
        if claim_token.is_empty() || now < 0 || lease_ms <= 0 {
            return Err(StorageError::InvalidOutput(
                "claim token, timestamp, and positive lease are required".into(),
            ));
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let claimed = claim_sqlite(&mut tx, claim_token, now, lease_ms).await?;
                tx.commit().await?;
                Ok(claimed)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let claimed = claim_postgres(&mut tx, claim_token, now, lease_ms).await?;
                tx.commit().await?;
                Ok(claimed)
            }
        }
    }

    pub async fn release_output(
        &self,
        export_id: &str,
        claim_token: &str,
    ) -> Result<bool, StorageError> {
        let affected = match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => sqlx::query(
                "UPDATE output_outbox SET claim_token=NULL,claimed_at=NULL,claim_until=NULL \
                 WHERE export_id=? AND claim_token=? AND published_at IS NULL",
            )
            .bind(export_id)
            .bind(claim_token)
            .execute(pool)
            .await?
            .rows_affected(),
            StorageInner::Postgres { pool, .. } => sqlx::query(
                "UPDATE output_outbox SET claim_token=NULL,claimed_at=NULL,claim_until=NULL \
                 WHERE export_id=$1 AND claim_token=$2 AND published_at IS NULL",
            )
            .bind(export_id)
            .bind(claim_token)
            .execute(pool)
            .await?
            .rows_affected(),
        };
        Ok(affected == 1)
    }

    pub async fn mark_output_published(
        &self,
        export_id: &str,
        claim_token: &str,
        now: i64,
    ) -> Result<OutputMark, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let result = sqlx::query(
                    "UPDATE output_outbox SET published_at=COALESCE(published_at,?),\
                     claim_token=NULL,claimed_at=NULL,claim_until=NULL \
                     WHERE export_id=? AND claim_token=? AND published_at IS NULL",
                )
                .bind(now)
                .bind(export_id)
                .bind(claim_token)
                .execute(&mut *tx)
                .await?;
                if result.rows_affected() == 0 {
                    let published: Option<i64> = sqlx::query_scalar(
                        "SELECT published_at FROM output_outbox WHERE export_id=?",
                    )
                    .bind(export_id)
                    .fetch_optional(&mut *tx)
                    .await?
                    .flatten();
                    tx.commit().await?;
                    return Ok(if published.is_some() {
                        OutputMark::Published
                    } else {
                        OutputMark::ClaimLost
                    });
                }
                reconcile_profiles_sqlite(&mut tx, now).await?;
                tx.commit().await?;
                Ok(OutputMark::Published)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let result = sqlx::query(
                    "UPDATE output_outbox SET published_at=COALESCE(published_at,$1),\
                     claim_token=NULL,claimed_at=NULL,claim_until=NULL \
                     WHERE export_id=$2 AND claim_token=$3 AND published_at IS NULL",
                )
                .bind(now)
                .bind(export_id)
                .bind(claim_token)
                .execute(&mut *tx)
                .await?;
                if result.rows_affected() == 0 {
                    let published: Option<i64> = sqlx::query_scalar(
                        "SELECT published_at FROM output_outbox WHERE export_id=$1",
                    )
                    .bind(export_id)
                    .fetch_optional(&mut *tx)
                    .await?
                    .flatten();
                    tx.commit().await?;
                    return Ok(if published.is_some() {
                        OutputMark::Published
                    } else {
                        OutputMark::ClaimLost
                    });
                }
                reconcile_profiles_postgres(&mut tx, now).await?;
                tx.commit().await?;
                Ok(OutputMark::Published)
            }
        }
    }

    pub(crate) async fn project_one_semantic(
        &self,
        adapters: &'static [OutputAdapterRegistration],
    ) -> Result<Option<ProjectedOne>, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let projected = project_sqlite(&mut tx, adapters).await?;
                tx.commit().await?;
                Ok(projected)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let projected = project_postgres(&mut tx, adapters).await?;
                tx.commit().await?;
                Ok(projected)
            }
        }
    }
}
