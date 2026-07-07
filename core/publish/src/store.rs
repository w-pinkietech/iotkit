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
mod tests {
    use super::*;

    #[test]
    fn enqueue_and_select_batch_is_ordered_and_exclusive() {
        let conn = crate::tests_support::open();
        let e = "epoch-A";
        let s1 = enqueue_measurement(&conn, e, 100, 1).unwrap();
        let s2 = enqueue_measurement(&conn, e, 101, 2).unwrap();
        let _s3 = enqueue_measurement(&conn, e, 102, 3).unwrap();
        assert!(s2 > s1);
        let batch = select_batch(&conn, e, s1, 10).unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].pub_seq, s2);
        assert_eq!(batch[0].reading_seq, Some(101));
    }

    #[test]
    fn enqueue_annotation_idempotent_on_epoch_subtype() {
        let conn = crate::tests_support::open();
        let a = enqueue_annotation(&conn, "epoch-A", "epoch_start", "{}", 1).unwrap();
        assert!(a.is_some());
        let b = enqueue_annotation(&conn, "epoch-A", "epoch_start", "{}", 2).unwrap();
        assert!(b.is_none(), "二重 enqueue は UNIQUE で None");
    }

    #[test]
    fn prune_outbox_by_reading_seqs_removes_only_matching_measurements() {
        let conn = crate::tests_support::open();
        let e = "epoch-A";
        enqueue_measurement(&conn, e, 200, 1).unwrap();
        let keep = enqueue_measurement(&conn, e, 201, 2).unwrap();
        enqueue_annotation(&conn, e, "epoch_start", "{}", 3).unwrap();

        assert_eq!(prune_outbox_by_reading_seqs(&conn, &[200, 999]).unwrap(), 1);

        let batch = select_batch(&conn, e, 0, 10).unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].pub_seq, keep);
        assert_eq!(batch[0].reading_seq, Some(201));
        assert_eq!(batch[1].kind, "annotation");
    }

    #[test]
    fn prune_outbox_by_reading_seqs_empty_slice_deletes_zero_rows() {
        let conn = crate::tests_support::open();

        assert_eq!(prune_outbox_by_reading_seqs(&conn, &[]).unwrap(), 0);
    }

    #[test]
    fn prune_outbox_for_quarantined_range_removes_only_quarantined_readings_in_window() {
        let conn = crate::tests_support::open();
        let e = "epoch-A";
        let system_id = vec![1_u8; 16];
        conn.execute(
            "INSERT INTO devices (system_id, hardware_id, kind, state, created_at)
             VALUES (?1, 'hw:test', 'individual', 'active', 1)",
            params![&system_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO series
                (series_id, system_id, measurement_key, channel_index, variant, created_at)
             VALUES
                (10, ?1, 'temperature', -1, 'primary', 1),
                (20, ?1, 'humidity', -1, 'primary', 1)",
            params![&system_id],
        )
        .unwrap();
        let matching = iotkit_core_timeseries::insert_reading_v3(
            &conn,
            &iotkit_core_timeseries::NewReading {
                series_id: 10,
                received_at_ms: 1_200,
                device_time_ms: None,
                time_source: "gateway".into(),
                values: vec![1.0],
                rssi: None,
                battery_pct: None,
                quarantined: false,
            },
        )
        .unwrap();
        let outside_range = iotkit_core_timeseries::insert_reading_v3(
            &conn,
            &iotkit_core_timeseries::NewReading {
                series_id: 10,
                received_at_ms: 2_200,
                device_time_ms: None,
                time_source: "gateway".into(),
                values: vec![2.0],
                rssi: None,
                battery_pct: None,
                quarantined: false,
            },
        )
        .unwrap();
        let other_series = iotkit_core_timeseries::insert_reading_v3(
            &conn,
            &iotkit_core_timeseries::NewReading {
                series_id: 20,
                received_at_ms: 1_300,
                device_time_ms: None,
                time_source: "gateway".into(),
                values: vec![3.0],
                rssi: None,
                battery_pct: None,
                quarantined: false,
            },
        )
        .unwrap();
        enqueue_measurement(&conn, e, matching, 1).unwrap();
        let keep_outside_range = enqueue_measurement(&conn, e, outside_range, 2).unwrap();
        let keep_other_series = enqueue_measurement(&conn, e, other_series, 3).unwrap();
        conn.execute(
            "UPDATE readings SET quarantined = 1
             WHERE series_id = ?1 AND received_at BETWEEN ?2 AND ?3",
            params![10, 1_000, 2_000],
        )
        .unwrap();

        assert_eq!(
            prune_outbox_for_quarantined_range(&conn, &[10], 1_000, 2_000).unwrap(),
            1
        );

        let batch = select_batch(&conn, e, 0, 10).unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].pub_seq, keep_outside_range);
        assert_eq!(batch[0].reading_seq, Some(outside_range));
        assert_eq!(batch[1].pub_seq, keep_other_series);
        assert_eq!(batch[1].reading_seq, Some(other_series));
    }

    #[test]
    fn prune_acked_outbox_removes_up_to_cursor_in_epoch_only() {
        let conn = crate::tests_support::open();
        let s1 = enqueue_measurement(&conn, "E", 300, 1).unwrap();
        let s2 = enqueue_measurement(&conn, "E", 301, 2).unwrap();
        enqueue_measurement(&conn, "OTHER", 302, 3).unwrap();

        assert_eq!(prune_acked_outbox(&conn, "E", s1).unwrap(), 1);

        let current = select_batch(&conn, "E", 0, 10).unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].pub_seq, s2);
        assert_eq!(select_batch(&conn, "OTHER", 0, 10).unwrap().len(), 1);
    }

    #[test]
    fn target_crud_and_cursor_update_round_trip() {
        let conn = crate::tests_support::open();
        assert_eq!(target_count(&conn).unwrap(), 0);
        let t = TargetRow {
            target_id: "target-1".into(),
            endpoint_url: "https://example.test/push".into(),
            credential_token: "token-a".into(),
            archive_responsible: true,
            schema_version: 1,
            cursor_epoch: None,
            cursor_pub_seq: 0,
        };

        target_insert(&conn, &t, 10).unwrap();
        assert_eq!(target_count(&conn).unwrap(), 1);
        assert!(archive_target_registered(&conn).unwrap());

        target_set_token(&conn, "target-1", "token-b").unwrap();
        target_set_archive_responsible(&conn, "target-1", false).unwrap();
        target_advance_cursor(&conn, "target-1", "epoch-B", 42).unwrap();
        let got = target_get(&conn).unwrap().unwrap();
        assert_eq!(got.target_id, "target-1");
        assert_eq!(got.credential_token, "token-b");
        assert!(!got.archive_responsible);
        assert_eq!(got.cursor_epoch.as_deref(), Some("epoch-B"));
        assert_eq!(got.cursor_pub_seq, 42);
        assert!(!archive_target_registered(&conn).unwrap());

        target_delete(&conn, "target-1").unwrap();
        assert_eq!(target_get(&conn).unwrap().map(|t| t.target_id), None);
    }

    #[test]
    fn has_unacked_true_when_cursor_behind_same_epoch() {
        let conn = crate::tests_support::open();
        let s = enqueue_measurement(&conn, "E", 500, 1).unwrap();
        let t = TargetRow {
            target_id: "t".into(),
            endpoint_url: "https://x".into(),
            credential_token: "k".into(),
            archive_responsible: true,
            schema_version: 1,
            cursor_epoch: Some("E".into()),
            cursor_pub_seq: s - 1,
        };
        assert!(has_unacked_pubseq_rows(&conn, "E", &t, &[500]).unwrap());
    }

    #[test]
    fn has_unacked_false_when_cursor_epoch_mismatch_means_effective_zero_but_no_current_epoch_rows()
    {
        let conn = crate::tests_support::open();
        enqueue_measurement(&conn, "OLD", 500, 1).unwrap();
        let t = TargetRow {
            target_id: "t".into(),
            endpoint_url: "https://x".into(),
            credential_token: "k".into(),
            archive_responsible: true,
            schema_version: 1,
            cursor_epoch: Some("OLD".into()),
            cursor_pub_seq: 9999,
        };
        assert!(!has_unacked_pubseq_rows(&conn, "NEW", &t, &[500]).unwrap());
    }

    #[test]
    fn has_unacked_pubseq_rows_empty_reading_seqs_returns_false() {
        let conn = crate::tests_support::open();
        let t = TargetRow {
            target_id: "t".into(),
            endpoint_url: "https://x".into(),
            credential_token: "k".into(),
            archive_responsible: true,
            schema_version: 1,
            cursor_epoch: Some("E".into()),
            cursor_pub_seq: 0,
        };

        assert!(!has_unacked_pubseq_rows(&conn, "E", &t, &[]).unwrap());
    }

    #[test]
    fn any_unacked_for_target_uses_effective_cursor_for_current_epoch() {
        let conn = crate::tests_support::open();
        let s1 = enqueue_measurement(&conn, "E", 600, 1).unwrap();
        let _s2 = enqueue_measurement(&conn, "E", 601, 2).unwrap();
        let current = TargetRow {
            target_id: "t".into(),
            endpoint_url: "https://x".into(),
            credential_token: "k".into(),
            archive_responsible: true,
            schema_version: 1,
            cursor_epoch: Some("E".into()),
            cursor_pub_seq: s1,
        };
        assert!(any_unacked_for_target(&conn, "E", &current).unwrap());

        let all_acked = TargetRow {
            cursor_pub_seq: i64::MAX,
            ..current.clone()
        };
        assert!(!any_unacked_for_target(&conn, "E", &all_acked).unwrap());

        let old_epoch_cursor = TargetRow {
            cursor_epoch: Some("OLD".into()),
            cursor_pub_seq: i64::MAX,
            ..current
        };
        assert!(any_unacked_for_target(&conn, "E", &old_epoch_cursor).unwrap());
    }

    #[test]
    fn outbox_backlog_count_uses_effective_cursor_for_current_epoch() {
        let conn = crate::tests_support::open();
        let s1 = enqueue_measurement(&conn, "E", 700, 1).unwrap();
        let s2 = enqueue_measurement(&conn, "E", 701, 2).unwrap();
        enqueue_measurement(&conn, "OTHER", 702, 3).unwrap();
        let current = TargetRow {
            target_id: "t".into(),
            endpoint_url: "https://x".into(),
            credential_token: "k".into(),
            archive_responsible: true,
            schema_version: 1,
            cursor_epoch: Some("E".into()),
            cursor_pub_seq: s1,
        };

        assert_eq!(outbox_backlog_count(&conn, "E", &current).unwrap(), 1);

        let all_acked = TargetRow {
            cursor_pub_seq: s2,
            ..current.clone()
        };
        assert_eq!(outbox_backlog_count(&conn, "E", &all_acked).unwrap(), 0);

        let old_epoch_cursor = TargetRow {
            cursor_epoch: Some("OLD".into()),
            cursor_pub_seq: i64::MAX,
            ..current
        };
        assert_eq!(
            outbox_backlog_count(&conn, "E", &old_epoch_cursor).unwrap(),
            2
        );
    }
}
