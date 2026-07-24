use iotkit_edge_custody_contract::{
    ActivationRequest, ActivationResult, DescriptorSnapshot, SCHEMA_VERSION,
};
use serde_json::{json, to_vec};
use sqlx::Row;
use uuid::Uuid;

use super::{
    AuditActor, Storage, StorageError, StorageInner,
    auth::{insert_audit_postgres, insert_audit_sqlite},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeNodeState {
    Discovered,
    Activating,
    Active,
    RecoveryHold,
}

impl EdgeNodeState {
    fn parse(value: &str) -> Result<Self, StorageError> {
        match value {
            "discovered" => Ok(Self::Discovered),
            "activating" => Ok(Self::Activating),
            "active" => Ok(Self::Active),
            "recovery_hold" => Ok(Self::RecoveryHold),
            _ => Err(StorageError::InvalidRecord(
                "database contains an invalid Edge Node state".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeNode {
    pub edge_node_ref: String,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub last_descriptor_at: i64,
    pub state: EdgeNodeState,
    pub activation_id: Option<String>,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptorApply {
    pub edge_node: EdgeNode,
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationCommand {
    pub activation_id: String,
    pub topic: String,
    pub payload_json: Vec<u8>,
    pub attempts: i64,
    pub last_attempt_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptorDevice {
    pub edge_node_id: String,
    pub system_id: String,
    pub identifier: Option<String>,
    pub state: String,
    pub presence: String,
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptorSignal {
    pub edge_node_id: String,
    pub series_key: String,
    pub system_id: String,
    pub measurement_key: String,
    pub variant: String,
    pub unit: Option<String>,
    pub value_type: String,
    pub presence: String,
}

impl Storage {
    pub async fn edge_id(&self) -> Result<String, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => Ok(sqlx::query_scalar(
                "SELECT edge_id FROM edge_meta WHERE singleton = 1",
            )
            .fetch_one(pool)
            .await?),
            StorageInner::Postgres { pool, .. } => Ok(sqlx::query_scalar(
                "SELECT edge_id FROM edge_meta WHERE singleton = 1",
            )
            .fetch_one(pool)
            .await?),
        }
    }

    pub async fn initialize_edge_identity(&self, now: i64) -> Result<String, StorageError> {
        if now < 0 {
            return Err(StorageError::InvalidRecord(
                "identity timestamp must not be negative".into(),
            ));
        }
        let candidate = prefixed_id("edge-");
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                sqlx::query(
                    "INSERT INTO edge_meta(singleton, edge_id, created_at) VALUES(1, ?, ?) \
                     ON CONFLICT(singleton) DO NOTHING",
                )
                .bind(candidate)
                .bind(now)
                .execute(pool)
                .await?;
            }
            StorageInner::Postgres { pool, .. } => {
                sqlx::query(
                    "INSERT INTO edge_meta(singleton, edge_id, created_at) VALUES(1, $1, $2) \
                     ON CONFLICT(singleton) DO NOTHING",
                )
                .bind(candidate)
                .bind(now)
                .execute(pool)
                .await?;
            }
        }
        self.edge_id().await
    }

    pub async fn ensure_edge_identity(
        &self,
        expected_edge_id: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        if expected_edge_id.is_empty() || now < 0 {
            return Err(StorageError::InvalidRecord(
                "Edge identity and timestamp must be valid".into(),
            ));
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                sqlx::query(
                    "INSERT INTO edge_meta(singleton, edge_id, created_at) VALUES(1, ?, ?) \
                     ON CONFLICT(singleton) DO NOTHING",
                )
                .bind(expected_edge_id)
                .bind(now)
                .execute(pool)
                .await?;
            }
            StorageInner::Postgres { pool, .. } => {
                sqlx::query(
                    "INSERT INTO edge_meta(singleton, edge_id, created_at) VALUES(1, $1, $2) \
                     ON CONFLICT(singleton) DO NOTHING",
                )
                .bind(expected_edge_id)
                .bind(now)
                .execute(pool)
                .await?;
            }
        }
        if self.edge_id().await? != expected_edge_id {
            return Err(StorageError::EdgeIdentityMismatch);
        }
        Ok(())
    }

    pub async fn apply_descriptor(
        &self,
        descriptor: &DescriptorSnapshot,
        now: i64,
    ) -> Result<DescriptorApply, StorageError> {
        descriptor
            .validate()
            .map_err(|error| StorageError::InvalidRecord(error.to_string()))?;
        if now < 0 {
            return Err(StorageError::InvalidRecord(
                "descriptor timestamp must not be negative".into(),
            ));
        }
        let hash = descriptor
            .content_sha256()
            .map_err(|error| StorageError::InvalidRecord(error.to_string()))?
            .to_vec();
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let existing = sqlx::query(
                    "SELECT ledger_epoch, descriptor_revision, content_sha256 \
                     FROM edge_descriptor_state \
                     WHERE edge_node_id = ?",
                )
                .bind(&descriptor.edge_node_id)
                .fetch_optional(&mut *tx)
                .await?;
                if let Some(row) = existing {
                    let ledger_epoch: String = row.try_get("ledger_epoch")?;
                    let revision: i64 = row.try_get("descriptor_revision")?;
                    let stored_hash: Vec<u8> = row.try_get("content_sha256")?;
                    if ledger_epoch == descriptor.ledger_epoch
                        && (descriptor.descriptor_revision as i64) < revision
                    {
                        let edge_node = load_sqlite(&mut tx, &descriptor.edge_node_id).await?;
                        tx.commit().await?;
                        return Ok(DescriptorApply {
                            edge_node,
                            applied: false,
                        });
                    }
                    if ledger_epoch == descriptor.ledger_epoch
                        && descriptor.descriptor_revision as i64 == revision
                    {
                        if stored_hash != hash {
                            return Err(StorageError::DescriptorConflict);
                        }
                        let edge_node = load_sqlite(&mut tx, &descriptor.edge_node_id).await?;
                        tx.commit().await?;
                        return Ok(DescriptorApply {
                            edge_node,
                            applied: false,
                        });
                    }
                }
                sqlx::query(
                    "UPDATE descriptor_devices SET presence = 'stale', updated_at = ? \
                     WHERE edge_node_id = ?",
                )
                .bind(now)
                .bind(&descriptor.edge_node_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE descriptor_signals SET presence = 'stale', updated_at = ? \
                     WHERE edge_node_id = ?",
                )
                .bind(now)
                .bind(&descriptor.edge_node_id)
                .execute(&mut *tx)
                .await?;
                for device in &descriptor.devices {
                    sqlx::query(
                        "INSERT INTO descriptor_devices(edge_node_id, system_id, identifier, \
                         state, presence, descriptor_revision, updated_at, model_id) \
                         VALUES(?, ?, ?, ?, 'current', ?, ?, ?) \
                         ON CONFLICT(edge_node_id, system_id) DO UPDATE SET \
                         identifier=excluded.identifier, state=excluded.state, \
                         presence='current', descriptor_revision=excluded.descriptor_revision, \
                         updated_at=excluded.updated_at, model_id=excluded.model_id",
                    )
                    .bind(&descriptor.edge_node_id)
                    .bind(&device.system_id)
                    .bind(&device.identifier)
                    .bind(&device.state)
                    .bind(descriptor.descriptor_revision as i64)
                    .bind(now)
                    .bind(&device.model_id)
                    .execute(&mut *tx)
                    .await?;
                }
                for signal in &descriptor.signals {
                    sqlx::query(
                        "INSERT INTO descriptor_signals(edge_node_id, series_key, system_id, \
                         measurement_key, channel_index, variant, unit, value_type, presence, \
                         descriptor_revision, updated_at) VALUES(?, ?, ?, ?, ?, ?, ?, ?, \
                         'current', ?, ?) ON CONFLICT(edge_node_id, series_key) DO UPDATE SET \
                         system_id=excluded.system_id, measurement_key=excluded.measurement_key, \
                         channel_index=excluded.channel_index, variant=excluded.variant, \
                         unit=excluded.unit, value_type=excluded.value_type, presence='current', \
                         descriptor_revision=excluded.descriptor_revision, \
                         updated_at=excluded.updated_at",
                    )
                    .bind(&descriptor.edge_node_id)
                    .bind(&signal.series_key)
                    .bind(&signal.system_id)
                    .bind(&signal.measurement_key)
                    .bind(signal.channel_index)
                    .bind(&signal.variant)
                    .bind(&signal.unit)
                    .bind(&signal.value_type)
                    .bind(descriptor.descriptor_revision as i64)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                }
                for device in &descriptor.devices {
                    sqlx::query(
                        "INSERT OR IGNORE INTO inventory_devices(device_ref,edge_node_id,\
                         system_id,created_at) VALUES(?,?,?,?)",
                    )
                    .bind(prefixed_id("dev_"))
                    .bind(&descriptor.edge_node_id)
                    .bind(&device.system_id)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                }
                for signal in &descriptor.signals {
                    sqlx::query(
                        "INSERT OR IGNORE INTO inventory_signals(signal_ref,edge_node_id,\
                         series_key,system_id,created_at) VALUES(?,?,?,?,?)",
                    )
                    .bind(prefixed_id("sig_"))
                    .bind(&descriptor.edge_node_id)
                    .bind(&signal.series_key)
                    .bind(&signal.system_id)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                }
                sqlx::query(
                    "INSERT INTO edge_descriptor_state(edge_node_id, ledger_epoch, \
                     descriptor_revision, content_sha256, updated_at) VALUES(?, ?, ?, ?, ?) \
                     ON CONFLICT(edge_node_id) DO UPDATE SET ledger_epoch=excluded.ledger_epoch, \
                     descriptor_revision=excluded.descriptor_revision, \
                     content_sha256=excluded.content_sha256, updated_at=excluded.updated_at",
                )
                .bind(&descriptor.edge_node_id)
                .bind(&descriptor.ledger_epoch)
                .bind(descriptor.descriptor_revision as i64)
                .bind(hash)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "INSERT INTO edge_node_activations(edge_node_ref, edge_node_id, ledger_epoch, \
                     state, last_descriptor_at, created_at, updated_at) \
                     VALUES(?, ?, ?, 'discovered', ?, ?, ?) \
                     ON CONFLICT(edge_node_id) DO UPDATE SET \
                     state=CASE WHEN edge_node_activations.ledger_epoch <> excluded.ledger_epoch \
                     THEN 'recovery_hold' ELSE edge_node_activations.state END, \
                     revision=CASE WHEN edge_node_activations.ledger_epoch <> excluded.ledger_epoch \
                     THEN edge_node_activations.revision + 1 ELSE edge_node_activations.revision END, \
                     last_descriptor_at=excluded.last_descriptor_at, updated_at=excluded.updated_at",
                )
                .bind(prefixed_id("en-"))
                .bind(&descriptor.edge_node_id)
                .bind(&descriptor.ledger_epoch)
                .bind(now)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                let edge_node = load_sqlite(&mut tx, &descriptor.edge_node_id).await?;
                tx.commit().await?;
                Ok(DescriptorApply {
                    edge_node,
                    applied: true,
                })
            }
            StorageInner::Postgres { .. } => {
                self.apply_descriptor_postgres(descriptor, now, hash).await
            }
        }
    }

    async fn apply_descriptor_postgres(
        &self,
        descriptor: &DescriptorSnapshot,
        now: i64,
        hash: Vec<u8>,
    ) -> Result<DescriptorApply, StorageError> {
        let StorageInner::Postgres { pool, .. } = self.inner.as_ref() else {
            unreachable!()
        };
        let mut tx = pool.begin().await?;
        let existing = sqlx::query(
            "SELECT ledger_epoch, descriptor_revision, content_sha256 \
             FROM edge_descriptor_state \
             WHERE edge_node_id = $1 FOR UPDATE",
        )
        .bind(&descriptor.edge_node_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = existing {
            let ledger_epoch: String = row.try_get("ledger_epoch")?;
            let revision: i64 = row.try_get("descriptor_revision")?;
            let stored_hash: Vec<u8> = row.try_get("content_sha256")?;
            if ledger_epoch == descriptor.ledger_epoch
                && descriptor.descriptor_revision as i64 <= revision
            {
                if descriptor.descriptor_revision as i64 == revision && stored_hash != hash {
                    return Err(StorageError::DescriptorConflict);
                }
                let edge_node = load_postgres(&mut tx, &descriptor.edge_node_id).await?;
                tx.commit().await?;
                return Ok(DescriptorApply {
                    edge_node,
                    applied: false,
                });
            }
        }
        sqlx::query(
            "UPDATE descriptor_devices SET presence='stale', updated_at=$1 WHERE edge_node_id=$2",
        )
        .bind(now)
        .bind(&descriptor.edge_node_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE descriptor_signals SET presence='stale', updated_at=$1 WHERE edge_node_id=$2",
        )
        .bind(now)
        .bind(&descriptor.edge_node_id)
        .execute(&mut *tx)
        .await?;
        for device in &descriptor.devices {
            sqlx::query(
                "INSERT INTO descriptor_devices(edge_node_id, system_id, identifier, state, \
                 presence, descriptor_revision, updated_at, model_id) \
                 VALUES($1,$2,$3,$4,'current',$5,$6,$7) \
                 ON CONFLICT(edge_node_id,system_id) DO UPDATE SET \
                 identifier=excluded.identifier,state=excluded.state,presence='current',\
                 descriptor_revision=excluded.descriptor_revision,updated_at=excluded.updated_at,\
                 model_id=excluded.model_id",
            )
            .bind(&descriptor.edge_node_id)
            .bind(&device.system_id)
            .bind(&device.identifier)
            .bind(&device.state)
            .bind(descriptor.descriptor_revision as i64)
            .bind(now)
            .bind(&device.model_id)
            .execute(&mut *tx)
            .await?;
        }
        for signal in &descriptor.signals {
            sqlx::query(
                "INSERT INTO descriptor_signals(edge_node_id,series_key,system_id,measurement_key,\
                 channel_index,variant,unit,value_type,presence,descriptor_revision,updated_at) \
                 VALUES($1,$2,$3,$4,$5,$6,$7,$8,'current',$9,$10) \
                 ON CONFLICT(edge_node_id,series_key) DO UPDATE SET \
                 system_id=excluded.system_id,measurement_key=excluded.measurement_key,\
                 channel_index=excluded.channel_index,variant=excluded.variant,unit=excluded.unit,\
                 value_type=excluded.value_type,presence='current',\
                 descriptor_revision=excluded.descriptor_revision,updated_at=excluded.updated_at",
            )
            .bind(&descriptor.edge_node_id)
            .bind(&signal.series_key)
            .bind(&signal.system_id)
            .bind(&signal.measurement_key)
            .bind(signal.channel_index)
            .bind(&signal.variant)
            .bind(&signal.unit)
            .bind(&signal.value_type)
            .bind(descriptor.descriptor_revision as i64)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        for device in &descriptor.devices {
            sqlx::query(
                "INSERT INTO inventory_devices(device_ref,edge_node_id,system_id,created_at) \
                 VALUES($1,$2,$3,$4) ON CONFLICT(edge_node_id,system_id) DO NOTHING",
            )
            .bind(prefixed_id("dev_"))
            .bind(&descriptor.edge_node_id)
            .bind(&device.system_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        for signal in &descriptor.signals {
            sqlx::query(
                "INSERT INTO inventory_signals(signal_ref,edge_node_id,series_key,system_id,\
                 created_at) VALUES($1,$2,$3,$4,$5) \
                 ON CONFLICT(edge_node_id,series_key) DO NOTHING",
            )
            .bind(prefixed_id("sig_"))
            .bind(&descriptor.edge_node_id)
            .bind(&signal.series_key)
            .bind(&signal.system_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO edge_descriptor_state(edge_node_id,ledger_epoch,descriptor_revision,\
             content_sha256,updated_at) VALUES($1,$2,$3,$4,$5) \
             ON CONFLICT(edge_node_id) DO UPDATE SET ledger_epoch=excluded.ledger_epoch,\
             descriptor_revision=excluded.descriptor_revision,\
             content_sha256=excluded.content_sha256,updated_at=excluded.updated_at",
        )
        .bind(&descriptor.edge_node_id)
        .bind(&descriptor.ledger_epoch)
        .bind(descriptor.descriptor_revision as i64)
        .bind(hash)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO edge_node_activations(edge_node_ref,edge_node_id,ledger_epoch,state,\
             last_descriptor_at,created_at,updated_at) VALUES($1,$2,$3,'discovered',$4,$4,$4) \
             ON CONFLICT(edge_node_id) DO UPDATE SET \
             state=CASE WHEN edge_node_activations.ledger_epoch <> excluded.ledger_epoch \
             THEN 'recovery_hold' ELSE edge_node_activations.state END, \
             revision=CASE WHEN edge_node_activations.ledger_epoch <> excluded.ledger_epoch \
             THEN edge_node_activations.revision + 1 ELSE edge_node_activations.revision END, \
             last_descriptor_at=excluded.last_descriptor_at,updated_at=excluded.updated_at",
        )
        .bind(prefixed_id("en-"))
        .bind(&descriptor.edge_node_id)
        .bind(&descriptor.ledger_epoch)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let edge_node = load_postgres(&mut tx, &descriptor.edge_node_id).await?;
        tx.commit().await?;
        Ok(DescriptorApply {
            edge_node,
            applied: true,
        })
    }

    pub async fn edge_node(&self, edge_node_id: &str) -> Result<EdgeNode, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let node = load_sqlite(&mut tx, edge_node_id).await?;
                tx.commit().await?;
                Ok(node)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let node = load_postgres(&mut tx, edge_node_id).await?;
                tx.commit().await?;
                Ok(node)
            }
        }
    }

    pub async fn list_edge_nodes(&self, limit: i64) -> Result<Vec<EdgeNode>, StorageError> {
        if !(1..=100).contains(&limit) {
            return Err(StorageError::InvalidRecord(
                "Edge Node limit must be between 1 and 100".into(),
            ));
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let ids: Vec<String> = sqlx::query_scalar(
                    "SELECT edge_node_id FROM edge_node_activations \
                     ORDER BY edge_node_id LIMIT ?",
                )
                .bind(limit)
                .fetch_all(&mut *tx)
                .await?;
                let mut nodes = Vec::with_capacity(ids.len());
                for id in ids {
                    nodes.push(load_sqlite(&mut tx, &id).await?);
                }
                tx.commit().await?;
                Ok(nodes)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let ids: Vec<String> = sqlx::query_scalar(
                    "SELECT edge_node_id FROM edge_node_activations \
                     ORDER BY edge_node_id LIMIT $1",
                )
                .bind(limit)
                .fetch_all(&mut *tx)
                .await?;
                let mut nodes = Vec::with_capacity(ids.len());
                for id in ids {
                    nodes.push(load_postgres(&mut tx, &id).await?);
                }
                tx.commit().await?;
                Ok(nodes)
            }
        }
    }

    pub async fn list_descriptor_devices(&self) -> Result<Vec<DescriptorDevice>, StorageError> {
        let sql = "SELECT edge_node_id,system_id,identifier,state,presence,model_id \
                   FROM descriptor_devices ORDER BY edge_node_id,system_id";
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => sqlx::query(sql)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(row_to_descriptor_device)
                .collect(),
            StorageInner::Postgres { pool, .. } => sqlx::query(sql)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(row_to_descriptor_device)
                .collect(),
        }
    }

    pub async fn list_descriptor_signals(&self) -> Result<Vec<DescriptorSignal>, StorageError> {
        let sql = "SELECT edge_node_id,series_key,system_id,measurement_key,variant,unit,\
                   value_type,presence FROM descriptor_signals \
                   ORDER BY edge_node_id,series_key";
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => sqlx::query(sql)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(row_to_descriptor_signal)
                .collect(),
            StorageInner::Postgres { pool, .. } => sqlx::query(sql)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(row_to_descriptor_signal)
                .collect(),
        }
    }

    pub async fn request_activation(
        &self,
        edge_node_id: &str,
        now: i64,
    ) -> Result<ActivationCommand, StorageError> {
        self.request_activation_as(AuditActor::local_cli(), edge_node_id, now)
            .await
    }

    pub async fn request_activation_as(
        &self,
        actor: AuditActor,
        edge_node_id: &str,
        now: i64,
    ) -> Result<ActivationCommand, StorageError> {
        let edge_id = self.initialize_edge_identity(now).await?;
        let activation_id = prefixed_id("act-");
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let node = load_sqlite(&mut tx, edge_node_id).await?;
                if node.state == EdgeNodeState::Active || node.state == EdgeNodeState::Activating {
                    return pending_sqlite(&mut tx, node.activation_id.as_deref()).await;
                }
                if node.state != EdgeNodeState::Discovered {
                    return Err(StorageError::ActivationConflict);
                }
                let command = activation_command(
                    &activation_id,
                    &edge_id,
                    &node.edge_node_id,
                    &node.ledger_epoch,
                    now,
                )?;
                sqlx::query(
                    "UPDATE edge_node_activations SET state='activating',activation_id=?,\
                     grant_revision=1,request_json=?,result_json=NULL,revision=revision+1,\
                     updated_at=? WHERE edge_node_id=? AND state='discovered'",
                )
                .bind(&command.activation_id)
                .bind(&command.payload_json)
                .bind(now)
                .bind(edge_node_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "INSERT INTO activation_command_outbox(activation_id,topic,payload_json,\
                     created_at) VALUES(?,?,?,?)",
                )
                .bind(&command.activation_id)
                .bind(&command.topic)
                .bind(&command.payload_json)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "edge_node.activation.request",
                    edge_node_id,
                    json!({"activation_id":command.activation_id}),
                )
                .await?;
                tx.commit().await?;
                Ok(command)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let node = load_postgres(&mut tx, edge_node_id).await?;
                if node.state == EdgeNodeState::Active || node.state == EdgeNodeState::Activating {
                    return pending_postgres(&mut tx, node.activation_id.as_deref()).await;
                }
                if node.state != EdgeNodeState::Discovered {
                    return Err(StorageError::ActivationConflict);
                }
                let command = activation_command(
                    &activation_id,
                    &edge_id,
                    &node.edge_node_id,
                    &node.ledger_epoch,
                    now,
                )?;
                sqlx::query(
                    "UPDATE edge_node_activations SET state='activating',activation_id=$1,\
                     grant_revision=1,request_json=$2,result_json=NULL,revision=revision+1,\
                     updated_at=$3 WHERE edge_node_id=$4 AND state='discovered'",
                )
                .bind(&command.activation_id)
                .bind(&command.payload_json)
                .bind(now)
                .bind(edge_node_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "INSERT INTO activation_command_outbox(activation_id,topic,payload_json,\
                     created_at) VALUES($1,$2,$3,$4)",
                )
                .bind(&command.activation_id)
                .bind(&command.topic)
                .bind(&command.payload_json)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "edge_node.activation.request",
                    edge_node_id,
                    json!({"activation_id":command.activation_id}),
                )
                .await?;
                tx.commit().await?;
                Ok(command)
            }
        }
    }

    pub async fn apply_activation_result(
        &self,
        result: &ActivationResult,
        now: i64,
    ) -> Result<EdgeNode, StorageError> {
        result
            .validate()
            .map_err(|error| StorageError::InvalidRecord(error.to_string()))?;
        let encoded =
            to_vec(result).map_err(|error| StorageError::InvalidRecord(error.to_string()))?;
        let edge_id = self.edge_id().await?;
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let row = sqlx::query(
                    "SELECT edge_node_id,result_json FROM edge_node_activations \
                     WHERE activation_id=?",
                )
                .bind(&result.activation_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(StorageError::ActivationConflict)?;
                let stored_edge_node_id: String = row.try_get("edge_node_id")?;
                let stored_result: Option<Vec<u8>> = row.try_get("result_json")?;
                let node = load_sqlite(&mut tx, &stored_edge_node_id).await?;
                if node.state == EdgeNodeState::Active
                    && stored_result.as_deref() == Some(encoded.as_slice())
                {
                    tx.commit().await?;
                    return Ok(node);
                }
                if validate_result(&node, result, &edge_id).is_err() {
                    sqlx::query(
                        "UPDATE edge_node_activations SET state='recovery_hold',result_json=?,\
                         last_result_at=?,revision=revision+1,updated_at=? WHERE activation_id=?",
                    )
                    .bind(encoded)
                    .bind(now)
                    .bind(now)
                    .bind(&result.activation_id)
                    .execute(&mut *tx)
                    .await?;
                    tx.commit().await?;
                    return Err(StorageError::ActivationConflict);
                }
                sqlx::query(
                    "UPDATE edge_node_activations SET state='active',result_json=?,\
                     last_result_at=?,revision=revision+1,updated_at=? \
                     WHERE edge_node_id=? AND state='activating'",
                )
                .bind(encoded)
                .bind(now)
                .bind(now)
                .bind(&result.edge_node_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE activation_command_outbox SET completed_at=? WHERE activation_id=?",
                )
                .bind(now)
                .bind(&result.activation_id)
                .execute(&mut *tx)
                .await?;
                let node = load_sqlite(&mut tx, &result.edge_node_id).await?;
                tx.commit().await?;
                Ok(node)
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let row = sqlx::query(
                    "SELECT edge_node_id,result_json FROM edge_node_activations \
                     WHERE activation_id=$1 FOR UPDATE",
                )
                .bind(&result.activation_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(StorageError::ActivationConflict)?;
                let stored_edge_node_id: String = row.try_get("edge_node_id")?;
                let stored_result: Option<Vec<u8>> = row.try_get("result_json")?;
                let node = load_postgres(&mut tx, &stored_edge_node_id).await?;
                if node.state == EdgeNodeState::Active
                    && stored_result.as_deref() == Some(encoded.as_slice())
                {
                    tx.commit().await?;
                    return Ok(node);
                }
                if validate_result(&node, result, &edge_id).is_err() {
                    sqlx::query(
                        "UPDATE edge_node_activations SET state='recovery_hold',result_json=$1,\
                         last_result_at=$2,revision=revision+1,updated_at=$2 \
                         WHERE activation_id=$3",
                    )
                    .bind(encoded)
                    .bind(now)
                    .bind(&result.activation_id)
                    .execute(&mut *tx)
                    .await?;
                    tx.commit().await?;
                    return Err(StorageError::ActivationConflict);
                }
                sqlx::query(
                    "UPDATE edge_node_activations SET state='active',result_json=$1,\
                     last_result_at=$2,revision=revision+1,updated_at=$2 \
                     WHERE edge_node_id=$3 AND state='activating'",
                )
                .bind(encoded)
                .bind(now)
                .bind(&result.edge_node_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE activation_command_outbox SET completed_at=$1 WHERE activation_id=$2",
                )
                .bind(now)
                .bind(&result.activation_id)
                .execute(&mut *tx)
                .await?;
                let node = load_postgres(&mut tx, &result.edge_node_id).await?;
                tx.commit().await?;
                Ok(node)
            }
        }
    }

    pub async fn pending_activation_commands(
        &self,
        limit: i64,
    ) -> Result<Vec<ActivationCommand>, StorageError> {
        if !(1..=1_000).contains(&limit) {
            return Err(StorageError::InvalidRecord(
                "activation command limit must be between 1 and 1000".into(),
            ));
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let rows = sqlx::query(
                    "SELECT activation_id,topic,payload_json,attempts,last_attempt_at \
                     FROM activation_command_outbox WHERE completed_at IS NULL \
                     ORDER BY created_at,activation_id LIMIT ?",
                )
                .bind(limit)
                .fetch_all(pool)
                .await?;
                rows.into_iter().map(row_to_command).collect()
            }
            StorageInner::Postgres { pool, .. } => {
                let rows = sqlx::query(
                    "SELECT activation_id,topic,payload_json,attempts,last_attempt_at \
                     FROM activation_command_outbox WHERE completed_at IS NULL \
                     ORDER BY created_at,activation_id LIMIT $1",
                )
                .bind(limit)
                .fetch_all(pool)
                .await?;
                rows.into_iter().map(row_to_command).collect()
            }
        }
    }

    pub async fn mark_activation_attempt(
        &self,
        activation_id: &str,
        at: i64,
    ) -> Result<(), StorageError> {
        if at < 0 {
            return Err(StorageError::InvalidRecord(
                "activation attempt timestamp must not be negative".into(),
            ));
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                sqlx::query(
                    "UPDATE activation_command_outbox SET attempts=attempts+1,last_attempt_at=? \
                     WHERE activation_id=? AND completed_at IS NULL",
                )
                .bind(at)
                .bind(activation_id)
                .execute(pool)
                .await?;
            }
            StorageInner::Postgres { pool, .. } => {
                sqlx::query(
                    "UPDATE activation_command_outbox SET attempts=attempts+1,last_attempt_at=$1 \
                     WHERE activation_id=$2 AND completed_at IS NULL",
                )
                .bind(at)
                .bind(activation_id)
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }
}

