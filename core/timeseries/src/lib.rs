//! iotkit-core-timeseries: sensor reading persistence (INSERT/query/delete).

mod error;
mod model;

pub use error::TimeseriesError;
pub use model::{ReadingRow, TimeRange};

use std::time::{SystemTime, UNIX_EPOCH};

use iotkit_core_storage::{DbHandle, Migration};
use iotkit_core_types::{AdapterId, DeviceKey, SensorType};

/// Timeseries migrations. Append to core/storage MIGRATIONS when assembling.
/// (gateway側でv3=ledgerを間に挟んで連結する。versionは昇順検証があるため
/// 1, 2, 3, 4 の順に並べて渡す必要がある。)
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 2,
        label: "timeseries",
        sql: include_str!("../migrations/0002_timeseries.sql"),
    },
    Migration {
        version: 4,
        label: "readings_v3",
        sql: include_str!("../migrations/0004_readings_v3.sql"),
    },
];

/// A new reading to be inserted into the v3 `readings` table.
/// Wave 0: `time_quality` is not settable here -- it defaults to 'unsynced' at
/// the schema level (D3 boundary: NTP state evaluation is Wave 1, the column
/// exists from day one but the value is fixed for now).
pub struct NewReading {
    pub series_id: i64,
    pub received_at_ms: i64,
    pub device_time_ms: Option<i64>,
    pub time_source: String,
    pub values: Vec<f64>,
    pub rssi: Option<i16>,
    pub battery_pct: Option<u8>,
    pub quarantined: bool,
}

/// Bound on staged_readings rows retained per hardware_id (oldest purged past this).
pub const STAGED_READINGS_CAP_PER_HW: i64 = 1000;

/// Attempt to claim (sender_id, envelope_id) in `ingest_dedup`.
/// Returns `true` if this is the first claim (proceed with ingest),
/// `false` if already claimed (duplicate -- D1 dedup key is sender-scoped).
pub fn try_claim_envelope(
    conn: &rusqlite::Connection, sender_id: &str, envelope_id: &str,
) -> Result<bool, TimeseriesError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let n = conn.execute(
        "INSERT INTO ingest_dedup (sender_id, envelope_id, received_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(sender_id, envelope_id) DO NOTHING",
        rusqlite::params![sender_id, envelope_id, now],
    ).map_err(|e| TimeseriesError::Storage(iotkit_core_storage::StorageError::Sqlite(e)))?;
    Ok(n == 1)
}

/// Insert a reading into the v3 `readings` table. Returns the monotonic `seq`.
/// Unlike v2, identical (series, time, values) tuples are NOT deduplicated --
/// dedup happens once, upstream, via `try_claim_envelope` on (sender, envelope_id).
pub fn insert_reading_v3(
    conn: &rusqlite::Connection, r: &NewReading,
) -> Result<i64, TimeseriesError> {
    for v in &r.values {
        if !v.is_finite() {
            return Err(TimeseriesError::InvalidReading(format!("non-finite value {v}")));
        }
    }
    let values_json = serde_json::to_string(&r.values)
        .map_err(|e| TimeseriesError::InvalidReading(e.to_string()))?;
    conn.execute(
        "INSERT INTO readings (series_id, received_at, device_time, time_source, values_json, rssi, battery_pct, quarantined)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            r.series_id, r.received_at_ms, r.device_time_ms, r.time_source,
            values_json, r.rssi, r.battery_pct, r.quarantined as i32
        ],
    ).map_err(|e| TimeseriesError::Storage(iotkit_core_storage::StorageError::Sqlite(e)))?;
    Ok(conn.last_insert_rowid())
}

