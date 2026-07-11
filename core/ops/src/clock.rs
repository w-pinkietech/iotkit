use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::json;

use crate::OpsError;

/// Maximum wall-clock movement allowed between displaying a manual confirmation and committing it.
pub const MAX_MANUAL_CONFIRMATION_DRIFT_MS: i64 = 30_000;

pub trait Clock: Send + Sync {
    fn wall_time_ms(&self) -> i64;
    fn monotonic_ms(&self) -> u64;
    fn kernel_synchronized(&self) -> bool;
}

pub struct SystemClock {
    monotonic_origin: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            monotonic_origin: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn wall_time_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0)
    }

    fn monotonic_ms(&self) -> u64 {
        self.monotonic_origin.elapsed().as_millis() as u64
    }

    fn kernel_synchronized(&self) -> bool {
        kernel_synchronized()
    }
}

#[cfg(target_os = "linux")]
fn kernel_synchronized() -> bool {
    // SAFETY: `timex` is fully initialized and `adjtimex` only fills/reads this buffer.
    let mut tx: libc::timex = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::adjtimex(&mut tx) };
    result >= 0 && tx.status & libc::STA_UNSYNC == 0
}

#[cfg(not(target_os = "linux"))]
fn kernel_synchronized() -> bool {
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustSource {
    KernelSync,
    ManualLocalRoot,
}

impl TrustSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::KernelSync => "kernel_sync",
            Self::ManualLocalRoot => "manual_local_root",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockEvidence {
    Untrusted,
    Trusted {
        source: TrustSource,
        observed_at_ms: i64,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ClockTrustError {
    #[error("trusted wall clock is required")]
    Untrusted,
    #[error(transparent)]
    Ops(#[from] OpsError),
}

struct RuntimeState {
    evidence: ClockEvidence,
    last_wall_ms: Option<i64>,
    consumed_manual_seq: i64,
    last_checkpoint_monotonic_ms: u64,
}

pub struct ClockTrust {
    clock: Arc<dyn Clock>,
    backward_tolerance_ms: i64,
    checkpoint_interval_ms: u64,
    state: Mutex<RuntimeState>,
}

impl ClockTrust {
    pub fn load(
        conn: &Connection,
        clock: Arc<dyn Clock>,
        backward_tolerance: Duration,
        checkpoint_interval: Duration,
    ) -> Result<Self, OpsError> {
        let manual_seq = conn.query_row(
            "SELECT manual_evidence_seq FROM auth_state WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(Self {
            clock,
            backward_tolerance_ms: backward_tolerance.as_millis() as i64,
            checkpoint_interval_ms: checkpoint_interval.as_millis() as u64,
            state: Mutex::new(RuntimeState {
                evidence: ClockEvidence::Untrusted,
                last_wall_ms: None,
                consumed_manual_seq: manual_seq,
                last_checkpoint_monotonic_ms: 0,
            }),
        })
    }

    pub fn evidence(&self) -> ClockEvidence {
        self.state
            .lock()
            .expect("clock trust mutex poisoned")
            .evidence
    }

    pub fn wall_time_ms(&self) -> i64 {
        self.clock.wall_time_ms()
    }

    pub fn persisted_floor(conn: &Connection) -> Result<i64, OpsError> {
        conn.query_row(
            "SELECT clock_floor_ms FROM auth_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(OpsError::from)
    }

    pub fn refresh(&self, conn: &Connection) -> Result<ClockEvidence, OpsError> {
        let mut state = self.state.lock().expect("clock trust mutex poisoned");
        let now = self.clock.wall_time_ms();
        let floor = Self::persisted_floor(conn)?;
        let manual_seq: i64 = conn.query_row(
            "SELECT manual_evidence_seq FROM auth_state WHERE id = 1",
            [],
            |row| row.get(0),
        )?;

        let stepped_back = state
            .last_wall_ms
            .is_some_and(|last| now < last.saturating_sub(self.backward_tolerance_ms));
        let behind_floor = now < floor;
        if stepped_back || behind_floor {
            state.evidence = ClockEvidence::Untrusted;
        } else if self.clock.kernel_synchronized() {
            state.evidence = ClockEvidence::Trusted {
                source: TrustSource::KernelSync,
                observed_at_ms: now,
            };
        } else if manual_seq > state.consumed_manual_seq {
            state.consumed_manual_seq = manual_seq;
            state.evidence = ClockEvidence::Trusted {
                source: TrustSource::ManualLocalRoot,
                observed_at_ms: now,
            };
        } else if matches!(
            state.evidence,
            ClockEvidence::Trusted {
                source: TrustSource::KernelSync,
                ..
            }
        ) {
            state.evidence = ClockEvidence::Untrusted;
        } else if let ClockEvidence::Trusted { source, .. } = state.evidence {
            state.evidence = ClockEvidence::Trusted {
                source,
                observed_at_ms: now,
            };
        }
        state.last_wall_ms = Some(now);
        Ok(state.evidence)
    }

    pub fn trusted_now_and_advance(&self, conn: &Connection) -> Result<i64, ClockTrustError> {
        let evidence = self.refresh(conn)?;
        let ClockEvidence::Trusted {
            source,
            observed_at_ms,
        } = evidence
        else {
            return Err(ClockTrustError::Untrusted);
        };
        conn.execute(
            "UPDATE auth_state
             SET clock_floor_ms = MAX(clock_floor_ms, ?1),
                 clock_evidence_source = ?2,
                 clock_evidence_at_ms = ?1
             WHERE id = 1",
            params![observed_at_ms, source.as_str()],
        )
        .map_err(OpsError::from)?;
        Ok(observed_at_ms)
    }

    pub fn checkpoint_if_due(&self, conn: &Connection) -> Result<bool, ClockTrustError> {
        let monotonic = self.clock.monotonic_ms();
        {
            let state = self.state.lock().expect("clock trust mutex poisoned");
            if monotonic.saturating_sub(state.last_checkpoint_monotonic_ms)
                < self.checkpoint_interval_ms
            {
                return Ok(false);
            }
        }
        self.trusted_now_and_advance(conn)?;
        self.state
            .lock()
            .expect("clock trust mutex poisoned")
            .last_checkpoint_monotonic_ms = monotonic;
        Ok(true)
    }
}

pub fn confirm_time_with_clock(
    conn: &Connection,
    clock: &dyn Clock,
    displayed_at_ms: i64,
) -> Result<(), OpsError> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let floor = ClockTrust::persisted_floor(&tx)?;
    let confirmed_at_ms = clock.wall_time_ms();
    let forward_drift_ms = confirmed_at_ms.checked_sub(displayed_at_ms);
    if displayed_at_ms < floor
        || confirmed_at_ms < floor
        || !matches!(forward_drift_ms, Some(0..=MAX_MANUAL_CONFIRMATION_DRIFT_MS))
    {
        return Err(OpsError::ClockUntrusted);
    }
    tx.execute(
        "UPDATE auth_state
         SET manual_evidence_seq = manual_evidence_seq + 1,
             clock_evidence_source = 'manual_local_root',
             clock_evidence_at_ms = ?1
         WHERE id = 1",
        [confirmed_at_ms],
    )?;
    iotkit_core_ledger::record_event(
        &tx,
        "clock_trust_confirmed",
        None,
        &json!({
            "actor": "local_cli",
            "source": "manual_local_root",
            "displayed_at_ms": displayed_at_ms,
            "confirmed_at_ms": confirmed_at_ms,
            "floor_ms": floor,
        })
        .to_string(),
    )?;
    tx.commit()?;
    Ok(())
}

pub fn manual_evidence_at(conn: &Connection) -> Result<Option<i64>, OpsError> {
    conn.query_row(
        "SELECT clock_evidence_at_ms FROM auth_state
         WHERE id = 1 AND clock_evidence_source = 'manual_local_root'",
        [],
        |row| row.get(0),
    )
    .optional()
    .map_err(OpsError::from)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

    use iotkit_core_storage::Migration;

    use super::*;

    #[derive(Default)]
    struct TestClock {
        wall: AtomicI64,
        monotonic: AtomicU64,
        synchronized: AtomicBool,
    }

    impl TestClock {
        fn set(&self, wall: i64, monotonic: u64, synchronized: bool) {
            self.wall.store(wall, Ordering::SeqCst);
            self.monotonic.store(monotonic, Ordering::SeqCst);
            self.synchronized.store(synchronized, Ordering::SeqCst);
        }
    }

    impl Clock for TestClock {
        fn wall_time_ms(&self) -> i64 {
            self.wall.load(Ordering::SeqCst)
        }

        fn monotonic_ms(&self) -> u64 {
            self.monotonic.load(Ordering::SeqCst)
        }

        fn kernel_synchronized(&self) -> bool {
            self.synchronized.load(Ordering::SeqCst)
        }
    }

    fn migrations() -> Vec<Migration> {
        let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
        all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
        all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
        all.extend_from_slice(crate::MIGRATIONS);
        all.sort_by_key(|migration| migration.version);
        all
    }

    fn trust(conn: &Connection, clock: Arc<TestClock>) -> ClockTrust {
        ClockTrust::load(
            conn,
            clock,
            Duration::from_millis(10),
            Duration::from_millis(100),
        )
        .unwrap()
    }

    #[test]
    fn startup_is_untrusted_and_kernel_sync_cannot_cross_floor_backwards() {
        let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
        db.with_conn_sync(|conn| {
            conn.execute("UPDATE auth_state SET clock_floor_ms = 1_000", [])
                .unwrap();
            let clock = Arc::new(TestClock::default());
            clock.set(900, 0, true);
            let owner = trust(conn, clock.clone());
            assert_eq!(owner.evidence(), ClockEvidence::Untrusted);
            assert!(matches!(
                owner.trusted_now_and_advance(conn),
                Err(ClockTrustError::Untrusted)
            ));

            clock.set(1_100, 1, true);
            assert_eq!(owner.trusted_now_and_advance(conn).unwrap(), 1_100);
            assert_eq!(ClockTrust::persisted_floor(conn).unwrap(), 1_100);

            clock.set(1_050, 2, true);
            assert!(matches!(
                owner.trusted_now_and_advance(conn),
                Err(ClockTrustError::Untrusted)
            ));
            assert_eq!(ClockTrust::persisted_floor(conn).unwrap(), 1_100);

            let restarted = trust(conn, clock);
            assert_eq!(restarted.evidence(), ClockEvidence::Untrusted);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn manual_confirmation_recovers_only_an_already_running_owner() {
        let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let clock = Arc::new(TestClock::default());
            clock.set(2_000, 0, false);
            let owner = trust(conn, clock.clone());
            confirm_time_with_clock(conn, clock.as_ref(), 2_000).unwrap();
            assert_eq!(owner.trusted_now_and_advance(conn).unwrap(), 2_000);

            let restarted = trust(conn, clock);
            assert!(matches!(
                restarted.trusted_now_and_advance(conn),
                Err(ClockTrustError::Untrusted)
            ));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn manual_confirmation_rejects_between_display_rollback_without_advancing_floor() {
        let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
        db.with_conn_sync(|conn| {
            conn.execute("UPDATE auth_state SET clock_floor_ms = 1_000", [])
                .unwrap();
            let clock = TestClock::default();
            clock.set(1_999, 0, false);

            assert!(matches!(
                confirm_time_with_clock(conn, &clock, 2_000),
                Err(OpsError::ClockUntrusted)
            ));
            assert_eq!(ClockTrust::persisted_floor(conn).unwrap(), 1_000);
            assert_eq!(manual_evidence_at(conn).unwrap(), None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn manual_confirmation_rejects_forward_jump_without_advancing_floor() {
        let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
        db.with_conn_sync(|conn| {
            conn.execute("UPDATE auth_state SET clock_floor_ms = 1_000", [])
                .unwrap();
            let clock = TestClock::default();
            clock.set(2_000 + MAX_MANUAL_CONFIRMATION_DRIFT_MS + 1, 0, false);

            assert!(matches!(
                confirm_time_with_clock(conn, &clock, 2_000),
                Err(OpsError::ClockUntrusted)
            ));
            assert_eq!(ClockTrust::persisted_floor(conn).unwrap(), 1_000);
            assert_eq!(manual_evidence_at(conn).unwrap(), None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn manual_confirmation_accepts_zero_and_maximum_normal_delay_boundaries() {
        let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let clock = TestClock::default();
            clock.set(2_000, 0, false);
            confirm_time_with_clock(conn, &clock, 2_000).unwrap();
            assert_eq!(manual_evidence_at(conn).unwrap(), Some(2_000));

            let displayed_at_ms = 3_000;
            clock.set(displayed_at_ms + MAX_MANUAL_CONFIRMATION_DRIFT_MS, 0, false);
            confirm_time_with_clock(conn, &clock, displayed_at_ms).unwrap();
            assert_eq!(
                manual_evidence_at(conn).unwrap(),
                Some(displayed_at_ms + MAX_MANUAL_CONFIRMATION_DRIFT_MS)
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn floor_write_failure_fails_closed_and_periodic_checkpoint_is_bounded() {
        let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let clock = Arc::new(TestClock::default());
            clock.set(3_000, 0, true);
            let owner = trust(conn, clock.clone());
            assert!(!owner.checkpoint_if_due(conn).unwrap());
            clock.set(3_100, 100, true);
            assert!(owner.checkpoint_if_due(conn).unwrap());
            assert_eq!(ClockTrust::persisted_floor(conn).unwrap(), 3_100);

            conn.execute_batch(
                "CREATE TRIGGER fail_clock_floor BEFORE UPDATE OF clock_floor_ms ON auth_state
                 WHEN NEW.clock_floor_ms > OLD.clock_floor_ms
                 BEGIN SELECT RAISE(ABORT, 'injected floor failure'); END;",
            )
            .unwrap();
            clock.set(3_200, 200, true);
            assert!(owner.trusted_now_and_advance(conn).is_err());
            assert_eq!(ClockTrust::persisted_floor(conn).unwrap(), 3_100);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn manual_confirmation_audit_failure_rolls_back_evidence() {
        let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
        db.with_conn_sync(|conn| {
            let clock = TestClock::default();
            clock.set(4_000, 0, false);
            conn.execute_batch(
                "CREATE TRIGGER fail_clock_audit BEFORE INSERT ON ledger_events
                 WHEN NEW.kind = 'clock_trust_confirmed'
                 BEGIN SELECT RAISE(ABORT, 'injected clock audit failure'); END;",
            )
            .unwrap();
            assert!(confirm_time_with_clock(conn, &clock, 4_000).is_err());
            let sequence: i64 = conn
                .query_row(
                    "SELECT manual_evidence_seq FROM auth_state WHERE id = 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(sequence, 0);
            Ok(())
        })
        .unwrap();
    }
}
