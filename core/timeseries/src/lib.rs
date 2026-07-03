//! iotkit-core-timeseries: readings persistence.

mod error;

pub use error::TimeseriesError;

use iotkit_core_storage::Migration;

/// Timeseries migrations. Append to core/storage MIGRATIONS when assembling.
/// (gateway側でv3=ledgerを間に挟んで連結する。versionは昇順検証があるため
/// 1, 3, 4, 5, 7 の順に並べて渡す必要がある。)
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 4,
        label: "readings_v3",
        sql: include_str!("../migrations/0004_readings_v3.sql"),
    },
    Migration {
        version: 7,
        label: "drop_sensor_readings",
        sql: include_str!("../migrations/0007_drop_sensor_readings.sql"),
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

#[cfg(test)]
mod v3_tests {
    use super::*;
    use iotkit_core_ledger as ledger;
    use iotkit_core_storage::init_db_memory;

    fn v3_db() -> iotkit_core_storage::DbHandle {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(ledger::MIGRATIONS);
        all.extend_from_slice(crate::MIGRATIONS); // v4, v7
        // 昇順必須: 1(ledgerなし), 3, 4, 5, 7 の順に並べ替え
        all.sort_by_key(|m| m.version);
        init_db_memory(&all).unwrap()
    }

    fn seed_series(conn: &rusqlite::Connection) -> i64 {
        let sid = ledger::insert_device(conn, &ledger::NewDevice {
            hardware_id: "ble:aa".into(), user_label: None, parent: None,
            kind: ledger::DeviceKind::Individual,
            initial_state: ledger::DeviceState::Active,
        }).unwrap();
        ledger::ensure_series(conn, &sid, "temperature_c", ledger::CHANNEL_NA, ledger::DEFAULT_VARIANT, false, None)
            .unwrap()
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

    #[test]
    fn migration_v7_drops_sensor_readings_from_legacy_db() {
        let db = iotkit_core_storage::init_db_memory(iotkit_core_storage::MIGRATIONS).unwrap();
        db.with_conn_sync(|conn| {
            conn.execute_batch(
                "CREATE TABLE sensor_readings (
                    adapter_id  TEXT NOT NULL,
                    device_key  TEXT NOT NULL,
                    ingested_at INTEGER NOT NULL,
                    sensor_type TEXT NOT NULL,
                    values_json TEXT NOT NULL,
                    rssi        INTEGER,
                    battery_pct INTEGER,
                    PRIMARY KEY (adapter_id, device_key, ingested_at, sensor_type)
                );
                INSERT INTO sensor_readings
                    (adapter_id, device_key, ingested_at, sensor_type, values_json)
                VALUES ('a1', 'd1', 1000, 'temperature', '[21.0]');
                INSERT INTO _schema_version (version, label, applied_at)
                VALUES (2, 'timeseries', 0);",
            )
            .unwrap();

            let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
            all.extend_from_slice(ledger::MIGRATIONS);
            all.extend_from_slice(crate::MIGRATIONS);
            all.sort_by_key(|m| m.version);
            iotkit_core_storage::run_migrations(conn, &all).unwrap();

            let table_exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='sensor_readings'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(!table_exists, "sensor_readings must be dropped by v7");
            Ok(())
        })
        .unwrap();
    }
}