/// Append a row to `staged_readings` (D5 path A: witnessed-but-not-yet-approved
/// device data). Bounded per hardware_id -- oldest rows beyond
/// `STAGED_READINGS_CAP_PER_HW` are purged after each insert.
pub fn insert_staged_reading(
    conn: &rusqlite::Connection, hardware_id: &str, received_at_ms: i64, payload_json: &str,
) -> Result<(), TimeseriesError> {
    conn.execute(
        "INSERT INTO staged_readings (hardware_id, received_at, payload_json) VALUES (?1, ?2, ?3)",
        rusqlite::params![hardware_id, received_at_ms, payload_json],
    ).map_err(|e| TimeseriesError::Storage(iotkit_core_storage::StorageError::Sqlite(e)))?;
    conn.execute(
        "DELETE FROM staged_readings WHERE hardware_id = ?1 AND id NOT IN (
            SELECT id FROM staged_readings WHERE hardware_id = ?1 ORDER BY id DESC LIMIT ?2)",
        rusqlite::params![hardware_id, STAGED_READINGS_CAP_PER_HW],
    ).map_err(|e| TimeseriesError::Storage(iotkit_core_storage::StorageError::Sqlite(e)))?;
    Ok(())
}

/// Delete `ingest_dedup` rows older than `cutoff_ms` (TTL 72h enforcement).
/// Returns the number of rows deleted.
pub fn purge_dedup_before(
    conn: &rusqlite::Connection, cutoff_ms: i64,
) -> Result<u64, TimeseriesError> {
    let n = conn.execute(
        "DELETE FROM ingest_dedup WHERE received_at < ?1", rusqlite::params![cutoff_ms],
    ).map_err(|e| TimeseriesError::Storage(iotkit_core_storage::StorageError::Sqlite(e)))?;
    Ok(n as u64)
}

/// Helper: convert SystemTime to unix milliseconds.
fn system_time_to_millis(t: SystemTime) -> Result<i64, TimeseriesError> {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .map_err(|_| TimeseriesError::InvalidReading("timestamp before epoch".to_string()))
}

pub async fn insert_reading(
    db: &DbHandle,
    adapter_id: &AdapterId,
    device_key: &DeviceKey,
    ingested_at: SystemTime,
    sensor_type: &SensorType,
    values: &[f64],
    rssi: Option<i16>,
    battery_pct: Option<u8>,
) -> Result<(), TimeseriesError> {
    // Validate values
    for (i, v) in values.iter().enumerate() {
        if v.is_nan() || v.is_infinite() {
            return Err(TimeseriesError::InvalidReading(format!(
                "NaN/Inf in values at index {i}"
            )));
        }
    }

    // Convert timestamp
    let millis = system_time_to_millis(ingested_at)?;

    // Serialize values to JSON
    let values_json = serde_json::to_string(values)
        .map_err(|e| TimeseriesError::InvalidReading(format!("JSON serialization failed: {e}")))?;

    // Prepare owned values for the closure
    let adapter_id_str = adapter_id.as_str().to_string();
    let device_key_str = device_key.as_str().to_string();
    let sensor_type_str = sensor_type.as_db_str().to_string();

    let inserted = db
        .with_conn(move |conn| {
            let changed = conn.execute(
                "INSERT INTO sensor_readings (adapter_id, device_key, ingested_at, sensor_type, values_json, rssi, battery_pct)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(adapter_id, device_key, ingested_at, sensor_type) DO NOTHING",
                rusqlite::params![
                    adapter_id_str,
                    device_key_str,
                    millis,
                    sensor_type_str,
                    values_json,
                    rssi,
                    battery_pct.map(|b| b as i32),
                ],
            )?;
            Ok(changed > 0)
        })
        .await?;

    if !inserted {
        tracing::warn!(
            adapter_id = adapter_id.as_str(),
            device_key = device_key.as_str(),
            ingested_at = millis,
            sensor_type = sensor_type.as_db_str(),
            "duplicate reading ignored (first-write-wins)"
        );
    }

    Ok(())
}

/// Helper: parse a row from sensor_readings into ReadingRow.
fn row_to_reading(row: &rusqlite::Row<'_>) -> Result<ReadingRow, rusqlite::Error> {
    let adapter_id: String = row.get(0)?;
    let device_key: String = row.get(1)?;
    let ingested_at: i64 = row.get(2)?;
    let sensor_type_str: String = row.get(3)?;
    let values_json: String = row.get(4)?;
    let rssi: Option<i16> = row.get(5)?;
    let battery_pct: Option<i32> = row.get(6)?;

    let sensor_type = SensorType::from_db_str(&sensor_type_str);
    let values: Vec<f64> = serde_json::from_str(&values_json)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e)))?;

    Ok(ReadingRow {
        adapter_id,
        device_key,
        ingested_at,
        sensor_type,
        values,
        rssi,
        battery_pct: battery_pct.map(|b| b as u8),
    })
}