fn row_to_descriptor_device<R: sqlx::Row>(row: R) -> Result<DescriptorDevice, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    Option<String>: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    Ok(DescriptorDevice {
        edge_node_id: row.try_get("edge_node_id")?,
        system_id: row.try_get("system_id")?,
        identifier: row.try_get("identifier")?,
        state: row.try_get("state")?,
        presence: row.try_get("presence")?,
        model_id: row.try_get("model_id")?,
    })
}

fn row_to_descriptor_signal<R: sqlx::Row>(row: R) -> Result<DescriptorSignal, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    Option<String>: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    Ok(DescriptorSignal {
        edge_node_id: row.try_get("edge_node_id")?,
        series_key: row.try_get("series_key")?,
        system_id: row.try_get("system_id")?,
        measurement_key: row.try_get("measurement_key")?,
        variant: row.try_get("variant")?,
        unit: row.try_get("unit")?,
        value_type: row.try_get("value_type")?,
        presence: row.try_get("presence")?,
    })
}

fn activation_command(
    activation_id: &str,
    edge_id: &str,
    edge_node_id: &str,
    ledger_epoch: &str,
    now: i64,
) -> Result<ActivationCommand, StorageError> {
    let request = ActivationRequest {
        schema_version: SCHEMA_VERSION,
        activation_id: activation_id.into(),
        edge_id: edge_id.into(),
        edge_node_id: edge_node_id.into(),
        expected_ledger_epoch: ledger_epoch.into(),
        grant_revision: 1,
        issued_at: now,
    };
    request
        .validate()
        .map_err(|error| StorageError::InvalidRecord(error.to_string()))?;
    let payload_json =
        to_vec(&request).map_err(|error| StorageError::InvalidRecord(error.to_string()))?;
    Ok(ActivationCommand {
        activation_id: activation_id.into(),
        topic: format!("iotkit/v1/edge-nodes/{edge_node_id}/activation/request"),
        payload_json,
        attempts: 0,
        last_attempt_at: None,
    })
}

