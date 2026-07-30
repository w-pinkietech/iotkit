use crate::PublishError;
use rusqlite::{Connection, ErrorCode, OptionalExtension, params, params_from_iter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxRow {
    pub pub_seq: i64,
    pub epoch: String,
    pub kind: String,
    pub subtype: Option<String>,
    pub reading_seq: Option<i64>,
    pub annotation_json: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct TargetRow {
    pub target_id: String,
    pub endpoint_url: String,
    pub credential_token: String,
    pub archive_responsible: bool,
    pub schema_version: i64,
    pub cursor_epoch: Option<String>,
    pub cursor_pub_seq: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryOutboxRebuild {
    pub replayed_records: i64,
    pub last_new_publication_seq: i64,
}

impl std::fmt::Debug for TargetRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TargetRow")
            .field("target_id", &self.target_id)
            .field("endpoint_url", &self.endpoint_url)
            .field("credential_token", &"***")
            .field("archive_responsible", &self.archive_responsible)
            .field("schema_version", &self.schema_version)
            .field("cursor_epoch", &self.cursor_epoch)
            .field("cursor_pub_seq", &self.cursor_pub_seq)
            .finish()
    }
}

pub fn enqueue_measurement(
    conn: &Connection,
    epoch: &str,
    reading_seq: i64,
    now_ms: i64,
) -> Result<i64, PublishError> {
    require_publication_admitted(conn)?;
    conn.execute(
        "INSERT INTO publication_log(epoch, kind, reading_seq, created_at)
         VALUES(?1, 'measurement', ?2, ?3)",
        params![epoch, reading_seq, now_ms],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn enqueue_annotation(
    conn: &Connection,
    epoch: &str,
    subtype: &str,
    payload_json: &str,
    now_ms: i64,
) -> Result<Option<i64>, PublishError> {
    require_publication_admitted(conn)?;
    match conn.execute(
        "INSERT INTO publication_log(epoch, kind, subtype, annotation_json, created_at)
         VALUES(?1, 'annotation', ?2, ?3, ?4)",
        params![epoch, subtype, payload_json, now_ms],
    ) {
        Ok(_) => Ok(Some(conn.last_insert_rowid())),
        Err(rusqlite::Error::SqliteFailure(err, _))
            if err.code == ErrorCode::ConstraintViolation =>
        {
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

pub fn enqueue_commissioning_smoke(
    conn: &Connection,
    epoch: &str,
    test_id: &str,
    now_ms: i64,
) -> Result<i64, PublishError> {
    validate_commissioning_smoke_test_id(test_id)?;
    require_publication_admitted(conn)?;
    let payload_json = serde_json::to_string(&serde_json::json!({"test_id": test_id}))
        .map_err(|error| PublishError::Invalid(error.to_string()))?;
    conn.execute(
        "INSERT INTO publication_log(epoch, kind, annotation_json, created_at)
         VALUES(?1, 'commissioning_smoke', ?2, ?3)",
        params![epoch, payload_json, now_ms],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn validate_commissioning_smoke_test_id(test_id: &str) -> Result<(), PublishError> {
    let Some(random) = test_id.strip_prefix("smoke-") else {
        return Err(PublishError::Invalid(
            "commissioning smoke test_id must start with smoke-".into(),
        ));
    };
    if random.len() != 32
        || !random
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(PublishError::Invalid(
            "commissioning smoke test_id must contain 128-bit lowercase hex".into(),
        ));
    }
    Ok(())
}

fn require_publication_admitted(conn: &Connection) -> Result<(), PublishError> {
    if !crate::activation::publication_admitted(conn)? {
        return Err(PublishError::Invalid(
            "Edge Node activation has not admitted publication".into(),
        ));
    }
    Ok(())
}

pub fn select_batch(
    conn: &Connection,
    epoch: &str,
    after_pub_seq: i64,
    limit: u32,
) -> Result<Vec<OutboxRow>, PublishError> {
    let mut stmt = conn.prepare(
        "SELECT pub_seq, epoch, kind, subtype, reading_seq, annotation_json
         FROM publication_log
         WHERE epoch = ?1 AND pub_seq > ?2
         ORDER BY pub_seq ASC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![epoch, after_pub_seq, limit], |row| {
        Ok(OutboxRow {
            pub_seq: row.get(0)?,
            epoch: row.get(1)?,
            kind: row.get(2)?,
            subtype: row.get(3)?,
            reading_seq: row.get(4)?,
            annotation_json: row.get(5)?,
        })
    })?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(PublishError::from)
}

pub fn prune_outbox_by_reading_seqs(
    conn: &Connection,
    reading_seqs: &[i64],
) -> Result<u64, PublishError> {
    if reading_seqs.is_empty() {
        return Ok(0);
    }

    let placeholders = repeat_vars(reading_seqs.len());
    let sql = format!("DELETE FROM publication_log WHERE reading_seq IN ({placeholders})");
    let changed = conn.execute(&sql, params_from_iter(reading_seqs.iter()))?;
    Ok(changed as u64)
}

/// Prune measurement outbox rows for retroactively-quarantined readings in a series/time window (§9.2).
/// Uses a subquery over readings (series_ids small; readings via subquery) -- no host-var list explosion.
pub fn prune_outbox_for_quarantined_range(
    conn: &Connection,
    series_ids: &[i64],
    since_ms: i64,
    to_ms: i64,
) -> Result<u64, PublishError> {
    if series_ids.is_empty() {
        return Ok(0);
    }

    let placeholders = repeat_vars(series_ids.len());
    let sql = format!(
        "DELETE FROM publication_log
         WHERE reading_seq IN (
             SELECT seq FROM readings
             WHERE series_id IN ({placeholders})
               AND received_at BETWEEN ? AND ?
               AND quarantined = 1
         )"
    );
    let changed = conn.execute(
        &sql,
        params_from_iter(series_ids.iter().copied().chain([since_ms, to_ms])),
    )?;
    Ok(changed as u64)
}

pub fn prune_acked_outbox(
    conn: &Connection,
    epoch: &str,
    upto_pub_seq: i64,
) -> Result<u64, PublishError> {
    let changed = conn.execute(
        "DELETE FROM publication_log WHERE epoch = ?1 AND pub_seq <= ?2",
        params![epoch, upto_pub_seq],
    )?;
    Ok(changed as u64)
}

pub fn target_insert(conn: &Connection, t: &TargetRow, now_ms: i64) -> Result<(), PublishError> {
    conn.execute(
        "INSERT INTO target_registry(
             target_id, endpoint_url, credential_token, archive_responsible,
             schema_version, cursor_epoch, cursor_pub_seq, created_at
         )
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            t.target_id,
            t.endpoint_url,
            t.credential_token,
            t.archive_responsible,
            t.schema_version,
            t.cursor_epoch,
            t.cursor_pub_seq,
            now_ms
        ],
    )?;
    Ok(())
}

pub fn target_get(conn: &Connection) -> Result<Option<TargetRow>, PublishError> {
    conn.query_row(
        "SELECT target_id, endpoint_url, credential_token, archive_responsible,
                schema_version, cursor_epoch, cursor_pub_seq
         FROM target_registry
         LIMIT 1",
        [],
        target_from_row,
    )
    .optional()
    .map_err(PublishError::from)
}

pub fn target_count(conn: &Connection) -> Result<i64, PublishError> {
    conn.query_row("SELECT count(*) FROM target_registry", [], |row| row.get(0))
        .map_err(PublishError::from)
}

pub fn target_delete(conn: &Connection, target_id: &str) -> Result<(), PublishError> {
    conn.execute(
        "DELETE FROM target_registry WHERE target_id = ?1",
        params![target_id],
    )?;
    Ok(())
}

pub fn target_set_token(
    conn: &Connection,
    target_id: &str,
    token: &str,
) -> Result<(), PublishError> {
    conn.execute(
        "UPDATE target_registry SET credential_token = ?2 WHERE target_id = ?1",
        params![target_id, token],
    )?;
    Ok(())
}

pub fn target_set_archive_responsible(
    conn: &Connection,
    target_id: &str,
    on: bool,
) -> Result<(), PublishError> {
    conn.execute(
        "UPDATE target_registry SET archive_responsible = ?2 WHERE target_id = ?1",
        params![target_id, on],
    )?;
    Ok(())
}

pub fn target_advance_cursor(
    conn: &Connection,
    target_id: &str,
    epoch: &str,
    pub_seq: i64,
) -> Result<(), PublishError> {
    conn.execute(
        "UPDATE target_registry
         SET cursor_epoch = ?2, cursor_pub_seq = ?3
         WHERE target_id = ?1",
        params![target_id, epoch, pub_seq],
    )?;
    Ok(())
}

pub fn has_unacked_pubseq_rows(
    conn: &Connection,
    current_epoch: &str,
    target: &TargetRow,
    reading_seqs: &[i64],
) -> Result<bool, PublishError> {
    if reading_seqs.is_empty() {
        return Ok(false);
    }

    let cursor = effective_cursor(current_epoch, target);
    let placeholders = repeat_vars(reading_seqs.len());
    let sql = format!(
        "SELECT EXISTS(
             SELECT 1
             FROM publication_log
             WHERE epoch = ?1
               AND kind = 'measurement'
               AND pub_seq > ?2
               AND reading_seq IN ({placeholders})
         )"
    );

    let mut values: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(reading_seqs.len() + 2);
    values.push(&current_epoch);
    values.push(&cursor);
    for seq in reading_seqs {
        values.push(seq);
    }

    conn.query_row(&sql, values.as_slice(), |row| row.get(0))
        .map_err(PublishError::from)
}

pub fn archive_target_registered(conn: &Connection) -> Result<bool, PublishError> {
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM target_registry WHERE archive_responsible = 1",
        [],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub fn any_unacked_for_target(
    conn: &Connection,
    current_epoch: &str,
    target: &TargetRow,
) -> Result<bool, PublishError> {
    let cursor = effective_cursor(current_epoch, target);
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM publication_log
             WHERE epoch = ?1 AND pub_seq > ?2
         )",
        params![current_epoch, cursor],
        |row| row.get(0),
    )
    .map_err(PublishError::from)
}

pub fn outbox_backlog_count(
    conn: &Connection,
    current_epoch: &str,
    target: &TargetRow,
) -> Result<i64, PublishError> {
    let cursor = effective_cursor(current_epoch, target);
    conn.query_row(
        "SELECT count(*) FROM publication_log WHERE epoch = ?1 AND pub_seq > ?2",
        params![current_epoch, cursor],
        |row| row.get(0),
    )
    .map_err(PublishError::from)
}

/// Rebuilds the remaining old-epoch outbox as a fresh contiguous recovery stream.
///
/// The caller owns the surrounding Immediate transaction together with the ledger
/// epoch and durable recovery receipt changes.
pub fn rebuild_recovery_outbox(
    tx: &rusqlite::Transaction<'_>,
    old_epoch: &str,
    new_epoch: &str,
    edge_accepted_through: i64,
    now_ms: i64,
) -> Result<RecoveryOutboxRebuild, PublishError> {
    if old_epoch.is_empty()
        || new_epoch.is_empty()
        || old_epoch == new_epoch
        || old_epoch.contains(':')
        || new_epoch.contains(':')
        || old_epoch.chars().any(char::is_control)
        || new_epoch.chars().any(char::is_control)
        || edge_accepted_through < 0
        || now_ms < 0
    {
        return Err(PublishError::Invalid(
            "invalid recovery outbox boundary".into(),
        ));
    }
    let target: TargetRow = tx
        .query_row(
            "SELECT target_id, endpoint_url, credential_token, archive_responsible,
                    schema_version, cursor_epoch, cursor_pub_seq
             FROM target_registry",
            [],
            target_from_row,
        )
        .optional()?
        .ok_or_else(|| PublishError::Invalid("recovery target is missing".into()))?;
    let target_count: i64 =
        tx.query_row("SELECT count(*) FROM target_registry", [], |row| row.get(0))?;
    if target_count != 1
        || target.cursor_epoch.as_deref() != Some(old_epoch)
        || target.cursor_pub_seq < 0
        || edge_accepted_through < target.cursor_pub_seq
    {
        return Err(PublishError::Invalid(
            "recovery target cursor does not match".into(),
        ));
    }
    let foreign_epoch_rows: i64 = tx.query_row(
        "SELECT count(*) FROM publication_log WHERE epoch<>?1",
        [old_epoch],
        |row| row.get(0),
    )?;
    if foreign_epoch_rows != 0 {
        return Err(PublishError::Invalid(
            "recovery outbox contains another epoch".into(),
        ));
    }

    let carried = {
        let mut statement = tx.prepare(
            "SELECT kind,subtype,reading_seq,annotation_json,created_at
             FROM publication_log
             WHERE epoch=?1 AND pub_seq>?2
               AND NOT (kind='annotation' AND subtype='epoch_start')
             ORDER BY pub_seq",
        )?;
        statement
            .query_map(params![old_epoch, edge_accepted_through], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };

    tx.execute("DELETE FROM publication_log", [])?;
    tx.execute(
        "DELETE FROM sqlite_sequence WHERE name='publication_log'",
        [],
    )?;
    tx.execute(
        "INSERT INTO publication_log(
             epoch,kind,subtype,annotation_json,created_at
         ) VALUES(?1,'annotation','epoch_start',?2,?3)",
        params![
            new_epoch,
            serde_json::json!({"prior_epoch": old_epoch}).to_string(),
            now_ms
        ],
    )?;
    for (kind, subtype, reading_seq, annotation_json, created_at) in &carried {
        tx.execute(
            "INSERT INTO publication_log(
                 epoch,kind,subtype,reading_seq,annotation_json,created_at
             ) VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                new_epoch,
                kind,
                subtype,
                reading_seq,
                annotation_json,
                created_at
            ],
        )?;
    }
    let changed = tx.execute(
        "UPDATE target_registry
         SET cursor_epoch=?1,cursor_pub_seq=0
         WHERE target_id=?2 AND cursor_epoch=?3 AND cursor_pub_seq=?4",
        params![
            new_epoch,
            target.target_id,
            old_epoch,
            target.cursor_pub_seq
        ],
    )?;
    if changed != 1 {
        return Err(PublishError::Invalid(
            "recovery target cursor changed".into(),
        ));
    }
    let replayed_records = i64::try_from(carried.len())
        .map_err(|_| PublishError::Invalid("recovery outbox is too large".into()))?;
    Ok(RecoveryOutboxRebuild {
        replayed_records,
        last_new_publication_seq: replayed_records + 1,
    })
}

fn target_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TargetRow> {
    Ok(TargetRow {
        target_id: row.get(0)?,
        endpoint_url: row.get(1)?,
        credential_token: row.get(2)?,
        archive_responsible: row.get(3)?,
        schema_version: row.get(4)?,
        cursor_epoch: row.get(5)?,
        cursor_pub_seq: row.get(6)?,
    })
}

/// Returns the target cursor for the current epoch, or zero across epochs.
pub fn effective_cursor(current_epoch: &str, target: &TargetRow) -> i64 {
    if target.cursor_epoch.as_deref() == Some(current_epoch) {
        target.cursor_pub_seq
    } else {
        0
    }
}

fn repeat_vars(len: usize) -> String {
    std::iter::repeat_n("?", len).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
#[path = "../tests/unit/store_tests.rs"]
mod tests;
