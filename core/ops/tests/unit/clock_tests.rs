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
