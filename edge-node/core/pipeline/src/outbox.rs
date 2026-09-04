//! Publication outbox. Inserts share the caller's transaction; the MQTT Output
//! Adapter reads the oldest row, publishes it, and deletes it after PUBACK.

use iotkit_core_types::PipelineId;
use rusqlite::{Connection, OptionalExtension, params};

use crate::wire::InputTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxRow {
    pub outbox_seq: i64,
    pub pipeline_id: String,
    pub topic: String,
    pub payload: Vec<u8>,
    pub retain: bool,
    pub created_at: InputTime,
}

pub fn enqueue(
    conn: &Connection,
    pipeline_id: &PipelineId,
    topic: &str,
    payload: &[u8],
    retain: bool,
    at: InputTime,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO observation_outbox
            (pipeline_id, topic, payload, retain, created_uptime_ms, created_unix_epoch_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            pipeline_id.as_str(),
            topic,
            payload,
            retain,
            at.uptime_ms,
            at.unix_epoch_ms
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn oldest(conn: &Connection) -> rusqlite::Result<Option<OutboxRow>> {
    conn.query_row(
        "SELECT outbox_seq, pipeline_id, topic, payload, retain, created_uptime_ms, created_unix_epoch_ms
         FROM observation_outbox ORDER BY outbox_seq ASC LIMIT 1",
        [],
        row_to_outbox,
    )
    .optional()
}

pub fn delete(conn: &Connection, outbox_seq: i64) -> rusqlite::Result<bool> {
    let deleted = conn.execute(
        "DELETE FROM observation_outbox WHERE outbox_seq = ?1",
        [outbox_seq],
    )?;
    Ok(deleted == 1)
}

pub fn count(conn: &Connection) -> rusqlite::Result<u64> {
    conn.query_row("SELECT COUNT(*) FROM observation_outbox", [], |row| {
        row.get::<_, i64>(0).map(|count| count as u64)
    })
}

pub fn all(conn: &Connection) -> rusqlite::Result<Vec<OutboxRow>> {
    let mut statement = conn.prepare(
        "SELECT outbox_seq, pipeline_id, topic, payload, retain, created_uptime_ms, created_unix_epoch_ms
         FROM observation_outbox ORDER BY outbox_seq ASC",
    )?;
    let rows = statement.query_map([], row_to_outbox)?;
    rows.collect()
}

fn row_to_outbox(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxRow> {
    Ok(OutboxRow {
        outbox_seq: row.get(0)?,
        pipeline_id: row.get(1)?,
        topic: row.get(2)?,
        payload: row.get(3)?,
        retain: row.get(4)?,
        created_at: InputTime {
            uptime_ms: row.get(5)?,
            unix_epoch_ms: row.get(6)?,
        },
    })
}
