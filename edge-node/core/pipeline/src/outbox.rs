//! Publication outbox. Inserts share the caller's transaction; the MQTT Output
//! Adapter reads the oldest row, publishes it, and deletes it after PUBACK.

use iotkit_core_types::PipelineId;
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxRow {
    pub outbox_seq: i64,
    pub pipeline_id: String,
    pub topic: String,
    pub payload: Vec<u8>,
    pub retain: bool,
    pub created_at: i64,
}

pub fn enqueue(
    conn: &Connection,
    pipeline_id: &PipelineId,
    topic: &str,
    payload: &[u8],
    retain: bool,
    now: i64,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO observation_outbox (pipeline_id, topic, payload, retain, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![pipeline_id.as_str(), topic, payload, retain, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn oldest(conn: &Connection) -> rusqlite::Result<Option<OutboxRow>> {
    conn.query_row(
        "SELECT outbox_seq, pipeline_id, topic, payload, retain, created_at
         FROM observation_outbox ORDER BY outbox_seq ASC LIMIT 1",
        [],
        |row| {
            Ok(OutboxRow {
                outbox_seq: row.get(0)?,
                pipeline_id: row.get(1)?,
                topic: row.get(2)?,
                payload: row.get(3)?,
                retain: row.get(4)?,
                created_at: row.get(5)?,
            })
        },
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
        "SELECT outbox_seq, pipeline_id, topic, payload, retain, created_at
         FROM observation_outbox ORDER BY outbox_seq ASC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(OutboxRow {
            outbox_seq: row.get(0)?,
            pipeline_id: row.get(1)?,
            topic: row.get(2)?,
            payload: row.get(3)?,
            retain: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    rows.collect()
}
