use crate::TimeseriesError;
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use std::borrow::Cow;

pub struct ReadingRowV3 {
    pub seq: i64,
    pub series_id: i64,
    pub event_time: i64,
    pub event_time_source: String,
    pub received_at: i64,
    pub device_time: Option<i64>,
    pub time_source: String,
    pub time_quality: String,
    pub values: Vec<f64>,
    pub rssi: Option<i16>,
    pub battery_pct: Option<u8>,
    pub quarantined: bool,
}

pub struct Bucket {
    pub bucket_start: i64,
    pub count: i64,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
}

fn row_to_reading(row: &rusqlite::Row<'_>) -> Result<ReadingRowV3, rusqlite::Error> {
    let values_json: String = row.get(8)?;
    let values = serde_json::from_str(&values_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            8,
            rusqlite::types::Type::Text,
            Box::new(e),
        )
    })?;
    Ok(ReadingRowV3 {
        seq: row.get(0)?,
        series_id: row.get(1)?,
        received_at: row.get(2)?,
        device_time: row.get(3)?,
        time_source: row.get(4)?,
        time_quality: row.get(5)?,
        event_time: row.get(6)?,
        event_time_source: row.get(7)?,
        values,
        rssi: row.get(9)?,
        battery_pct: row.get(10)?,
        quarantined: row.get::<_, i32>(11)? != 0,
    })
}

const READING_COLS: &str = "seq, series_id, received_at, device_time, time_source, \
    time_quality, event_time, event_time_source, values_json, rssi, battery_pct, quarantined";

