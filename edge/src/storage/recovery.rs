use serde_json::json;

use super::{Storage, StorageError, StorageInner};

impl Storage {
    pub async fn accept_restored_archive_loss(
        &self,
        edge_node_id: &str,
        ledger_epoch: &str,
        confirmed_edge_id: &str,
        reason: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        if edge_node_id.is_empty()
            || ledger_epoch.is_empty()
            || reason.trim().is_empty()
            || reason.chars().count() > 1024
            || now < 0
        {
            return Err(StorageError::InvalidRecord(
                "archive-loss decision fields are invalid".into(),
            ));
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let edge_id: String =
                    sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
                        .fetch_one(&mut *tx)
                        .await?;
                if confirmed_edge_id != edge_id {
                    return Err(StorageError::EdgeIdentityMismatch);
                }
                let pending: Option<(String, i64)> = sqlx::query_as(
                    "SELECT checks.restore_id, checks.observed_cursor_start \
                     FROM edge_restore_cursor_checks AS checks \
                     JOIN edge_restore_events AS events ON events.restore_id=checks.restore_id \
                     WHERE checks.edge_node_id=? AND checks.ledger_epoch=? \
                       AND checks.state='recovery_required' \
                     ORDER BY events.restored_at DESC, checks.restore_id DESC LIMIT 1",
                )
                .bind(edge_node_id)
                .bind(ledger_epoch)
                .fetch_optional(&mut *tx)
                .await?;
                let (restore_id, observed_start) =
                    pending.ok_or(StorageError::NoArchiveLossDecision)?;
                apply_sqlite_loss(
                    &mut tx,
                    edge_node_id,
                    ledger_epoch,
                    &restore_id,
                    observed_start - 1,
                    reason,
                    now,
                )
                .await?;
                tx.commit().await?;
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let edge_id: String =
                    sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
                        .fetch_one(&mut *tx)
                        .await?;
                if confirmed_edge_id != edge_id {
                    return Err(StorageError::EdgeIdentityMismatch);
                }
                let pending: Option<(String, i64)> = sqlx::query_as(
                    "SELECT checks.restore_id, checks.observed_cursor_start \
                     FROM edge_restore_cursor_checks AS checks \
                     JOIN edge_restore_events AS events ON events.restore_id=checks.restore_id \
                     WHERE checks.edge_node_id=$1 AND checks.ledger_epoch=$2 \
                       AND checks.state='recovery_required' \
                     ORDER BY events.restored_at DESC, checks.restore_id DESC LIMIT 1 FOR UPDATE",
                )
                .bind(edge_node_id)
                .bind(ledger_epoch)
                .fetch_optional(&mut *tx)
                .await?;
                let (restore_id, observed_start) =
                    pending.ok_or(StorageError::NoArchiveLossDecision)?;
                apply_postgres_loss(
                    &mut tx,
                    edge_node_id,
                    ledger_epoch,
                    &restore_id,
                    observed_start - 1,
                    reason,
                    now,
                )
                .await?;
                tx.commit().await?;
            }
        }
        Ok(())
    }
}

async fn apply_sqlite_loss(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    edge_node_id: &str,
    ledger_epoch: &str,
    restore_id: &str,
    accepted_through: i64,
    reason: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO accepted_cursors(edge_node_id,ledger_epoch,accepted_through,updated_at) \
         VALUES(?,?,?,?) ON CONFLICT(edge_node_id,ledger_epoch) DO UPDATE SET \
         accepted_through=excluded.accepted_through,updated_at=excluded.updated_at",
    )
    .bind(edge_node_id)
    .bind(ledger_epoch)
    .bind(accepted_through)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE edge_restore_cursor_checks SET state='archive_lost',updated_at=? \
         WHERE restore_id=? AND edge_node_id=? AND ledger_epoch=? AND state='recovery_required'",
    )
    .bind(now)
    .bind(restore_id)
    .bind(edge_node_id)
    .bind(ledger_epoch)
    .execute(&mut **tx)
    .await?;
    let changed = sqlx::query(
        "UPDATE edge_node_activations SET state='active',revision=revision+1,updated_at=? \
         WHERE edge_node_id=? AND ledger_epoch=? AND state='recovery_hold'",
    )
    .bind(now)
    .bind(edge_node_id)
    .bind(ledger_epoch)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(StorageError::NoArchiveLossDecision);
    }
    let summary = serde_json::to_vec(&json!({
        "restore_id": restore_id, "ledger_epoch": ledger_epoch,
        "accepted_through": accepted_through, "reason": reason.trim()
    }))
    .map_err(StorageError::EncodeRecord)?;
    sqlx::query(
        "INSERT INTO audit_events(occurred_at,actor_class,actor_ref,operation,resource_ref, \
         outcome,summary_json) VALUES(?,'local_cli','local-cli', \
         'edge_restore.accept_archive_loss',?,'success',?)",
    )
    .bind(now)
    .bind(edge_node_id)
    .bind(summary)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn apply_postgres_loss(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    edge_node_id: &str,
    ledger_epoch: &str,
    restore_id: &str,
    accepted_through: i64,
    reason: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO accepted_cursors(edge_node_id,ledger_epoch,accepted_through,updated_at) \
         VALUES($1,$2,$3,$4) ON CONFLICT(edge_node_id,ledger_epoch) DO UPDATE SET \
         accepted_through=excluded.accepted_through,updated_at=excluded.updated_at",
    )
    .bind(edge_node_id)
    .bind(ledger_epoch)
    .bind(accepted_through)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE edge_restore_cursor_checks SET state='archive_lost',updated_at=$1 \
         WHERE restore_id=$2 AND edge_node_id=$3 AND ledger_epoch=$4 \
           AND state='recovery_required'",
    )
    .bind(now)
    .bind(restore_id)
    .bind(edge_node_id)
    .bind(ledger_epoch)
    .execute(&mut **tx)
    .await?;
    let changed = sqlx::query(
        "UPDATE edge_node_activations SET state='active',revision=revision+1,updated_at=$1 \
         WHERE edge_node_id=$2 AND ledger_epoch=$3 AND state='recovery_hold'",
    )
    .bind(now)
    .bind(edge_node_id)
    .bind(ledger_epoch)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(StorageError::NoArchiveLossDecision);
    }
    sqlx::query(
        "INSERT INTO audit_events(occurred_at,actor_class,actor_ref,operation,resource_ref, \
         outcome,summary_json) VALUES($1,'local_cli','local-cli', \
         'edge_restore.accept_archive_loss',$2,'success',$3)",
    )
    .bind(now)
    .bind(edge_node_id)
    .bind(json!({
        "restore_id": restore_id, "ledger_epoch": ledger_epoch,
        "accepted_through": accepted_through, "reason": reason.trim()
    }))
    .execute(&mut **tx)
    .await?;
    Ok(())
}
