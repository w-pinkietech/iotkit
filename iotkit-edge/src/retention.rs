use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{ffi::CString, os::unix::ffi::OsStrExt};

use iotkit_core_storage::DbHandle;
use rusqlite::{TransactionBehavior, named_params, params_from_iter};

use iotkit_edge::health::{DbHealth, HealthState, RetentionHealth, now_ms};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const DEDUP_TTL_MS: i64 = 72 * 60 * 60 * 1000;
const PURGE_BATCH: usize = 5_000;

#[derive(Debug, Clone, Copy)]
pub struct RetentionConfig {
    pub retention_days: u64,
    pub quarantine_ttl_days: u64,
    pub disk_high_watermark_pct: u64,
}

#[derive(Debug, Default)]
pub struct WatermarkLatch {
    exceeded: bool,
}

/// Custody-aware readings purge. effective_cursor:
/// None = floor-only (no protection), Some(n) = protect unacked current-epoch measurements.
/// Returns deleted readings count. Call inside the caller's transaction.
fn purge_readings_custody_aware(
    conn: &rusqlite::Connection,
    cutoff_ms: i64,
    current_epoch: &str,
    effective_cursor: Option<i64>,
) -> Result<u64, rusqlite::Error> {
    const SELECT_PROTECTED: &str = "
        SELECT seq FROM readings
        WHERE received_at < :cutoff
          AND NOT (
              quarantined = 0
              AND seq IN (
                  SELECT p.reading_seq FROM publication_log p
                  WHERE p.kind='measurement'
                    AND p.reading_seq IS NOT NULL
                    AND p.epoch = :cur
                    AND p.pub_seq > :eff
              )
          )
        LIMIT :batch";
    const SELECT_FLOOR_ONLY: &str =
        "SELECT seq FROM readings WHERE received_at < :cutoff LIMIT :batch";

    if let Some(eff) = effective_cursor {
        iotkit_core_publish::store::prune_acked_outbox(conn, current_epoch, eff)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    }

    let mut total = 0_u64;
    loop {
        let seqs = match effective_cursor {
            Some(eff) => {
                let mut stmt = conn.prepare(SELECT_PROTECTED)?;
                stmt.query_map(
                    named_params! {
                        ":cutoff": cutoff_ms,
                        ":cur": current_epoch,
                        ":eff": eff,
                        ":batch": PURGE_BATCH as i64,
                    },
                    |row| row.get::<_, i64>(0),
                )?
                .collect::<Result<Vec<_>, _>>()?
            }
            None => {
                let mut stmt = conn.prepare(SELECT_FLOOR_ONLY)?;
                stmt.query_map(
                    named_params! { ":cutoff": cutoff_ms, ":batch": PURGE_BATCH as i64 },
                    |row| row.get::<_, i64>(0),
                )?
                .collect::<Result<Vec<_>, _>>()?
            }
        };

        if seqs.is_empty() {
            break;
        }

        iotkit_core_publish::store::prune_outbox_by_reading_seqs(conn, &seqs)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

        let placeholders = std::iter::repeat_n("?", seqs.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("DELETE FROM readings WHERE seq IN ({placeholders})");
        let deleted = conn.execute(&sql, params_from_iter(seqs.iter()))?;
        total += deleted as u64;

        if seqs.len() < PURGE_BATCH {
            break;
        }
    }

    Ok(total)
}

pub fn spawn_retention_task(
    db: DbHandle,
    db_path: std::path::PathBuf,
    config: RetentionConfig,
    health: Arc<Mutex<HealthState>>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut latch = WatermarkLatch::default();
        loop {
            if let Err(e) =
                run_retention_once_with_latch(&db, &db_path, config, health.clone(), &mut latch)
                    .await
            {
                tracing::error!(error = %e, "retention task failed");
            }
            tokio::time::sleep(interval).await;
        }
    })
}

