use iotkit_edge_custody_contract::{CollectorState, StatusAdapter, StatusHeartbeat};
use sqlx::{Postgres, Row, Sqlite, Transaction};

use super::{Storage, StorageError, StorageInner};

/// Outcome for latest-only status evidence. A replay is successful transport
/// handling but deliberately does not refresh the live-liveness clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusApply {
    AcceptedLive,
    StoredRetained,
    IgnoredReplay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeNodeStatus {
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub boot_id: String,
    pub status_seq: u64,
    pub collector_state: CollectorState,
    pub adapters: Vec<StatusAdapter>,
    pub accepted_through: i64,
    pub pending_publications: i64,
    pub storage_pressure: bool,
    pub received_at: i64,
    pub last_live_received_at: Option<i64>,
    /// Edge receipt time of the current positive pending interval. It is
    /// operational evidence, not a Node-provided timestamp.
    pub pending_since_at: Option<i64>,
}

impl Storage {
    /// Stores one active-epoch status snapshot.  `retained` is MQTT packet
    /// metadata, not payload data, so an old retained message cannot be
    /// mistaken for current liveness after the Edge restarts.
    pub async fn apply_edge_node_status(
        &self,
        heartbeat: &StatusHeartbeat,
        received_at: i64,
        retained: bool,
    ) -> Result<StatusApply, StorageError> {
        heartbeat
            .validate()
            .map_err(|error| StorageError::InvalidRecord(error.to_string()))?;
        if received_at < 0 {
            return Err(StorageError::InvalidRecord(
                "status receipt timestamp must not be negative".into(),
            ));
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut transaction = pool.begin().await?;
                ensure_active_sqlite(&mut transaction, heartbeat).await?;
                let changed =
                    write_sqlite(&mut transaction, heartbeat, received_at, retained).await?;
                if changed == 0 {
                    ensure_active_sqlite(&mut transaction, heartbeat).await?;
                }
                transaction.commit().await?;
                Ok(if changed == 0 {
                    StatusApply::IgnoredReplay
                } else if retained {
                    StatusApply::StoredRetained
                } else {
                    StatusApply::AcceptedLive
                })
            }
            StorageInner::Postgres { pool, .. } => {
                let mut transaction = pool.begin().await?;
                ensure_active_postgres(&mut transaction, heartbeat).await?;
                let changed =
                    write_postgres(&mut transaction, heartbeat, received_at, retained).await?;
                if changed == 0 {
                    ensure_active_postgres(&mut transaction, heartbeat).await?;
                }
                transaction.commit().await?;
                Ok(if changed == 0 {
                    StatusApply::IgnoredReplay
                } else if retained {
                    StatusApply::StoredRetained
                } else {
                    StatusApply::AcceptedLive
                })
            }
        }
    }

    /// Reads one current active-epoch snapshot. Diagnostics uses aggregate SQL
    /// for fleet facts; it never loads an unbounded status inventory.
    pub async fn edge_node_status(
        &self,
        edge_node_id: &str,
    ) -> Result<Option<EdgeNodeStatus>, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => sqlx::query(
                "SELECT status.edge_node_id,status.ledger_epoch,status.boot_id,status.status_seq,\
                 status.collector_state,status.adapters_json,status.accepted_through,\
                 status.pending_publications,status.storage_pressure,status.received_at,\
                 status.last_live_received_at,status.pending_since_at FROM edge_node_status AS status \
                 JOIN edge_node_activations AS activation \
                 ON activation.edge_node_id=status.edge_node_id \
                 AND activation.ledger_epoch=status.ledger_epoch \
                 WHERE activation.state='active' AND status.edge_node_id=?",
            )
            .bind(edge_node_id)
            .fetch_optional(pool)
            .await?
            .map(decode_sqlite)
            .transpose(),
            StorageInner::Postgres { pool, .. } => sqlx::query(
                "SELECT status.edge_node_id,status.ledger_epoch,status.boot_id,status.status_seq,\
                 status.collector_state,status.adapters_json,status.accepted_through,\
                 status.pending_publications,status.storage_pressure,status.received_at,\
                 status.last_live_received_at,status.pending_since_at FROM edge_node_status AS status \
                 JOIN edge_node_activations AS activation \
                 ON activation.edge_node_id=status.edge_node_id \
                 AND activation.ledger_epoch=status.ledger_epoch \
                 WHERE activation.state='active' AND status.edge_node_id=$1",
            )
            .bind(edge_node_id)
            .fetch_optional(pool)
            .await?
            .map(decode_postgres)
            .transpose(),
        }
    }
}

