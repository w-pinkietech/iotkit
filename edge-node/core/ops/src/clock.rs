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
#[path = "../tests/unit/clock_tests.rs"]
mod tests;
