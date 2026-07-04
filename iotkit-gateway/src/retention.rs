use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{ffi::CString, os::unix::ffi::OsStrExt};

use iotkit_core_storage::DbHandle;
use rusqlite::TransactionBehavior;

use crate::health::{DbHealth, HealthState, RetentionHealth, now_ms};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const DEDUP_TTL_MS: i64 = 72 * 60 * 60 * 1000;

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
            let purged_readings =
                iotkit_core_timeseries::query::purge_readings_before(conn, reading_cutoff)
                    .map_err(|e| {
                        iotkit_core_storage::StorageError::Sqlite(
                            rusqlite::Error::ToSqlConversionFailure(Box::new(e)),
                        )
                    })?;
            let purged_dedup = iotkit_core_timeseries::purge_dedup_before(conn, dedup_cutoff)
                .map_err(|e| {
                    iotkit_core_storage::StorageError::Sqlite(
                        rusqlite::Error::ToSqlConversionFailure(Box::new(e)),
                    )
                })?;
            let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
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
            let detail = format!(
                r#"{{"readings":{},"dedup":{},"expired_quarantines":{}}}"#,
                purged_readings,
                purged_dedup,
                expired.len()
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

    let db_health =
        observe_watermark_latched(db, &db_path, config.disk_high_watermark_pct, latch).await?;
    {
        let mut state = health.lock().expect("health state mutex poisoned");
        state.db = db_health;
        state.retention = RetentionHealth {
            days: config.retention_days,
            last_purge_at: Some(now),
            last_purged_rows: purged_readings + purged_dedup,
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
    let used_pct = if total == 0 {
        0
    } else {
        100_u64.saturating_sub(available.saturating_mul(100) / total)
    };
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
        all.sort_by_key(|m| m.version);
        all
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