async fn ensure_active_sqlite(
    transaction: &mut Transaction<'_, Sqlite>,
    heartbeat: &StatusHeartbeat,
) -> Result<(), StorageError> {
    let state: Option<String> = sqlx::query_scalar(
        "SELECT state FROM edge_node_activations \
         WHERE edge_node_id=? AND ledger_epoch=?",
    )
    .bind(&heartbeat.edge_node_id)
    .bind(&heartbeat.ledger_epoch)
    .fetch_optional(&mut **transaction)
    .await?;
    if state.as_deref() != Some("active") {
        return Err(StorageError::EdgeNodeNotActive);
    }
    Ok(())
}

async fn ensure_active_postgres(
    transaction: &mut Transaction<'_, Postgres>,
    heartbeat: &StatusHeartbeat,
) -> Result<(), StorageError> {
    let state: Option<String> = sqlx::query_scalar(
        "SELECT state FROM edge_node_activations \
         WHERE edge_node_id=$1 AND ledger_epoch=$2 FOR SHARE",
    )
    .bind(&heartbeat.edge_node_id)
    .bind(&heartbeat.ledger_epoch)
    .fetch_optional(&mut **transaction)
    .await?;
    if state.as_deref() != Some("active") {
        return Err(StorageError::EdgeNodeNotActive);
    }
    Ok(())
}

async fn write_sqlite(
    transaction: &mut Transaction<'_, Sqlite>,
    heartbeat: &StatusHeartbeat,
    received_at: i64,
    retained: bool,
) -> Result<u64, StorageError> {
    let adapters = serde_json::to_vec(&heartbeat.adapters).map_err(StorageError::EncodeRecord)?;
    let statement = if retained {
        "INSERT INTO edge_node_status(\
         edge_node_id,ledger_epoch,boot_id,status_seq,collector_state,adapters_json,\
         accepted_through,pending_publications,storage_pressure,received_at,last_live_received_at,pending_since_at\
         ) SELECT ?,?,?,?,?,?,?,?,?,?,?,? WHERE EXISTS(\
         SELECT 1 FROM edge_node_activations WHERE edge_node_id=? AND ledger_epoch=? AND state='active'\
         ) ON CONFLICT(edge_node_id) DO NOTHING"
    } else {
        "INSERT INTO edge_node_status(\
         edge_node_id,ledger_epoch,boot_id,status_seq,collector_state,adapters_json,\
         accepted_through,pending_publications,storage_pressure,received_at,last_live_received_at,pending_since_at\
         ) SELECT ?,?,?,?,?,?,?,?,?,?,?,? WHERE EXISTS(\
         SELECT 1 FROM edge_node_activations WHERE edge_node_id=? AND ledger_epoch=? AND state='active'\
         ) ON CONFLICT(edge_node_id) DO UPDATE SET \
         ledger_epoch=excluded.ledger_epoch,boot_id=excluded.boot_id,status_seq=excluded.status_seq,\
         collector_state=excluded.collector_state,adapters_json=excluded.adapters_json,\
         accepted_through=excluded.accepted_through,pending_publications=excluded.pending_publications,\
         storage_pressure=excluded.storage_pressure,received_at=excluded.received_at,\
         last_live_received_at=excluded.last_live_received_at,\
         pending_since_at=CASE WHEN excluded.pending_publications=0 THEN NULL \
           WHEN edge_node_status.boot_id<>excluded.boot_id \
             OR edge_node_status.ledger_epoch<>excluded.ledger_epoch \
             OR edge_node_status.pending_publications=0 \
             OR edge_node_status.pending_since_at IS NULL \
             OR (edge_node_status.ledger_epoch=excluded.ledger_epoch \
                 AND excluded.accepted_through>edge_node_status.accepted_through) THEN excluded.received_at \
           ELSE edge_node_status.pending_since_at END WHERE \
         edge_node_status.boot_id<>excluded.boot_id OR edge_node_status.status_seq<excluded.status_seq"
    };
    let result = sqlx::query(statement)
        .bind(&heartbeat.edge_node_id)
        .bind(&heartbeat.ledger_epoch)
        .bind(&heartbeat.boot_id)
        .bind(heartbeat.status_seq as i64)
        .bind(collector_state_text(heartbeat.collector_state))
        .bind(adapters)
        .bind(heartbeat.accepted_through)
        .bind(heartbeat.pending_publications)
        .bind(heartbeat.storage_pressure)
        .bind(received_at)
        .bind((!retained).then_some(received_at))
        .bind((!retained && heartbeat.pending_publications > 0).then_some(received_at))
        .bind(&heartbeat.edge_node_id)
        .bind(&heartbeat.ledger_epoch)
        .execute(&mut **transaction)
        .await?;
    Ok(result.rows_affected())
}