fn validate_result(
    node: &EdgeNode,
    result: &ActivationResult,
    edge_id: &str,
) -> Result<(), StorageError> {
    if node.state != EdgeNodeState::Activating
        || node.activation_id.as_deref() != Some(&result.activation_id)
        || node.edge_node_id != result.edge_node_id
        || node.ledger_epoch != result.ledger_epoch
        || result.edge_id != edge_id
    {
        return Err(StorageError::ActivationConflict);
    }
    Ok(())
}

async fn load_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    edge_node_id: &str,
) -> Result<EdgeNode, StorageError> {
    let row = sqlx::query(
        "SELECT edge_node_ref,edge_node_id,ledger_epoch,last_descriptor_at,state,activation_id,revision \
         FROM edge_node_activations WHERE edge_node_id=?",
    )
    .bind(edge_node_id)
    .fetch_one(&mut **tx)
    .await?;
    row_to_node(row)
}

async fn load_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    edge_node_id: &str,
) -> Result<EdgeNode, StorageError> {
    let row = sqlx::query(
        "SELECT edge_node_ref,edge_node_id,ledger_epoch,last_descriptor_at,state,activation_id,revision \
         FROM edge_node_activations WHERE edge_node_id=$1 FOR UPDATE",
    )
    .bind(edge_node_id)
    .fetch_one(&mut **tx)
    .await?;
    row_to_node(row)
}

