use iotkit_output_adapter_api::{
    AdapterError, IdentityScope, Observation, ObservationKind, ObservationValue, ProfileRequest,
};
use serde::Deserialize;
use serde_json::{Map, Value, json, value::RawValue};
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

use super::{
    AuditActor, Storage, StorageError, StorageInner,
    auth::{insert_audit_postgres, insert_audit_sqlite},
};

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

#[derive(Debug, Clone)]
pub(crate) struct StoredPublication {
    pub topic: String,
    pub qos: u8,
    pub retain: bool,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredOutputObservation {
    pub observation_id: String,
    pub series_id: String,
    pub sequence: u64,
    pub observed_at: i64,
    pub value: ObservationValue,
}

#[derive(Debug, Clone)]
pub(crate) struct OutputPublicationSnapshot {
    pub adapter_id: String,
    pub config: Vec<u8>,
    pub kind: SemanticKind,
    pub observation: Option<StoredOutputObservation>,
    pub actual: Option<StoredPublication>,
    pub pending_count: i64,
    pub published_count: i64,
    pub oldest_pending_at: Option<i64>,
    pub last_published_at: Option<i64>,
}

async fn latest_output_observation_sqlite(
    pool: &sqlx::SqlitePool,
    binding_id: &str,
) -> Result<Option<StoredOutputObservation>, StorageError> {
    let row = sqlx::query(
        "SELECT observation_id,series_id,sequence,kind,value_json,reading,observed_at \
         FROM semantic_observations WHERE rule_id=(\
           SELECT rule_id FROM output_bindings WHERE binding_id=?) \
         ORDER BY observation_row_id DESC LIMIT 1",
    )
    .bind(binding_id)
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        let decoded = decode_stored_observation(
            row.try_get("observation_id")?,
            String::new(),
            row.try_get("series_id")?,
            row.try_get("sequence")?,
            parse_semantic_kind(&row.try_get::<String, _>("kind")?)?,
            &row.try_get::<Vec<u8>, _>("value_json")?,
            row.try_get("reading")?,
            row.try_get("observed_at")?,
        )?;
        Ok(StoredOutputObservation {
            observation_id: decoded.observation_id,
            series_id: decoded.series_id,
            sequence: decoded.sequence,
            observed_at: decoded.observed_at,
            value: decoded.value,
        })
    })
    .transpose()
}