async fn write_postgres(
    transaction: &mut Transaction<'_, Postgres>,
    heartbeat: &StatusHeartbeat,
    received_at: i64,
    retained: bool,
) -> Result<u64, StorageError> {
    let adapters = serde_json::to_value(&heartbeat.adapters).map_err(StorageError::EncodeRecord)?;
    let statement = if retained {
        "INSERT INTO edge_node_status(\
         edge_node_id,ledger_epoch,boot_id,status_seq,collector_state,adapters_json,\
         accepted_through,pending_publications,storage_pressure,received_at,last_live_received_at,pending_since_at\
         ) SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12 WHERE EXISTS(\
         SELECT 1 FROM edge_node_activations WHERE edge_node_id=$13 AND ledger_epoch=$14 AND state='active'\
         ) ON CONFLICT(edge_node_id) DO NOTHING"
    } else {
        "INSERT INTO edge_node_status(\
         edge_node_id,ledger_epoch,boot_id,status_seq,collector_state,adapters_json,\
         accepted_through,pending_publications,storage_pressure,received_at,last_live_received_at,pending_since_at\
         ) SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12 WHERE EXISTS(\
         SELECT 1 FROM edge_node_activations WHERE edge_node_id=$13 AND ledger_epoch=$14 AND state='active'\
         ) ON CONFLICT(edge_node_id) DO UPDATE SET \
         ledger_epoch=excluded.ledger_epoch,boot_id=excluded.boot_id,status_seq=excluded.status_seq,\
         collector_state=excluded.collector_state,adapters_json=excluded.adapters_json,\
         accepted_through=excluded.accepted_through,pending_publications=excluded.pending_publications,\
         storage_pressure=excluded.storage_pressure,received_at=excluded.received_at,\
         last_live_received_at=excluded.last_live_received_at,\
         pending_since_at=CASE WHEN excluded.pending_publications=0 THEN NULL \
           WHEN edge_node_status.boot_id<>excluded.boot_id \
             OR edge_node_status.ledger_epoch<>excluded.ledger_epoch \
             OR edge_node_status.pending_publications=0 \
             OR edge_node_status.pending_since_at IS NULL \
             OR (edge_node_status.ledger_epoch=excluded.ledger_epoch \
                 AND excluded.accepted_through>edge_node_status.accepted_through) THEN excluded.received_at \
           ELSE edge_node_status.pending_since_at END WHERE \
         edge_node_status.boot_id<>excluded.boot_id OR edge_node_status.status_seq<excluded.status_seq"
    };
    let result = sqlx::query(statement)
        .bind(&heartbeat.edge_node_id)
        .bind(&heartbeat.ledger_epoch)
        .bind(&heartbeat.boot_id)
        .bind(heartbeat.status_seq as i64)
        .bind(collector_state_text(heartbeat.collector_state))
        .bind(adapters)
        .bind(heartbeat.accepted_through)
        .bind(heartbeat.pending_publications)
        .bind(heartbeat.storage_pressure)
        .bind(received_at)
        .bind((!retained).then_some(received_at))
        .bind((!retained && heartbeat.pending_publications > 0).then_some(received_at))
        .bind(&heartbeat.edge_node_id)
        .bind(&heartbeat.ledger_epoch)
        .execute(&mut **transaction)
        .await?;
    Ok(result.rows_affected())
}