pub async fn query_readings(
    db: &DbHandle,
    adapter_id: &AdapterId,
    device_key: &DeviceKey,
    sensor_type: Option<&SensorType>,
    range: TimeRange,
    limit: u32,
) -> Result<Vec<ReadingRow>, TimeseriesError> {
    if range.start >= range.end {
        return Err(TimeseriesError::InvalidReading(
            "start >= end in time range".to_string(),
        ));
    }

    let start_millis = system_time_to_millis(range.start)?;
    let end_millis = system_time_to_millis(range.end)?;
    let adapter_id_str = adapter_id.as_str().to_string();
    let device_key_str = device_key.as_str().to_string();
    let sensor_type_str = sensor_type.map(|st| st.as_db_str().to_string());

    let rows = db
        .with_conn(move |conn| {
            let rows = if let Some(ref st) = sensor_type_str {
                let mut stmt = conn.prepare(
                    "SELECT adapter_id, device_key, ingested_at, sensor_type, values_json, rssi, battery_pct
                     FROM sensor_readings
                     WHERE adapter_id = ?1 AND device_key = ?2 AND sensor_type = ?3
                       AND ingested_at >= ?4 AND ingested_at < ?5
                     ORDER BY ingested_at DESC, sensor_type ASC
                     LIMIT ?6",
                )?;
                stmt.query_map(
                    rusqlite::params![adapter_id_str, device_key_str, st, start_millis, end_millis, limit],
                    row_to_reading,
                )?
                .collect::<Result<Vec<_>, _>>()?
            } else {
                let mut stmt = conn.prepare(
                    "SELECT adapter_id, device_key, ingested_at, sensor_type, values_json, rssi, battery_pct
                     FROM sensor_readings
                     WHERE adapter_id = ?1 AND device_key = ?2
                       AND ingested_at >= ?3 AND ingested_at < ?4
                     ORDER BY ingested_at DESC, sensor_type ASC
                     LIMIT ?5",
                )?;
                stmt.query_map(
                    rusqlite::params![adapter_id_str, device_key_str, start_millis, end_millis, limit],
                    row_to_reading,
                )?
                .collect::<Result<Vec<_>, _>>()?
            };
            Ok(rows)
        })
        .await?;

    Ok(rows)
}

