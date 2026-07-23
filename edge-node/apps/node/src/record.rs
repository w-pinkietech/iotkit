use iotkit_core_ledger::{SystemId, series_key_of};
use rusqlite::params;

const SCHEMA_VERSION: u32 = 1;

#[derive(serde::Serialize)]
pub struct MeasurementRecord {
    pub family: &'static str,
    pub schema_version: u32,
    pub epoch: String,
    pub pub_seq: i64,
    pub series_key: String,
    pub values: Vec<f64>,
    pub event_time: i64,
    pub event_time_source: String,
    pub time_source: String,
    pub time_quality: String,
    pub received_at: i64,
    pub device_time: Option<i64>,
}

#[derive(serde::Serialize)]
pub struct AnnotationRecord {
    pub family: &'static str,
    pub schema_version: u32,
    pub epoch: String,
    pub pub_seq: i64,
    pub subtype: String,
    pub prior_epoch: String,
}

#[derive(serde::Serialize)]
pub struct CommissioningSmokeRecord {
    pub family: &'static str,
    pub schema_version: u32,
    pub epoch: String,
    pub pub_seq: i64,
    pub test_id: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CommissioningSmokePayload {
    test_id: String,
}

pub fn materialize_batch(
    conn: &rusqlite::Connection,
    rows: &[iotkit_core_publish::store::OutboxRow],
) -> Result<Vec<serde_json::Value>, String> {
    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        let v = match row.kind.as_str() {
            "measurement" => materialize_measurement(conn, row)?,
            "annotation" => materialize_annotation(row)?,
            "commissioning_smoke" => materialize_commissioning_smoke(row)?,
            _ => return Err(format!("unknown outbox kind: {}", row.kind)),
        };
        records.push(v);
    }
    Ok(records)
}

fn materialize_commissioning_smoke(
    row: &iotkit_core_publish::store::OutboxRow,
) -> Result<serde_json::Value, String> {
    let payload_json = row
        .annotation_json
        .as_deref()
        .ok_or_else(|| "commissioning smoke missing payload".to_string())?;
    let payload: CommissioningSmokePayload =
        serde_json::from_str(payload_json).map_err(|error| error.to_string())?;
    iotkit_core_publish::store::validate_commissioning_smoke_test_id(&payload.test_id)
        .map_err(|error| error.to_string())?;
    let record = CommissioningSmokeRecord {
        family: "commissioning_smoke",
        schema_version: SCHEMA_VERSION,
        epoch: row.epoch.clone(),
        pub_seq: row.pub_seq,
        test_id: payload.test_id,
    };
    serde_json::to_value(record).map_err(|error| error.to_string())
}

fn materialize_measurement(
    conn: &rusqlite::Connection,
    row: &iotkit_core_publish::store::OutboxRow,
) -> Result<serde_json::Value, String> {
    let reading_seq = row
        .reading_seq
        .ok_or_else(|| "measurement missing reading_seq".to_string())?;
    let (
        series_id,
        event_time,
        event_time_source,
        received_at,
        device_time,
        time_source,
        time_quality,
        values_json,
    ): (i64, i64, String, i64, Option<i64>, String, String, String) = conn
        .query_row(
            "SELECT series_id, event_time, event_time_source, received_at, device_time, time_source, time_quality, values_json
             FROM readings
             WHERE seq = ?1",
            params![reading_seq],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;

    let (system_id_blob, measurement_key, channel_index, variant): (Vec<u8>, String, i32, String) =
        conn.query_row(
            "SELECT system_id, measurement_key, channel_index, variant
             FROM series
             WHERE series_id = ?1",
            params![series_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|e| e.to_string())?;
    let system_id_bytes: [u8; 16] = system_id_blob
        .try_into()
        .map_err(|_| "system_id blob len".to_string())?;
    let system_id = SystemId::from_bytes(system_id_bytes);
    let values = serde_json::from_str::<Vec<f64>>(&values_json).map_err(|e| e.to_string())?;
    let series_key = series_key_of(&system_id, &measurement_key, channel_index, &variant);
    let rec = MeasurementRecord {
        family: "measurement",
        schema_version: SCHEMA_VERSION,
        epoch: row.epoch.clone(),
        pub_seq: row.pub_seq,
        series_key,
        values,
        event_time,
        event_time_source,
        time_source,
        time_quality,
        received_at,
        device_time,
    };
    serde_json::to_value(&rec).map_err(|e| e.to_string())
}

fn materialize_annotation(
    row: &iotkit_core_publish::store::OutboxRow,
) -> Result<serde_json::Value, String> {
    let annotation_json = row
        .annotation_json
        .as_deref()
        .ok_or_else(|| "annotation missing annotation_json".to_string())?;
    let payload: serde_json::Value =
        serde_json::from_str(annotation_json).map_err(|e| e.to_string())?;
    let subtype = row
        .subtype
        .clone()
        .ok_or_else(|| "annotation missing subtype".to_string())?;
    let prior_epoch = payload
        .get("prior_epoch")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "annotation missing prior_epoch".to_string())?
        .to_string();
    let rec = AnnotationRecord {
        family: "annotation",
        schema_version: SCHEMA_VERSION,
        epoch: row.epoch.clone(),
        pub_seq: row.pub_seq,
        subtype,
        prior_epoch,
    };
    serde_json::to_value(&rec).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "../tests/unit/record_tests.rs"]
mod tests;
