//! iotkit-core-timeseries: sensor reading persistence (INSERT/query/delete).

mod error;
mod model;

pub use error::TimeseriesError;
pub use model::{ReadingRow, TimeRange};

use std::time::{SystemTime, UNIX_EPOCH};

use iotkit_core_storage::{DbHandle, Migration};
use iotkit_core_types::{AdapterId, DeviceKey, SensorType};

/// Timeseries migrations. Append to core/storage MIGRATIONS when assembling.
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 2,
    label: "timeseries",
    sql: include_str!("../migrations/0002_timeseries.sql"),
}];

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