pub async fn run_retention_once_with_latch(
    db: &DbHandle,
    db_path: &Path,
    config: RetentionConfig,
    health: Arc<Mutex<HealthState>>,
    latch: &mut WatermarkLatch,
) -> Result<(), String> {
    let now = now_ms();
    let reading_cutoff = now.saturating_sub((config.retention_days as i64).saturating_mul(DAY_MS));
    let dedup_cutoff = now.saturating_sub(DEDUP_TTL_MS);
    let ttl_ms = (config.quarantine_ttl_days as i64).saturating_mul(DAY_MS);
    let db_path = db_path.to_path_buf();
    let (purged_readings, purged_dedup) = db
        .with_conn(move |conn| {
            let started = std::time::Instant::now();
            let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
            let purged_dedup = iotkit_core_timeseries::purge_dedup_before(&tx, dedup_cutoff)
                .map_err(|e| {
                    iotkit_core_storage::StorageError::Sqlite(
                        rusqlite::Error::ToSqlConversionFailure(Box::new(e)),
                    )
                })?;
            let current_epoch = iotkit_core_ledger::ledger_epoch(&tx).map_err(|e| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(e),
                ))
            })?;
            let target = iotkit_core_publish::store::target_get(&tx).map_err(|e| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(e),
                ))
            })?;
            let effective_cursor = match &target {
                None => None,
                Some(t) if !t.archive_responsible => None,
                Some(t) => Some(iotkit_core_publish::store::effective_cursor(
                    &current_epoch,
                    t,
                )),
            };
            let purged_readings =
                purge_readings_custody_aware(&tx, reading_cutoff, &current_epoch, effective_cursor)
                    .map_err(|e| {
                        iotkit_core_storage::StorageError::Sqlite(
                            rusqlite::Error::ToSqlConversionFailure(Box::new(e)),
                        )
                    })?;
            let expired =
                iotkit_core_ledger::expire_quarantined_devices(&tx, ttl_ms).map_err(|e| {
                    iotkit_core_storage::StorageError::Sqlite(
                        rusqlite::Error::ToSqlConversionFailure(Box::new(e)),
                    )
                })?;
            if !expired.is_empty() {
                iotkit_core_ledger::bump_generation(&tx).map_err(|e| {
                    iotkit_core_storage::StorageError::Sqlite(
                        rusqlite::Error::ToSqlConversionFailure(Box::new(e)),
                    )
                })?;
            }
            let duration_ms = started.elapsed().as_millis() as i64;
            let detail = format!(
                r#"{{"readings":{},"dedup":{},"expired_quarantines":{},"duration_ms":{}}}"#,
                purged_readings,
                purged_dedup,
                expired.len(),
                duration_ms
            );
            iotkit_core_ledger::record_event(&tx, "retention_purge", None, &detail).map_err(
                |e| {
                    iotkit_core_storage::StorageError::Sqlite(
                        rusqlite::Error::ToSqlConversionFailure(Box::new(e)),
                    )
                },
            )?;
            tx.commit()?;
            Ok((purged_readings, purged_dedup))
        })
        .await
        .map_err(|e| e.to_string())?;

    let purged_sightings = db
        .with_conn(move |conn| {
            let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
            let n = iotkit_core_ledger::purge_sightings(&tx, now).map_err(|e| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(e),
                ))
            })?;
            tx.commit()?;
            Ok(n)
        })
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "sightings purge failed (non-fatal, will retry next pass)");
            0
        });
    tracing::info!(purged_sightings, "sightings purge");

    let db_health =
        observe_watermark_latched(db, &db_path, config.disk_high_watermark_pct, latch).await?;
    {
        let mut state = health.lock().expect("health state mutex poisoned");
        state.db = db_health;
        state.retention = RetentionHealth {
            days: config.retention_days,
            last_purge_at: Some(now),
            last_purged_rows: purged_readings + purged_dedup + purged_sightings,
        };
    }
    Ok(())
}

pub fn observe_db_health(db_path: &Path, disk_high_watermark_pct: u64) -> Result<DbHealth, String> {
    let size_bytes = file_len(db_path) + file_len(&wal_path(db_path));
    let stat_path = match db_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let c_path = CString::new(stat_path.as_os_str().as_bytes())
        .map_err(|_| format!("path contains NUL: {}", stat_path.display()))?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: c_path is a valid NUL-terminated path and stat points to writable memory.
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: statvfs returned success and initialized the struct.
    let stat = unsafe { stat.assume_init() };
    let available = stat.f_bavail.saturating_mul(stat.f_frsize);
    let total = stat.f_blocks.saturating_mul(stat.f_frsize);
    let used_pct = available
        .saturating_mul(100)
        .checked_div(total)
        .map_or(0, |available_pct| 100_u64.saturating_sub(available_pct));
    Ok(DbHealth {
        size_bytes,
        disk_available_bytes: available,
        watermark_exceeded: used_pct >= disk_high_watermark_pct,
    })
}