pub fn query_readings_v3(
    conn: &Connection,
    series_id: i64,
    from_event_ms: i64,
    to_event_ms: i64,
    limit: u32,
    include_quarantined: bool,
) -> Result<Vec<ReadingRowV3>, TimeseriesError> {
    let sql = if include_quarantined {
        format!(
            "SELECT {READING_COLS} FROM readings
             WHERE series_id = ?1 AND event_time >= ?2 AND event_time < ?3
             ORDER BY event_time ASC, seq ASC LIMIT ?4"
        )
    } else {
        format!(
            "SELECT {READING_COLS} FROM readings
             WHERE series_id = ?1 AND event_time >= ?2 AND event_time < ?3 AND quarantined = 0
             ORDER BY event_time ASC, seq ASC LIMIT ?4"
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    stmt.query_map(params![series_id, from_event_ms, to_event_ms, limit], row_to_reading)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(TimeseriesError::from)
}

pub fn aggregate_readings_v3(
    conn: &Connection,
    series_id: i64,
    from_event_ms: i64,
    to_event_ms: i64,
    bucket_ms: i64,
    include_quarantined: bool,
) -> Result<Vec<Bucket>, TimeseriesError> {
    if bucket_ms <= 0 {
        return Err(TimeseriesError::InvalidReading(format!(
            "bucket_ms must be positive: {bucket_ms}"
        )));
    }

    let non_scalar_sql = if include_quarantined {
        "SELECT COUNT(*) FROM readings
         WHERE series_id = ?1 AND event_time >= ?2 AND event_time < ?3
           AND COALESCE(json_array_length(values_json), -1) != 1"
    } else {
        "SELECT COUNT(*) FROM readings
         WHERE series_id = ?1 AND event_time >= ?2 AND event_time < ?3
           AND quarantined = 0
           AND COALESCE(json_array_length(values_json), -1) != 1"
    };
    let non_scalar_count: i64 = conn.query_row(
        non_scalar_sql,
        params![series_id, from_event_ms, to_event_ms],
        |row| row.get(0),
    )?;
    if non_scalar_count > 0 {
        return Err(TimeseriesError::InvalidReading(format!(
            "series {series_id} has {non_scalar_count} non-scalar readings"
        )));
    }

    let aggregate_sql = if include_quarantined {
        "SELECT ?2 + ((event_time - ?2) / ?4) * ?4 AS bucket_start,
                COUNT(*),
                MIN(json_extract(values_json, '$[0]')),
                MAX(json_extract(values_json, '$[0]')),
                AVG(json_extract(values_json, '$[0]'))
         FROM readings
         WHERE series_id = ?1 AND event_time >= ?2 AND event_time < ?3
         GROUP BY bucket_start ORDER BY bucket_start"
    } else {
        "SELECT ?2 + ((event_time - ?2) / ?4) * ?4 AS bucket_start,
                COUNT(*),
                MIN(json_extract(values_json, '$[0]')),
                MAX(json_extract(values_json, '$[0]')),
                AVG(json_extract(values_json, '$[0]'))
         FROM readings
         WHERE series_id = ?1 AND event_time >= ?2 AND event_time < ?3 AND quarantined = 0
         GROUP BY bucket_start ORDER BY bucket_start"
    };
    let mut stmt = conn.prepare(aggregate_sql)?;
    stmt.query_map(
        params![series_id, from_event_ms, to_event_ms, bucket_ms],
        |row| {
            Ok(Bucket {
                bucket_start: row.get(0)?,
                count: row.get(1)?,
                min: row.get(2)?,
                max: row.get(3)?,
                avg: row.get(4)?,
            })
        },
    )?
    .collect::<Result<Vec<_>, _>>()
    .map_err(TimeseriesError::from)
}

fn csv_field(s: &str) -> Cow<'_, str> {
    if s.contains([',', '"', '\n', '\r']) {
        Cow::Owned(format!("\"{}\"", s.replace('"', "\"\"")))
    } else {
        Cow::Borrowed(s)
    }
}

pub fn export_csv<W: std::io::Write>(w: &mut W, rows: &[ReadingRowV3]) -> std::io::Result<()> {
    let max_values = rows.iter().map(|r| r.values.len()).max().unwrap_or(0);
    write!(
        w,
        "seq,event_time,event_time_source,received_at,device_time,time_source,time_quality,quarantined,rssi,battery_pct"
    )?;
    for i in 0..max_values {
        write!(w, ",v{i}")?;
    }
    writeln!(w)?;
    for row in rows {
        write!(
            w,
            "{},{},{},{},",
            row.seq,
            row.event_time,
            csv_field(&row.event_time_source),
            row.received_at
        )?;
        if let Some(device_time) = row.device_time {
            write!(w, "{device_time}")?;
        }
        write!(
            w,
            ",{},{},{},",
            csv_field(&row.time_source),
            csv_field(&row.time_quality),
            if row.quarantined { 1 } else { 0 }
        )?;
        if let Some(rssi) = row.rssi {
            write!(w, "{rssi}")?;
        }
        write!(w, ",")?;
        if let Some(battery_pct) = row.battery_pct {
            write!(w, "{battery_pct}")?;
        }
        for value in &row.values {
            write!(w, ",{value}")?;
        }
        for _ in row.values.len()..max_values {
            write!(w, ",")?;
        }
        writeln!(w)?;
    }
    Ok(())
}

pub fn latest_by_series(
    conn: &Connection,
    series_id: i64,
) -> Result<Option<ReadingRowV3>, TimeseriesError> {
    conn.query_row(
        &format!(
            "SELECT {READING_COLS} FROM readings
             WHERE series_id = ?1
             ORDER BY event_time DESC, seq DESC LIMIT 1"
        ),
        params![series_id],
        row_to_reading,
    )
    .optional()
    .map_err(TimeseriesError::from)
}

pub fn list_staged_for_hardware(
    conn: &Connection,
    hardware_id: &str,
    limit: u32,
) -> Result<Vec<(i64, String)>, TimeseriesError> {
    let mut stmt = conn
        .prepare(
            "SELECT received_at, payload_json FROM staged_readings
             WHERE hardware_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
    stmt.query_map(params![hardware_id, limit], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(TimeseriesError::from)
}

pub fn mark_readings_quarantined(
    conn: &Connection,
    series_ids: &[i64],
    from_received_ms: i64,
    to_received_ms: i64,
) -> Result<u64, TimeseriesError> {
    if series_ids.is_empty() {
        return Ok(0);
    }
    let vars = std::iter::repeat_n("?", series_ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "UPDATE readings SET quarantined = 1
         WHERE series_id IN ({vars}) AND received_at >= ? AND received_at <= ?"
    );
    let mut values: Vec<Value> = series_ids.iter().copied().map(Value::from).collect();
    values.push(Value::from(from_received_ms));
    values.push(Value::from(to_received_ms));
    conn.execute(&sql, params_from_iter(values))
        .map(|n| n as u64)
        .map_err(TimeseriesError::from)
}