fn decode_sqlite(row: sqlx::sqlite::SqliteRow) -> Result<EdgeNodeStatus, StorageError> {
    let adapters_json: Vec<u8> = row.try_get("adapters_json")?;
    decode_status(
        row.try_get("edge_node_id")?,
        row.try_get("ledger_epoch")?,
        row.try_get("boot_id")?,
        row.try_get("status_seq")?,
        row.try_get("collector_state")?,
        serde_json::from_slice(&adapters_json).map_err(StorageError::EncodeRecord)?,
        row.try_get("accepted_through")?,
        row.try_get("pending_publications")?,
        row.try_get("storage_pressure")?,
        row.try_get("received_at")?,
        row.try_get("last_live_received_at")?,
        row.try_get("pending_since_at")?,
    )
}

fn decode_postgres(row: sqlx::postgres::PgRow) -> Result<EdgeNodeStatus, StorageError> {
    let adapters: Vec<StatusAdapter> = serde_json::from_value(row.try_get("adapters_json")?)
        .map_err(StorageError::EncodeRecord)?;
    decode_status(
        row.try_get("edge_node_id")?,
        row.try_get("ledger_epoch")?,
        row.try_get("boot_id")?,
        row.try_get("status_seq")?,
        row.try_get("collector_state")?,
        adapters,
        row.try_get("accepted_through")?,
        row.try_get("pending_publications")?,
        row.try_get("storage_pressure")?,
        row.try_get("received_at")?,
        row.try_get("last_live_received_at")?,
        row.try_get("pending_since_at")?,
    )
}

#[allow(clippy::too_many_arguments)]
fn decode_status(
    edge_node_id: String,
    ledger_epoch: String,
    boot_id: String,
    status_seq: i64,
    collector_state: String,
    adapters: Vec<StatusAdapter>,
    accepted_through: i64,
    pending_publications: i64,
    storage_pressure: bool,
    received_at: i64,
    last_live_received_at: Option<i64>,
    pending_since_at: Option<i64>,
) -> Result<EdgeNodeStatus, StorageError> {
    let status_seq = u64::try_from(status_seq).map_err(|_| {
        StorageError::InvalidRecord("database contains invalid status sequence".into())
    })?;
    let collector_state = match collector_state.as_str() {
        "running" => CollectorState::Running,
        "stopped" => CollectorState::Stopped,
        _ => {
            return Err(StorageError::InvalidRecord(
                "database contains invalid collector state".into(),
            ));
        }
    };
    let heartbeat = StatusHeartbeat {
        schema_version: 1,
        edge_node_id: edge_node_id.clone(),
        ledger_epoch: ledger_epoch.clone(),
        boot_id: boot_id.clone(),
        status_seq,
        collector_state,
        adapters: adapters.clone(),
        accepted_through,
        pending_publications,
        storage_pressure,
    };
    heartbeat
        .validate()
        .map_err(|error| StorageError::InvalidRecord(error.to_string()))?;
    if received_at < 0
        || last_live_received_at.is_some_and(|value| value > received_at)
        || pending_since_at.is_some_and(|value| value > received_at)
    {
        return Err(StorageError::InvalidRecord(
            "database contains invalid status receipt time".into(),
        ));
    }
    Ok(EdgeNodeStatus {
        edge_node_id,
        ledger_epoch,
        boot_id,
        status_seq,
        collector_state,
        adapters,
        accepted_through,
        pending_publications,
        storage_pressure,
        received_at,
        last_live_received_at,
        pending_since_at,
    })
}

fn collector_state_text(state: CollectorState) -> &'static str {
    match state {
        CollectorState::Running => "running",
        CollectorState::Stopped => "stopped",
    }
}
