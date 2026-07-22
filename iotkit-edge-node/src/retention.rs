use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{ffi::CString, os::unix::ffi::OsStrExt};

use iotkit_core_storage::DbHandle;
use rusqlite::{TransactionBehavior, named_params, params_from_iter};

use iotkit_edge_node::health::{DbHealth, HealthState, RetentionHealth, now_ms};

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
#[path = "../tests/unit/retention_tests.rs"]
mod tests;