pub async fn latest_reading(
    db: &DbHandle,
    adapter_id: &AdapterId,
    device_key: &DeviceKey,
    sensor_type: Option<&SensorType>,
) -> Result<Option<ReadingRow>, TimeseriesError> {
    let adapter_id_str = adapter_id.as_str().to_string();
    let device_key_str = device_key.as_str().to_string();
    let sensor_type_str = sensor_type.map(|st| st.as_db_str().to_string());

    let row = db
        .with_conn(move |conn| {
            let row = if let Some(ref st) = sensor_type_str {
                conn.query_row(
                    "SELECT adapter_id, device_key, ingested_at, sensor_type, values_json, rssi, battery_pct
                     FROM sensor_readings
                     WHERE adapter_id = ?1 AND device_key = ?2 AND sensor_type = ?3
                     ORDER BY ingested_at DESC
                     LIMIT 1",
                    rusqlite::params![adapter_id_str, device_key_str, st],
                    row_to_reading,
                )
            } else {
                conn.query_row(
                    "SELECT adapter_id, device_key, ingested_at, sensor_type, values_json, rssi, battery_pct
                     FROM sensor_readings
                     WHERE adapter_id = ?1 AND device_key = ?2
                     ORDER BY ingested_at DESC, sensor_type ASC
                     LIMIT 1",
                    rusqlite::params![adapter_id_str, device_key_str],
                    row_to_reading,
                )
            };
            match row {
                Ok(r) => Ok(Some(r)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
        .await?;

    Ok(row)
}

pub async fn delete_before(
    db: &DbHandle,
    cutoff: SystemTime,
) -> Result<u64, TimeseriesError> {
    let cutoff_millis = system_time_to_millis(cutoff)?;

    let deleted = db
        .with_conn(move |conn| {
            let count = conn.execute(
                "DELETE FROM sensor_readings WHERE ingested_at < ?1",
                rusqlite::params![cutoff_millis],
            )?;
            Ok(count as u64)
        })
        .await?;

    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn all_migrations() -> Vec<Migration> {
        let mut m = Vec::from(iotkit_core_storage::MIGRATIONS);
        m.extend_from_slice(MIGRATIONS);
        m
    }

    fn test_db() -> DbHandle {
        iotkit_core_storage::init_db_memory(&all_migrations()).unwrap()
    }

    fn ts(millis: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(millis)
    }

    #[tokio::test]
    async fn reject_nan_in_values() {
        let db = test_db();
        let result = insert_reading(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            ts(1000),
            &SensorType::Temperature,
            &[f64::NAN],
            None,
            None,
        )
        .await;
        assert!(matches!(result, Err(TimeseriesError::InvalidReading(msg)) if msg.contains("NaN")));
    }

    #[tokio::test]
    async fn reject_infinity_in_values() {
        let db = test_db();
        let result = insert_reading(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            ts(1000),
            &SensorType::Temperature,
            &[f64::INFINITY],
            None,
            None,
        )
        .await;
        assert!(matches!(result, Err(TimeseriesError::InvalidReading(msg)) if msg.contains("Inf")));
    }

    #[tokio::test]
    async fn reject_pre_epoch_timestamp() {
        let db = test_db();
        let pre_epoch = UNIX_EPOCH - Duration::from_secs(1);
        let result = insert_reading(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            pre_epoch,
            &SensorType::Temperature,
            &[25.0],
            None,
            None,
        )
        .await;
        assert!(matches!(result, Err(TimeseriesError::InvalidReading(msg)) if msg.contains("epoch")));
    }

    #[tokio::test]
    async fn insert_succeeds() {
        let db = test_db();
        insert_reading(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            ts(1000),
            &SensorType::Temperature,
            &[25.3],
            Some(-50),
            Some(85),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn insert_multiple_sensor_types_same_timestamp() {
        let db = test_db();
        let t = ts(1000);
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), t, &SensorType::Temperature, &[25.3], None, None).await.unwrap();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), t, &SensorType::Acceleration, &[0.1, -0.3, 9.8], None, None).await.unwrap();
    }

    #[tokio::test]
    async fn insert_and_query_single() {
        let db = test_db();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000), &SensorType::Temperature, &[25.3], Some(-50), Some(85)).await.unwrap();

        let rows = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            None,
            TimeRange { start: ts(0), end: ts(2000) },
            100,
        ).await.unwrap();

        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.adapter_id, "a1");
        assert_eq!(r.device_key, "d1");
        assert_eq!(r.ingested_at, 1000);
        assert_eq!(r.sensor_type, SensorType::Temperature);
        assert_eq!(r.values, vec![25.3]);
        assert_eq!(r.rssi, Some(-50));
        assert_eq!(r.battery_pct, Some(85));
    }

    #[tokio::test]
    async fn query_with_sensor_type_filter() {
        let db = test_db();
        let t = ts(1000);
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), t, &SensorType::Temperature, &[25.3], None, None).await.unwrap();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), t, &SensorType::Acceleration, &[0.1, -0.3, 9.8], None, None).await.unwrap();

        let rows = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            Some(&SensorType::Temperature),
            TimeRange { start: ts(0), end: ts(2000) },
            100,
        ).await.unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sensor_type, SensorType::Temperature);
    }

    #[tokio::test]
    async fn query_time_range() {
        let db = test_db();
        for i in 0..5 {
            insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000 + i * 1000), &SensorType::Temperature, &[20.0 + i as f64], None, None).await.unwrap();
        }

        let rows = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            None,
            TimeRange { start: ts(2000), end: ts(4000) },
            100,
        ).await.unwrap();

        assert_eq!(rows.len(), 2); // ts 2000 and 3000
    }

    #[tokio::test]
    async fn query_respects_limit() {
        let db = test_db();
        for i in 0..10 {
            insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000 + i * 1000), &SensorType::Temperature, &[20.0], None, None).await.unwrap();
        }

        let rows = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            None,
            TimeRange { start: ts(0), end: ts(100_000) },
            3,
        ).await.unwrap();

        assert_eq!(rows.len(), 3);
    }

    #[tokio::test]
    async fn query_returns_newest_first() {
        let db = test_db();
        for i in 0..3 {
            insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000 + i * 1000), &SensorType::Temperature, &[20.0], None, None).await.unwrap();
        }

        let rows = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            None,
            TimeRange { start: ts(0), end: ts(100_000) },
            100,
        ).await.unwrap();

        assert!(rows[0].ingested_at > rows[1].ingested_at);
        assert!(rows[1].ingested_at > rows[2].ingested_at);
    }

    #[tokio::test]
    async fn query_rejects_invalid_range() {
        let db = test_db();
        let result = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            None,
            TimeRange { start: ts(2000), end: ts(1000) },
            100,
        ).await;
        assert!(matches!(result, Err(TimeseriesError::InvalidReading(msg)) if msg.contains("start >= end")));
    }

    #[tokio::test]
    async fn duplicate_pk_is_silently_ignored() {
        let db = test_db();
        let t = ts(1000);
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), t, &SensorType::Temperature, &[25.0], None, None).await.unwrap();
        // Second insert with same PK succeeds (OR IGNORE) but does not overwrite
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), t, &SensorType::Temperature, &[99.0], None, None).await.unwrap();

        // Original value is preserved
        let rows = query_readings(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), None, TimeRange { start: ts(0), end: ts(2000) }, 100).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].values, vec![25.0]);
    }

    #[tokio::test]
    async fn query_deterministic_ordering_same_timestamp() {
        let db = test_db();
        let t = ts(1000);
        // Insert in reverse alphabetical order
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), t, &SensorType::Temperature, &[25.0], None, None).await.unwrap();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), t, &SensorType::Acceleration, &[0.1], None, None).await.unwrap();

        let rows = query_readings(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), None, TimeRange { start: ts(0), end: ts(2000) }, 100).await.unwrap();
        assert_eq!(rows.len(), 2);
        // sensor_type ASC: acceleration < temperature
        assert_eq!(rows[0].sensor_type, SensorType::Acceleration);
        assert_eq!(rows[1].sensor_type, SensorType::Temperature);
    }

    #[tokio::test]
    async fn latest_reading_returns_most_recent() {
        let db = test_db();
        for i in 0..3 {
            insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000 + i * 1000), &SensorType::Temperature, &[20.0 + i as f64], None, None).await.unwrap();
        }

        let row = latest_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), None).await.unwrap().unwrap();
        assert_eq!(row.ingested_at, 3000);
        assert_eq!(row.values, vec![22.0]);
    }

    #[tokio::test]
    async fn latest_reading_empty() {
        let db = test_db();
        let row = latest_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), None).await.unwrap();
        assert!(row.is_none());
    }

    #[tokio::test]
    async fn latest_reading_with_sensor_type_filter() {
        let db = test_db();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000), &SensorType::Temperature, &[25.0], None, None).await.unwrap();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(2000), &SensorType::Acceleration, &[0.1], None, None).await.unwrap();

        let row = latest_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), Some(&SensorType::Temperature)).await.unwrap().unwrap();
        assert_eq!(row.sensor_type, SensorType::Temperature);
        assert_eq!(row.ingested_at, 1000);
    }

    #[tokio::test]
    async fn latest_reading_tiebreak_deterministic() {
        let db = test_db();
        let t = ts(1000);
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), t, &SensorType::Temperature, &[25.0], None, None).await.unwrap();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), t, &SensorType::Acceleration, &[0.1], None, None).await.unwrap();

        let row = latest_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), None).await.unwrap().unwrap();
        // Alphabetically first: acceleration < temperature
        assert_eq!(row.sensor_type, SensorType::Acceleration);
    }

    #[tokio::test]
    async fn values_json_round_trip() {
        let db = test_db();
        let values = vec![0.1, -0.3, 9.8];
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000), &SensorType::Acceleration, &values, None, None).await.unwrap();

        let rows = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            None,
            TimeRange { start: ts(0), end: ts(2000) },
            100,
        ).await.unwrap();

        assert_eq!(rows[0].values, values);
    }

    #[tokio::test]
    async fn delete_before_removes_old() {
        let db = test_db();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000), &SensorType::Temperature, &[20.0], None, None).await.unwrap();
        insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(5000), &SensorType::Temperature, &[25.0], None, None).await.unwrap();

        delete_before(&db, ts(3000)).await.unwrap();

        let rows = query_readings(
            &db,
            &AdapterId::new("a1"),
            &DeviceKey::new("d1"),
            None,
            TimeRange { start: ts(0), end: ts(100_000) },
            100,
        ).await.unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ingested_at, 5000);
    }

    #[tokio::test]
    async fn delete_before_returns_count() {
        let db = test_db();
        for i in 0..5 {
            insert_reading(&db, &AdapterId::new("a1"), &DeviceKey::new("d1"), ts(1000 + i * 1000), &SensorType::Temperature, &[20.0], None, None).await.unwrap();
        }

        let deleted = delete_before(&db, ts(3500)).await.unwrap();
        assert_eq!(deleted, 3); // ts 1000, 2000, 3000
    }
}

