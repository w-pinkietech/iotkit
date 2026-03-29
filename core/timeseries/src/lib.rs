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

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO sensor_readings (adapter_id, device_key, ingested_at, sensor_type, values_json, rssi, battery_pct)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
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
        Ok(())
    })
    .await?;

    Ok(())
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
}