pub async fn observe_watermark_latched(
    db: &DbHandle,
    db_path: &Path,
    disk_high_watermark_pct: u64,
    latch: &mut WatermarkLatch,
) -> Result<DbHealth, String> {
    let health = observe_db_health(db_path, disk_high_watermark_pct)?;
    if health.watermark_exceeded && !latch.exceeded {
        let size = health.size_bytes;
        let avail = health.disk_available_bytes;
        let detail = format!(
            r#"{{"size_bytes":{},"disk_available_bytes":{},"threshold_pct":{}}}"#,
            size, avail, disk_high_watermark_pct
        );
        db.with_conn(move |conn| {
            iotkit_core_ledger::record_event(conn, "disk_watermark_exceeded", None, &detail)
                .map_err(|e| {
                    iotkit_core_storage::StorageError::Sqlite(
                        rusqlite::Error::ToSqlConversionFailure(Box::new(e)),
                    )
                })?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?;
        latch.exceeded = true;
    } else if !health.watermark_exceeded {
        latch.exceeded = false;
    }
    Ok(health)
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn wal_path(path: &Path) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}-wal", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_migrations() -> Vec<iotkit_core_storage::Migration> {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
        all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
        all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
        all.extend_from_slice(iotkit_core_publish::MIGRATIONS);
        all.sort_by_key(|m| m.version);
        all
    }

    fn retention_db() -> iotkit_core_storage::DbHandle {
        iotkit_core_storage::init_db_memory(&all_migrations()).unwrap()
    }

    fn seed_series(conn: &rusqlite::Connection) -> i64 {
        let sid = iotkit_core_ledger::insert_device(
            conn,
            &iotkit_core_ledger::NewDevice {
                hardware_id: "ble:aa".into(),
                user_label: None,
                parent: None,
                kind: iotkit_core_ledger::DeviceKind::Individual,
                initial_state: iotkit_core_ledger::DeviceState::Active,
            },
        )
        .unwrap();
        iotkit_core_ledger::ensure_series(
            conn,
            &sid,
            "temperature_c",
            iotkit_core_ledger::CHANNEL_NA,
            iotkit_core_ledger::DEFAULT_VARIANT,
            false,
            None,
        )
        .unwrap()
    }

    fn seed_reading(
        conn: &rusqlite::Connection,
        series_id: i64,
        received_at_ms: i64,
        quarantined: bool,
    ) -> i64 {
        iotkit_core_timeseries::insert_reading_v3(
            conn,
            &iotkit_core_timeseries::NewReading {
                series_id,
                received_at_ms,
                device_time_ms: None,
                time_source: "edge".into(),
                values: vec![21.5],
                rssi: None,
                battery_pct: None,
                quarantined,
            },
        )
        .unwrap()
    }

    fn reading_seqs(conn: &rusqlite::Connection) -> Vec<i64> {
        let mut stmt = conn
            .prepare("SELECT seq FROM readings ORDER BY seq ASC")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn publog_reading_seqs(conn: &rusqlite::Connection) -> Vec<i64> {
        let mut stmt = conn
            .prepare(
                "SELECT reading_seq FROM publication_log
                 WHERE kind = 'measurement'
                 ORDER BY pub_seq ASC",
            )
            .unwrap();
        stmt.query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn publog_count_for_reading(conn: &rusqlite::Connection, seq: i64) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM publication_log WHERE reading_seq = ?1",
            rusqlite::params![seq],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn insert_target(
        conn: &rusqlite::Connection,
        archive_responsible: bool,
        cursor_epoch: Option<String>,
        cursor_pub_seq: i64,
    ) {
        iotkit_core_publish::store::target_insert(
            conn,
            &iotkit_core_publish::store::TargetRow {
                target_id: "target-1".into(),
                endpoint_url: "https://archive.example.test".into(),
                credential_token: "token-1".into(),
                archive_responsible,
                schema_version: 1,
                cursor_epoch,
                cursor_pub_seq,
            },
            1_000,
        )
        .unwrap();
    }

    async fn run_retention_for_test(db: &iotkit_core_storage::DbHandle) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("iotkit.db");
        let config = RetentionConfig {
            retention_days: 1,
            quarantine_ttl_days: 30,
            disk_high_watermark_pct: 101,
        };
        let health = Arc::new(Mutex::new(HealthState::new(config.retention_days)));
        let mut latch = WatermarkLatch::default();

        run_retention_once_with_latch(db, &db_path, config, health, &mut latch)
            .await
            .unwrap();
    }

    fn set_sqlite_variable_limit(conn: &rusqlite::Connection, limit: i32) {
        // SAFETY: This test-only call adjusts a documented SQLite runtime limit on
        // the active connection and does not retain the raw handle.
        unsafe {
            rusqlite::ffi::sqlite3_limit(
                conn.handle(),
                rusqlite::ffi::SQLITE_LIMIT_VARIABLE_NUMBER,
                limit,
            );
        }
    }

    #[test]
    fn floor_protects_recent_and_purges_old_acked() {
        let db = retention_db();
        db.with_conn_sync(|conn| {
            let cutoff = 1_000;
            let series_id = seed_series(conn);
            let old_acked = seed_reading(conn, series_id, cutoff - 1, false);
            let recent = seed_reading(conn, series_id, cutoff, false);
            let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
            let acked_pub_seq =
                iotkit_core_publish::store::enqueue_measurement(conn, &epoch, old_acked, 10)
                    .unwrap();
            iotkit_core_publish::store::enqueue_measurement(conn, &epoch, recent, 11).unwrap();

            let purged =
                purge_readings_custody_aware(conn, cutoff, &epoch, Some(acked_pub_seq)).unwrap();

            assert_eq!(purged, 1);
            assert_eq!(reading_seqs(conn), vec![recent]);
            assert_eq!(publog_reading_seqs(conn), vec![recent]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn unacked_pubseq_rows_are_protected_even_if_old() {
        let db = retention_db();
        db.with_conn_sync(|conn| {
            let cutoff = 1_000;
            let series_id = seed_series(conn);
            let old_unacked = seed_reading(conn, series_id, cutoff - 1, false);
            let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
            iotkit_core_publish::store::enqueue_measurement(conn, &epoch, old_unacked, 10).unwrap();

            let purged = purge_readings_custody_aware(conn, cutoff, &epoch, Some(0)).unwrap();

            assert_eq!(purged, 0);
            assert_eq!(reading_seqs(conn), vec![old_unacked]);
            assert_eq!(publog_reading_seqs(conn), vec![old_unacked]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn quarantined_rows_floor_purge_not_protected() {
        let db = retention_db();
        db.with_conn_sync(|conn| {
            let cutoff = 1_000;
            let series_id = seed_series(conn);
            let old_quarantined = seed_reading(conn, series_id, cutoff - 1, true);
            let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();

            let purged = purge_readings_custody_aware(conn, cutoff, &epoch, Some(0)).unwrap();

            assert_eq!(purged, 1);
            assert!(reading_seqs(conn).is_empty());
            assert_eq!(publog_count_for_reading(conn, old_quarantined), 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn epoch_mismatch_treats_all_current_as_unacked() {
        let db = retention_db();
        db.with_conn_sync(|conn| {
            let cutoff = 1_000;
            let series_id = seed_series(conn);
            let current_epoch_pubseq = seed_reading(conn, series_id, cutoff - 1, false);
            let unqueued = seed_reading(conn, series_id, cutoff - 1, false);
            let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
            iotkit_core_publish::store::enqueue_measurement(conn, &epoch, current_epoch_pubseq, 10)
                .unwrap();

            let purged = purge_readings_custody_aware(conn, cutoff, &epoch, Some(0)).unwrap();

            assert_eq!(purged, 1);
            assert_eq!(reading_seqs(conn), vec![current_epoch_pubseq]);
            assert_eq!(publog_reading_seqs(conn), vec![current_epoch_pubseq]);
            assert_eq!(publog_count_for_reading(conn, unqueued), 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn no_target_registered_purges_by_floor_only() {
        let db = retention_db();
        db.with_conn_sync(|conn| {
            let cutoff = 1_000;
            let series_id = seed_series(conn);
            let old_with_pubseq = seed_reading(conn, series_id, cutoff - 1, false);
            let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
            iotkit_core_publish::store::enqueue_measurement(conn, &epoch, old_with_pubseq, 10)
                .unwrap();

            let purged = purge_readings_custody_aware(conn, cutoff, &epoch, None).unwrap();

            assert_eq!(purged, 1);
            assert!(reading_seqs(conn).is_empty());
            assert!(publog_reading_seqs(conn).is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn old_epoch_outbox_and_readings_pruned_as_pair() {
        let db = retention_db();
        db.with_conn_sync(|conn| {
            let cutoff = 1_000;
            let series_id = seed_series(conn);
            let old_epoch_reading = seed_reading(conn, series_id, cutoff - 1, false);
            let current_epoch_reading = seed_reading(conn, series_id, cutoff - 1, false);
            let current_epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
            iotkit_core_publish::store::enqueue_measurement(
                conn,
                "previous-epoch",
                old_epoch_reading,
                10,
            )
            .unwrap();
            iotkit_core_publish::store::enqueue_measurement(
                conn,
                &current_epoch,
                current_epoch_reading,
                11,
            )
            .unwrap();

            let purged =
                purge_readings_custody_aware(conn, cutoff, &current_epoch, Some(0)).unwrap();

            assert_eq!(purged, 1);
            assert_eq!(reading_seqs(conn), vec![current_epoch_reading]);
            assert_eq!(publog_reading_seqs(conn), vec![current_epoch_reading]);
            assert_eq!(publog_count_for_reading(conn, old_epoch_reading), 0);
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn retention_target_not_archive_responsible_purges_unacked_floor_only() {
        let db = retention_db();
        let old_with_pubseq = db
            .with_conn_sync(|conn| {
                let series_id = seed_series(conn);
                let old_with_pubseq = seed_reading(conn, series_id, 1_000, false);
                let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
                iotkit_core_publish::store::enqueue_measurement(
                    conn,
                    &epoch,
                    old_with_pubseq,
                    2_000,
                )
                .unwrap();
                insert_target(conn, false, Some(epoch), 0);
                Ok(old_with_pubseq)
            })
            .unwrap();

        run_retention_for_test(&db).await;

        db.with_conn_sync(|conn| {
            assert!(reading_seqs(conn).is_empty());
            assert_eq!(publog_count_for_reading(conn, old_with_pubseq), 0);
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn retention_target_without_current_cursor_protects_current_epoch_unacked() {
        let db = retention_db();
        let old_unacked = db
            .with_conn_sync(|conn| {
                let series_id = seed_series(conn);
                let old_unacked = seed_reading(conn, series_id, 1_000, false);
                let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
                iotkit_core_publish::store::enqueue_measurement(conn, &epoch, old_unacked, 2_000)
                    .unwrap();
                insert_target(conn, true, None, 0);
                Ok(old_unacked)
            })
            .unwrap();

        run_retention_for_test(&db).await;

        db.with_conn_sync(|conn| {
            assert_eq!(reading_seqs(conn), vec![old_unacked]);
            assert_eq!(publog_reading_seqs(conn), vec![old_unacked]);
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn retention_purges_stale_sightings_and_keeps_fresh() {
        let db = retention_db();
        let now = now_ms();
        let stale_last_seen = now - (31 * DAY_MS);
        db.with_conn_sync(|conn| {
            conn.execute(
                "INSERT INTO sightings (hardware_id, source, first_seen, last_seen, observations)
                 VALUES (?1, ?2, ?3, ?3, 1)",
                rusqlite::params!["ble:stale", "adapter-test", stale_last_seen],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sightings (hardware_id, source, first_seen, last_seen, observations)
                 VALUES (?1, ?2, ?3, ?3, 1)",
                rusqlite::params!["ble:fresh", "adapter-test", now],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();

        run_retention_for_test(&db).await;

        db.with_conn_sync(|conn| {
            let remaining = iotkit_core_ledger::list_sightings(conn).unwrap();
            assert_eq!(remaining.len(), 1);
            assert_eq!(remaining[0].hardware_id, "ble:fresh");
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn retention_sightings_purge_failure_does_not_abort_critical_purge() {
        let db = retention_db();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("iotkit.db");
        let config = RetentionConfig {
            retention_days: 1,
            quarantine_ttl_days: 30,
            disk_high_watermark_pct: 101,
        };
        let health = Arc::new(Mutex::new(HealthState::new(config.retention_days)));
        let mut latch = WatermarkLatch::default();
        let now = now_ms();
        let stale_last_seen = now - (31 * DAY_MS);
        let old_reading = db
            .with_conn_sync(|conn| {
                let series_id = seed_series(conn);
                let old_reading = seed_reading(conn, series_id, 1_000, false);
                conn.execute(
                    "INSERT INTO sightings (hardware_id, source, first_seen, last_seen, observations)
                     VALUES (?1, ?2, ?3, ?3, 1)",
                    rusqlite::params!["ble:stale", "adapter-test", stale_last_seen],
                )
                .unwrap();
                conn.execute_batch(
                    "CREATE TRIGGER block_sightings_delete
                     BEFORE DELETE ON sightings
                     BEGIN
                         SELECT RAISE(FAIL, 'sightings delete blocked');
                     END;",
                )
                .unwrap();
                Ok(old_reading)
            })
            .unwrap();

        run_retention_once_with_latch(&db, &db_path, config, health.clone(), &mut latch)
            .await
            .expect("sightings purge failure must be non-fatal");

        db.with_conn_sync(|conn| {
            assert!(reading_seqs(conn).is_empty());
            assert_eq!(publog_count_for_reading(conn, old_reading), 0);
            let sightings = iotkit_core_ledger::list_sightings(conn).unwrap();
            assert_eq!(sightings.len(), 1);
            assert_eq!(sightings[0].hardware_id, "ble:stale");
            let detail: String = conn
                .query_row(
                    "SELECT detail FROM ledger_events WHERE kind = 'retention_purge'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(
                detail.starts_with(
                    r#"{"readings":1,"dedup":0,"expired_quarantines":0,"duration_ms":"#
                )
            );
            assert!(detail.ends_with('}'));
            assert!(!detail.contains(r#""sightings""#));
            Ok(())
        })
        .unwrap();

        let state = health.lock().expect("health state mutex poisoned");
        assert_eq!(state.retention.last_purged_rows, 1);
    }

    #[test]
    fn purge_readings_batches_more_than_purge_batch_victims() {
        let db = retention_db();
        db.with_conn_sync(|conn| {
            let cutoff = 1_000;
            let series_id = seed_series(conn);
            for _ in 0..=PURGE_BATCH {
                seed_reading(conn, series_id, cutoff - 1, false);
            }
            let epoch = iotkit_core_ledger::ledger_epoch(conn).unwrap();
            set_sqlite_variable_limit(conn, PURGE_BATCH as i32);

            let purged = purge_readings_custody_aware(conn, cutoff, &epoch, None).unwrap();

            assert_eq!(purged, (PURGE_BATCH + 1) as u64);
            assert!(reading_seqs(conn).is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn watermark_latch_records_once_until_recovered() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("iotkit.db");
        let db = iotkit_core_storage::init_db(&db_path, &all_migrations()).unwrap();
        let mut latch = WatermarkLatch::default();

        observe_watermark_latched(&db, &db_path, 0, &mut latch)
            .await
            .unwrap();
        observe_watermark_latched(&db, &db_path, 0, &mut latch)
            .await
            .unwrap();
        observe_watermark_latched(&db, &db_path, 101, &mut latch)
            .await
            .unwrap();
        observe_watermark_latched(&db, &db_path, 0, &mut latch)
            .await
            .unwrap();

        db.with_conn_sync(|conn| {
            let events: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM ledger_events WHERE kind = 'disk_watermark_exceeded'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(events, 2);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn db_health_uses_current_dir_for_basename_db_path() {
        let original_dir = std::env::current_dir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("iotkit.db"), b"db").unwrap();

        struct CurrentDirGuard(std::path::PathBuf);

        impl Drop for CurrentDirGuard {
            fn drop(&mut self) {
                std::env::set_current_dir(&self.0).unwrap();
            }
        }

        let _guard = CurrentDirGuard(original_dir);
        std::env::set_current_dir(dir.path()).unwrap();

        let health = observe_db_health(Path::new("iotkit.db"), 101).unwrap();

        assert_eq!(health.size_bytes, 2);
        assert!(!health.watermark_exceeded);
    }
}