#[cfg(test)]
mod v3_tests {
    use super::*;
    use iotkit_core_ledger as ledger;
    use iotkit_core_storage::init_db_memory;

    fn v3_db() -> iotkit_core_storage::DbHandle {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(ledger::MIGRATIONS);
        all.extend_from_slice(crate::MIGRATIONS); // v2, v4
        // 昇順必須: 1(ledgerなし), 2, 3, 4 の順に並べ替え
        all.sort_by_key(|m| m.version);
        init_db_memory(&all).unwrap()
    }

    fn seed_series(conn: &rusqlite::Connection) -> i64 {
        let sid = ledger::insert_device(conn, &ledger::NewDevice {
            hardware_id: "ble:aa".into(), user_label: None, parent: None,
            kind: ledger::DeviceKind::Individual,
            initial_state: ledger::DeviceState::Active,
        }).unwrap();
        ledger::ensure_series(conn, &sid, "temperature_c", -1, "primary", false).unwrap()
    }

    #[test]
    fn claim_envelope_detects_duplicates() {
        let db = v3_db();
        db.with_conn_sync(|conn| {
            assert!(try_claim_envelope(conn, "adapterA", "e-1").unwrap());
            assert!(!try_claim_envelope(conn, "adapterA", "e-1").unwrap());
            assert!(try_claim_envelope(conn, "adapterB", "e-1").unwrap()); // 送信者スコープ(D1)
            Ok(())
        }).unwrap();
    }

