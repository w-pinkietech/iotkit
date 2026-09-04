//! SQLite persistence of pipeline definitions and evaluation state.

use iotkit_core_types::PipelineId;
use rusqlite::{Connection, OptionalExtension, params};

use iotkit_core_types::EdgeNodeId;

use crate::definition::PipelineDefinition;
use crate::evaluator::EvaluationState;
use crate::wire::ObservationValue;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored pipeline definition {0} is not readable: {1}")]
    CorruptDefinition(String, String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PipelineState {
    pub structural_hash: String,
    pub series_id: String,
    pub next_sequence: u64,
    pub evaluation: EvaluationState,
    pub last_value: Option<ObservationValue>,
    pub last_timestamp: Option<i64>,
}

pub fn insert_definition(
    conn: &Connection,
    definition: &PipelineDefinition,
    now: i64,
) -> Result<(), StoreError> {
    let json = serde_json::to_string(definition).expect("definition serializes");
    conn.execute(
        "INSERT INTO pipeline_definition
            (pipeline_id, definition_json, structural_hash, input_adapter, input_subject,
             input_measurement_key, input_channel_index, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            definition.id.as_str(),
            json,
            definition.structural_hash(),
            definition.input.adapter,
            definition.input.subject,
            definition.input.measurement_key,
            definition.input.channel_index,
            now,
        ],
    )?;
    Ok(())
}

pub fn update_definition(
    conn: &Connection,
    definition: &PipelineDefinition,
    now: i64,
) -> Result<bool, StoreError> {
    let json = serde_json::to_string(definition).expect("definition serializes");
    let updated = conn.execute(
        "UPDATE pipeline_definition
         SET definition_json = ?2, structural_hash = ?3, input_adapter = ?4, input_subject = ?5,
             input_measurement_key = ?6, input_channel_index = ?7, updated_at = ?8
         WHERE pipeline_id = ?1",
        params![
            definition.id.as_str(),
            json,
            definition.structural_hash(),
            definition.input.adapter,
            definition.input.subject,
            definition.input.measurement_key,
            definition.input.channel_index,
            now,
        ],
    )?;
    Ok(updated == 1)
}

pub fn delete_definition(conn: &Connection, id: &PipelineId) -> Result<bool, StoreError> {
    // pipeline_state cascades through the foreign key.
    let deleted = conn.execute(
        "DELETE FROM pipeline_definition WHERE pipeline_id = ?1",
        [id.as_str()],
    )?;
    Ok(deleted == 1)
}

pub fn get_definition(
    conn: &Connection,
    id: &PipelineId,
) -> Result<Option<PipelineDefinition>, StoreError> {
    let json: Option<String> = conn
        .query_row(
            "SELECT definition_json FROM pipeline_definition WHERE pipeline_id = ?1",
            [id.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    json.map(|json| decode_definition(id.as_str(), &json))
        .transpose()
}

pub fn list_definitions(conn: &Connection) -> Result<Vec<PipelineDefinition>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT pipeline_id, definition_json FROM pipeline_definition ORDER BY pipeline_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    rows.map(|row| {
        let (id, json) = row?;
        decode_definition(&id, &json)
    })
    .collect()
}

/// Definitions whose input matches an accepted reading. `subject` is matched
/// only by definitions that name one.
pub fn definitions_for_input(
    conn: &Connection,
    adapter: &str,
    subject: Option<&str>,
    measurement_key: &str,
    channel_index: Option<u16>,
) -> Result<Vec<PipelineDefinition>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT pipeline_id, definition_json FROM pipeline_definition
         WHERE input_adapter = ?1 AND input_measurement_key = ?2
           AND input_channel_index IS ?3
           AND (input_subject IS NULL OR input_subject IS ?4)
         ORDER BY pipeline_id",
    )?;
    let rows = statement.query_map(
        params![adapter, measurement_key, channel_index, subject],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;
    rows.map(|row| {
        let (id, json) = row?;
        decode_definition(&id, &json)
    })
    .collect()
}

fn decode_definition(id: &str, json: &str) -> Result<PipelineDefinition, StoreError> {
    serde_json::from_str(json)
        .map_err(|error| StoreError::CorruptDefinition(id.to_owned(), error.to_string()))
}

pub fn get_state(conn: &Connection, id: &PipelineId) -> Result<Option<PipelineState>, StoreError> {
    conn.query_row(
        "SELECT structural_hash, series_id, next_sequence, initialized, active, counter,
                pending, pending_active, pending_since, last_value_json, last_timestamp
         FROM pipeline_state WHERE pipeline_id = ?1",
        [id.as_str()],
        |row| {
            let last_value_json: Option<String> = row.get(9)?;
            Ok(PipelineState {
                structural_hash: row.get(0)?,
                series_id: row.get(1)?,
                next_sequence: row.get::<_, i64>(2)? as u64,
                evaluation: EvaluationState {
                    initialized: row.get(3)?,
                    active: row.get(4)?,
                    counter: row.get(5)?,
                    pending: row.get(6)?,
                    pending_active: row.get(7)?,
                    pending_since: row.get(8)?,
                },
                last_value: last_value_json.as_deref().and_then(decode_value),
                last_timestamp: row.get(10)?,
            })
        },
    )
    .optional()
    .map_err(StoreError::from)
}

pub fn put_state(
    conn: &Connection,
    id: &PipelineId,
    state: &PipelineState,
    now: i64,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO pipeline_state
            (pipeline_id, structural_hash, series_id, next_sequence, initialized, active, counter,
             pending, pending_active, pending_since, last_value_json, last_timestamp, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(pipeline_id) DO UPDATE SET
            structural_hash = excluded.structural_hash,
            series_id = excluded.series_id,
            next_sequence = excluded.next_sequence,
            initialized = excluded.initialized,
            active = excluded.active,
            counter = excluded.counter,
            pending = excluded.pending,
            pending_active = excluded.pending_active,
            pending_since = excluded.pending_since,
            last_value_json = excluded.last_value_json,
            last_timestamp = excluded.last_timestamp,
            updated_at = excluded.updated_at",
        params![
            id.as_str(),
            state.structural_hash,
            state.series_id,
            state.next_sequence as i64,
            state.evaluation.initialized,
            state.evaluation.active,
            state.evaluation.counter,
            state.evaluation.pending,
            state.evaluation.pending_active,
            state.evaluation.pending_since,
            state.last_value.map(encode_value),
            state.last_timestamp,
            now,
        ],
    )?;
    Ok(())
}

pub fn list_states(conn: &Connection) -> Result<Vec<(PipelineId, PipelineState)>, StoreError> {
    let ids: Vec<String> = conn
        .prepare("SELECT pipeline_id FROM pipeline_state ORDER BY pipeline_id")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let mut states = Vec::with_capacity(ids.len());
    for id in ids {
        let Ok(id) = PipelineId::parse(id.as_str()) else {
            continue;
        };
        if let Some(state) = get_state(conn, &id)? {
            states.push((id, state));
        }
    }
    Ok(states)
}

fn encode_value(value: ObservationValue) -> String {
    value.to_json().to_string()
}

fn decode_value(json: &str) -> Option<ObservationValue> {
    match serde_json::from_str::<serde_json::Value>(json).ok()? {
        serde_json::Value::Bool(value) => Some(ObservationValue::State(value)),
        // Only measurement needs the last published value (change detection);
        // accumulated-count reads its current value from `counter`.
        serde_json::Value::Number(number) => number.as_f64().map(ObservationValue::Measurement),
        _ => None,
    }
}

const META_EDGE_NODE_ID: &str = "edge_node_id";

pub fn put_edge_node_id(conn: &Connection, edge_node_id: &EdgeNodeId) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO pipeline_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![META_EDGE_NODE_ID, edge_node_id.as_str()],
    )?;
    Ok(())
}

pub fn get_edge_node_id(conn: &Connection) -> Result<Option<EdgeNodeId>, StoreError> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM pipeline_meta WHERE key = ?1",
            [META_EDGE_NODE_ID],
            |row| row.get(0),
        )
        .optional()?;
    value
        .map(|value| {
            EdgeNodeId::parse(value.as_str()).map_err(|error| {
                StoreError::CorruptDefinition(META_EDGE_NODE_ID.into(), error.to_string())
            })
        })
        .transpose()
}