async fn latest_output_observation_postgres(
    pool: &sqlx::PgPool,
    binding_id: &str,
) -> Result<Option<StoredOutputObservation>, StorageError> {
    let row = sqlx::query(
        "SELECT observation_id,series_id,sequence,kind,value_json::text value_json,reading,observed_at \
         FROM semantic_observations WHERE rule_id=(\
           SELECT rule_id FROM output_bindings WHERE binding_id=$1) \
         ORDER BY observation_row_id DESC LIMIT 1",
    )
    .bind(binding_id)
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        let value: String = row.try_get("value_json")?;
        let decoded = decode_stored_observation(
            row.try_get("observation_id")?,
            String::new(),
            row.try_get("series_id")?,
            row.try_get("sequence")?,
            parse_semantic_kind(&row.try_get::<String, _>("kind")?)?,
            value.as_bytes(),
            row.try_get("reading")?,
            row.try_get("observed_at")?,
        )?;
        Ok(StoredOutputObservation {
            observation_id: decoded.observation_id,
            series_id: decoded.series_id,
            sequence: decoded.sequence,
            observed_at: decoded.observed_at,
            value: decoded.value,
        })
    })
    .transpose()
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
    pub async fn list_semantic_rules(&self) -> Result<Vec<SemanticRule>, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let rows = sqlx::query(
                    "SELECT rule.rule_id,rule.signal_ref,signal.edge_node_id,signal.series_key,\
                     rule.display_name,rule.kind,rule.series_id,rule.revision,rule.active \
                     FROM semantic_rules AS rule JOIN semantic_signals AS signal \
                     ON signal.signal_ref=rule.signal_ref ORDER BY rule.created_at,rule.rule_id",
                )
                .fetch_all(pool)
                .await?;
                rows.into_iter().map(row_to_semantic_rule).collect()
            }
            StorageInner::Postgres { pool, .. } => {
                let rows = sqlx::query(
                    "SELECT rule.rule_id,rule.signal_ref,signal.edge_node_id,signal.series_key,\
                     rule.display_name,rule.kind,rule.series_id,rule.revision,rule.active \
                     FROM semantic_rules AS rule JOIN semantic_signals AS signal \
                     ON signal.signal_ref=rule.signal_ref ORDER BY rule.created_at,rule.rule_id",
                )
                .fetch_all(pool)
                .await?;
                rows.into_iter().map(row_to_semantic_rule).collect()
            }
        }
    }

    pub async fn create_semantic_rule(
        &self,
        draft: SemanticRuleDraft,
        now: i64,
    ) -> Result<SemanticRule, StorageError> {
        self.create_semantic_rule_as(AuditActor::local_cli(), draft, now)
            .await
    }

    pub async fn create_semantic_rule_as(
        &self,
        actor: AuditActor,
        draft: SemanticRuleDraft,
        now: i64,
    ) -> Result<SemanticRule, StorageError> {
        validate_rule_draft(&draft, now)?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let rule = create_rule_sqlite(&mut tx, draft, now).await?;
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "semantic_rule.create",
                    &rule.rule_id,
                    json!({"revision":rule.revision,"signal_ref":rule.signal_ref}),
                )
                .await?;
                tx.commit().await?;
                Ok(rule)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let rule = create_rule_postgres(&mut tx, draft, now).await?;
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "semantic_rule.create",
                    &rule.rule_id,
                    json!({"revision":rule.revision,"signal_ref":rule.signal_ref}),
                )
                .await?;
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
        self.revise_semantic_rule_as(
            AuditActor::local_cli(),
            rule_id,
            display_name,
            spec,
            now,
        )
        .await
    }

    pub async fn revise_semantic_rule_as(
        &self,
        actor: AuditActor,
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
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "semantic_rule.revise",
                    rule_id,
                    json!({"revision":rule.revision}),
                )
                .await?;
                tx.commit().await?;
                Ok(rule)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let rule =
                    revise_rule_postgres(&mut tx, rule_id, display_name, spec, now).await?;
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "semantic_rule.revise",
                    rule_id,
                    json!({"revision":rule.revision}),
                )
                .await?;
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
        self.update_semantic_calibration_as(
            AuditActor::local_cli(),
            signal_ref,
            scale,
            offset,
            now,
        )
        .await
    }

    pub async fn update_semantic_calibration_as(
        &self,
        actor: AuditActor,
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
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "semantic_calibration.update",
                    signal_ref,
                    json!({"revision":revision,"scale":scale,"offset":offset}),
                )
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
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "semantic_calibration.update",
                    signal_ref,
                    json!({"revision":revision,"scale":scale,"offset":offset}),
                )
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
        self.retire_semantic_rule_as(AuditActor::local_cli(), rule_id, now)
            .await
    }

    pub async fn retire_semantic_rule_as(
        &self,
        actor: AuditActor,
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
                if result.rows_affected() == 1 {
                    insert_audit_sqlite(
                        &mut tx,
                        &actor,
                        now,
                        "semantic_rule.retire",
                        rule_id,
                        json!({}),
                    )
                    .await?;
                }
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
                if result.rows_affected() == 1 {
                    insert_audit_postgres(
                        &mut tx,
                        &actor,
                        now,
                        "semantic_rule.retire",
                        rule_id,
                        json!({}),
                    )
                    .await?;
                }
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
        self.reset_semantic_counter_as(AuditActor::local_cli(), rule_id, now)
            .await
    }

    pub async fn reset_semantic_counter_as(
        &self,
        actor: AuditActor,
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
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "semantic_counter.reset",
                    rule_id,
                    json!({"reset_id":reset_id}),
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
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "semantic_counter.reset",
                    rule_id,
                    json!({"reset_id":reset_id}),
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

    pub async fn latest_semantic_observation(
        &self,
        rule_id: &str,
    ) -> Result<Option<SemanticObservation>, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => sqlx::query(
                "SELECT observation_id,rule_id,series_id,sequence,kind,value_json,reading,\
                 observed_at FROM semantic_observations WHERE rule_id=? \
                 ORDER BY observation_row_id DESC LIMIT 1",
            )
            .bind(rule_id)
            .fetch_optional(pool)
            .await?
            .map(sqlite_row_to_observation)
            .transpose(),
            StorageInner::Postgres { pool, .. } => sqlx::query(
                "SELECT observation_id,rule_id,series_id,sequence,kind,\
                 value_json::text AS value_json,reading,observed_at \
                 FROM semantic_observations WHERE rule_id=$1 \
                 ORDER BY observation_row_id DESC LIMIT 1",
            )
            .bind(rule_id)
            .fetch_optional(pool)
            .await?
            .map(postgres_row_to_observation)
            .transpose(),
        }
    }

    pub(crate) async fn output_publication_snapshot(
        &self,
        binding_id: &str,
    ) -> Result<OutputPublicationSnapshot, StorageError> {
        if binding_id.is_empty() {
            return Err(StorageError::InvalidOutput("binding is required".into()));
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let route = sqlx::query(
                    "SELECT route.route_id,route.adapter_id,route.config_json,rule.kind \
                     FROM output_routes route JOIN semantic_rules rule ON rule.rule_id=route.rule_id \
                     WHERE route.binding_id=?",
                )
                .bind(binding_id)
                .fetch_optional(pool)
                .await?
                .ok_or(StorageError::SemanticNotFound)?;
                let route_id: String = route.try_get("route_id")?;
                let actual = sqlx::query(
                    "SELECT topic,qos,retain,payload_json FROM output_outbox WHERE route_id=? \
                     ORDER BY created_at DESC,export_id DESC LIMIT 1",
                )
                .bind(&route_id)
                .fetch_optional(pool)
                .await?
                .map(|row| {
                    Ok::<_, StorageError>(StoredPublication {
                        topic: row.try_get("topic")?,
                        qos: u8::try_from(row.try_get::<i64, _>("qos")?)
                            .map_err(|_| StorageError::InvalidOutput("invalid qos".into()))?,
                        retain: row.try_get("retain")?,
                        payload: row.try_get("payload_json")?,
                    })
                })
                .transpose()?;
                let observation = latest_output_observation_sqlite(pool, binding_id).await?;
                let delivery = sqlx::query(
                    "SELECT COALESCE(SUM(CASE WHEN published_at IS NULL THEN 1 ELSE 0 END),0) pending_count,\
                     COALESCE(SUM(CASE WHEN published_at IS NOT NULL THEN 1 ELSE 0 END),0) published_count,\
                     MIN(CASE WHEN published_at IS NULL THEN created_at END) oldest_pending_at,\
                     MAX(published_at) last_published_at FROM output_outbox WHERE route_id=?",
                )
                .bind(&route_id)
                .fetch_one(pool)
                .await?;
                Ok(OutputPublicationSnapshot {
                    adapter_id: route.try_get("adapter_id")?,
                    config: route.try_get("config_json")?,
                    kind: parse_semantic_kind(&route.try_get::<String, _>("kind")?)?,
                    observation,
                    actual,
                    pending_count: delivery.try_get("pending_count")?,
                    published_count: delivery.try_get("published_count")?,
                    oldest_pending_at: delivery.try_get("oldest_pending_at")?,
                    last_published_at: delivery.try_get("last_published_at")?,
                })
            }
            StorageInner::Postgres { pool, .. } => {
                let route = sqlx::query(
                    "SELECT route.route_id,route.adapter_id,route.config_json::text config_json,rule.kind \
                     FROM output_routes route JOIN semantic_rules rule ON rule.rule_id=route.rule_id \
                     WHERE route.binding_id=$1",
                )
                .bind(binding_id)
                .fetch_optional(pool)
                .await?
                .ok_or(StorageError::SemanticNotFound)?;
                let route_id: String = route.try_get("route_id")?;
                let actual = sqlx::query(
                    "SELECT topic,qos,retain,payload_json::text payload_json FROM output_outbox \
                     WHERE route_id=$1 ORDER BY created_at DESC,export_id DESC LIMIT 1",
                )
                .bind(&route_id)
                .fetch_optional(pool)
                .await?
                .map(|row| {
                    Ok::<_, StorageError>(StoredPublication {
                        topic: row.try_get("topic")?,
                        qos: u8::try_from(row.try_get::<i16, _>("qos")?)
                            .map_err(|_| StorageError::InvalidOutput("invalid qos".into()))?,
                        retain: row.try_get("retain")?,
                        payload: row.try_get::<String, _>("payload_json")?.into_bytes(),
                    })
                })
                .transpose()?;
                let observation = latest_output_observation_postgres(pool, binding_id).await?;
                let delivery = sqlx::query(
                    "SELECT COUNT(*) FILTER (WHERE published_at IS NULL)::bigint pending_count,\
                     COUNT(*) FILTER (WHERE published_at IS NOT NULL)::bigint published_count,\
                     MIN(created_at) FILTER (WHERE published_at IS NULL) oldest_pending_at,\
                     MAX(published_at) last_published_at FROM output_outbox WHERE route_id=$1",
                )
                .bind(&route_id)
                .fetch_one(pool)
                .await?;
                Ok(OutputPublicationSnapshot {
                    adapter_id: route.try_get("adapter_id")?,
                    config: route.try_get::<String, _>("config_json")?.into_bytes(),
                    kind: parse_semantic_kind(&route.try_get::<String, _>("kind")?)?,
                    observation,
                    actual,
                    pending_count: delivery.try_get("pending_count")?,
                    published_count: delivery.try_get("published_count")?,
                    oldest_pending_at: delivery.try_get("oldest_pending_at")?,
                    last_published_at: delivery.try_get("last_published_at")?,
                })
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
        self.activate_output_profile_as(
            AuditActor::local_cli(),
            display_name,
            registration,
            values,
            now,
        )
        .await
    }

    pub async fn activate_output_profile_as(
        &self,
        actor: AuditActor,
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
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "export_profile.activate",
                    &result.profile_id,
                    json!({"adapter_id":result.adapter_id,"revision":result.revision}),
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
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "export_profile.activate",
                    &result.profile_id,
                    json!({"adapter_id":result.adapter_id,"revision":result.revision}),
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
        self.configure_output_binding_as(
            AuditActor::local_cli(),
            binding_id,
            mode,
            values,
            adapters,
            now,
        )
        .await
    }

    pub async fn configure_output_binding_as(
        &self,
        actor: AuditActor,
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
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "output_binding.configure",
                    binding_id,
                    json!({"mode":mode}),
                )
                .await?;
                tx.commit().await?;
                Ok(result)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let result =
                    configure_binding_postgres(&mut tx, binding_id, mode, values, adapters, now)
                        .await?;
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "output_binding.configure",
                    binding_id,
                    json!({"mode":mode}),
                )
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
        self.confirm_output_binding_as(AuditActor::local_cli(), binding_id, now)
            .await
    }

    pub async fn confirm_output_binding_as(
        &self,
        actor: AuditActor,
        binding_id: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                confirm_binding_sqlite(&mut tx, binding_id, now).await?;
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "output_binding.confirm",
                    binding_id,
                    json!({}),
                )
                .await?;
                tx.commit().await?;
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                confirm_binding_postgres(&mut tx, binding_id, now).await?;
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "output_binding.confirm",
                    binding_id,
                    json!({}),
                )
                .await?;
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
        self.stop_output_profile_as(AuditActor::local_cli(), profile_id, now)
            .await
    }

    pub async fn stop_output_profile_as(
        &self,
        actor: AuditActor,
        profile_id: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                stop_profile_sqlite(&mut tx, profile_id, now).await?;
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "export_profile.stop",
                    profile_id,
                    json!({}),
                )
                .await?;
                tx.commit().await?;
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                stop_profile_postgres(&mut tx, profile_id, now).await?;
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "export_profile.stop",
                    profile_id,
                    json!({}),
                )
                .await?;
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