    #[test]
    fn insert_reading_v3_returns_monotonic_seq() {
        let db = v3_db();
        db.with_conn_sync(|conn| {
            let series_id = seed_series(conn);
            let r = NewReading {
                series_id, received_at_ms: 1000, device_time_ms: None,
                time_source: "gateway".into(), values: vec![21.5],
                rssi: None, battery_pct: None, quarantined: false,
            };
            let s1 = insert_reading_v3(conn, &r).unwrap();
            let s2 = insert_reading_v3(conn, &r).unwrap(); // 同時刻・同値でも別行(v2の暗黙dedup廃止)
            assert!(s2 > s1);
            Ok(())
        }).unwrap();
    }

    #[test]
    fn staged_readings_are_bounded_per_hardware_id() {
        let db = v3_db();
        db.with_conn_sync(|conn| {
            for i in 0..1005 {
                insert_staged_reading(conn, "ble:new", i, "{}").unwrap();
            }
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM staged_readings WHERE hardware_id='ble:new'", [], |r| r.get(0),
            ).unwrap();
            assert_eq!(n, 1000);
            let oldest: i64 = conn.query_row(
                "SELECT MIN(received_at) FROM staged_readings WHERE hardware_id='ble:new'", [], |r| r.get(0),
            ).unwrap();
            assert_eq!(oldest, 5); // 最古削除
            Ok(())
        }).unwrap();
    }

    #[test]
    fn purge_dedup_before_removes_old_entries() {
        let db = v3_db();
        db.with_conn_sync(|conn| {
            try_claim_envelope(conn, "a", "old").unwrap();
            conn.execute("UPDATE ingest_dedup SET received_at = 0", []).unwrap();
            try_claim_envelope(conn, "a", "new").unwrap();
            assert_eq!(purge_dedup_before(conn, 1).unwrap(), 1);
            Ok(())
        }).unwrap();
    }
}