fn row_to_node<R>(row: R) -> Result<EdgeNode, StorageError>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    for<'a> String: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> Option<String>: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> i64: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
{
    let state: String = row.try_get("state")?;
    Ok(EdgeNode {
        edge_node_ref: row.try_get("edge_node_ref")?,
        edge_node_id: row.try_get("edge_node_id")?,
        ledger_epoch: row.try_get("ledger_epoch")?,
        last_descriptor_at: row.try_get("last_descriptor_at")?,
        state: EdgeNodeState::parse(&state)?,
        activation_id: row.try_get("activation_id")?,
        revision: row.try_get("revision")?,
    })
}

async fn pending_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    activation_id: Option<&str>,
) -> Result<ActivationCommand, StorageError> {
    let id = activation_id.ok_or(StorageError::ActivationConflict)?;
    let row = sqlx::query(
        "SELECT activation_id,topic,payload_json,attempts,last_attempt_at \
         FROM activation_command_outbox \
         WHERE activation_id=?",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    row_to_command(row)
}

async fn pending_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    activation_id: Option<&str>,
) -> Result<ActivationCommand, StorageError> {
    let id = activation_id.ok_or(StorageError::ActivationConflict)?;
    let row = sqlx::query(
        "SELECT activation_id,topic,payload_json,attempts,last_attempt_at \
         FROM activation_command_outbox \
         WHERE activation_id=$1",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    row_to_command(row)
}

fn row_to_command<R>(row: R) -> Result<ActivationCommand, StorageError>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    for<'a> String: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> Vec<u8>: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> i64: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> Option<i64>: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
{
    Ok(ActivationCommand {
        activation_id: row.try_get("activation_id")?,
        topic: row.try_get("topic")?,
        payload_json: row.try_get("payload_json")?,
        attempts: row.try_get("attempts")?,
        last_attempt_at: row.try_get("last_attempt_at")?,
    })
}

fn prefixed_id(prefix: &str) -> String {
    format!("{prefix}{}", Uuid::new_v4().simple())
}
