use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use rusqlite::{OptionalExtension, params};

use super::*;
use crate::tier::{ActorKind, Tier, TokenKind};
use iotkit_core_storage::Migration;

#[derive(Default)]
struct TestClock {
    wall: AtomicI64,
    monotonic: AtomicU64,
    synchronized: AtomicBool,
}

impl TestClock {
    fn set_wall(&self, value: i64) {
        self.wall.store(value, Ordering::SeqCst);
    }
}

impl crate::Clock for TestClock {
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

fn test_clock(conn: &Connection, now: i64) -> (Arc<TestClock>, ClockTrust) {
    let clock = Arc::new(TestClock::default());
    clock.set_wall(now);
    clock.synchronized.store(true, Ordering::SeqCst);
    let trust = ClockTrust::load(
        conn,
        clock.clone(),
        Duration::from_millis(10),
        Duration::from_secs(60),
    )
    .unwrap();
    (clock, trust)
}

fn all_migrations() -> Vec<Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(crate::MIGRATIONS);
    all.sort_by_key(|m| m.version);
    all
}

fn event_details(conn: &rusqlite::Connection, kind: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT detail FROM ledger_events WHERE kind = ?1 ORDER BY event_id")
        .unwrap();
    stmt.query_map([kind], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn token_last_used(conn: &rusqlite::Connection, token_id: &str) -> Option<i64> {
    conn.query_row(
        "SELECT last_used_at FROM operator_tokens WHERE token_id = ?1",
        [token_id],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn local_passphrase_reset_establishes_ownership_and_audit_does_not_expose_secret() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(ownership_state(conn).unwrap(), OwnershipState::Unowned);
        let phc = hash_passphrase("correct horse battery staple").unwrap();
        reset_passphrase_with_hash(conn, &phc, "local_cli").unwrap();
        assert_eq!(ownership_state(conn).unwrap(), OwnershipState::Owned);

        let stored = load_passphrase_hash(conn).unwrap().unwrap();
        assert_eq!(stored, phc);
        assert!(verify_passphrase(&stored, "correct horse battery staple"));
        assert!(!verify_passphrase(&stored, "wrong passphrase"));

        let new_hash = hash_passphrase("new passphrase").unwrap();
        reset_passphrase_with_hash(conn, &new_hash, "local_cli").unwrap();
        let updated = load_passphrase_hash(conn).unwrap().unwrap();
        assert!(verify_passphrase(&updated, "new passphrase"));
        assert!(!verify_passphrase(&updated, "correct horse battery staple"));

        let reset_details = event_details(conn, "admin_passphrase_reset");
        assert_eq!(reset_details.len(), 2);
        let reset_detail: serde_json::Value = serde_json::from_str(&reset_details[1]).unwrap();
        assert_eq!(reset_detail["actor"], "local_cli");
        for detail in reset_details {
            assert!(!detail.contains("new passphrase"));
            assert!(!detail.contains("correct horse battery staple"));
            assert!(!detail.contains("passphrase_hash"));
            assert!(!detail.contains("$argon2"));
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn passphrase_reset_revokes_all_operator_and_session_authority() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (_clock, trust) = test_clock(conn, 1_000);
        let operator = issue_token(
            conn,
            &NewOperatorToken {
                name: "operator".to_string(),
                kind: TokenKind::Human,
                ceiling: Tier::Daily,
                is_session: false,
                expires_at: None,
            },
            "local_cli",
            None,
            None,
        )
        .unwrap();
        let session = issue_token(
            conn,
            &NewOperatorToken {
                name: "session".to_string(),
                kind: TokenKind::Human,
                ceiling: Tier::Construction,
                is_session: true,
                expires_at: Some(10_000),
            },
            "local_cli",
            None,
            Some(&trust),
        )
        .unwrap();

        let hash = hash_passphrase("replacement passphrase").unwrap();
        reset_passphrase_with_hash(conn, &hash, "local_cli").unwrap();

        assert!(
            authenticate(conn, operator.plaintext.expose(), &trust)
                .unwrap()
                .is_none()
        );
        assert!(
            authenticate(conn, session.plaintext.expose(), &trust)
                .unwrap()
                .is_none()
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn passphrase_reset_audit_failure_preserves_prior_authority_atomically() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (_clock, trust) = test_clock(conn, 1_000);
        let original_hash = hash_passphrase("original passphrase").unwrap();
        reset_passphrase_with_hash(conn, &original_hash, "local_cli").unwrap();
        let issued = issue_token(
            conn,
            &NewOperatorToken {
                name: "surviving operator".into(),
                kind: TokenKind::Human,
                ceiling: Tier::Routine,
                is_session: false,
                expires_at: None,
            },
            "local_cli",
            None,
            None,
        )
        .unwrap();
        let generation = auth_generation(conn).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_reset_audit BEFORE INSERT ON ledger_events
                 WHEN NEW.kind = 'admin_passphrase_reset'
                 BEGIN SELECT RAISE(ABORT, 'injected audit failure'); END;",
        )
        .unwrap();

        let replacement_hash = hash_passphrase("replacement passphrase").unwrap();
        assert!(reset_passphrase_with_hash(conn, &replacement_hash, "local_cli").is_err());

        assert_eq!(load_passphrase_hash(conn).unwrap().unwrap(), original_hash);
        assert_eq!(auth_generation(conn).unwrap(), generation);
        assert!(
            authenticate(conn, issued.plaintext.expose(), &trust)
                .unwrap()
                .is_some()
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn concurrent_passphrase_resets_serialize_and_leave_one_committed_credential() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.db");
    let db = iotkit_core_storage::init_db(&path, &all_migrations()).unwrap();
    let first = hash_passphrase("first concurrent passphrase").unwrap();
    let second = hash_passphrase("second concurrent passphrase").unwrap();
    let mut workers = Vec::new();
    for hash in [first.clone(), second.clone()] {
        let path = path.clone();
        workers.push(std::thread::spawn(move || {
            let conn = Connection::open(path).unwrap();
            conn.busy_timeout(Duration::from_secs(5)).unwrap();
            reset_passphrase_with_hash(&conn, &hash, "local_cli").unwrap();
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    db.with_conn_sync(|conn| {
        let committed = load_passphrase_hash(conn).unwrap().unwrap();
        assert!(committed == first || committed == second);
        assert_eq!(auth_generation(conn).unwrap(), 2);
        Ok(())
    })
    .unwrap();
}

#[test]
fn token_issue_authenticate_expire_revoke_and_audit_do_not_expose_secret() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (clock, trust) = test_clock(conn, 1_000);
        let issued = issue_token(
            conn,
            &NewOperatorToken {
                name: "daily human".to_string(),
                kind: TokenKind::Human,
                ceiling: Tier::Daily,
                is_session: false,
                expires_at: None,
            },
            "local_cli",
            Some("127.0.0.1"),
            None,
        )
        .unwrap();
        assert_eq!(format!("{:?}", issued.plaintext), "[REDACTED]");
        assert_eq!(issued.token_id.len(), 26);
        assert!(issued.token_id.starts_with("tok_"));
        assert_eq!(issued.plaintext.expose().len(), 47);
        assert!(issued.plaintext.expose().starts_with("iko_"));

        let actor = authenticate(conn, issued.plaintext.expose(), &trust)
            .unwrap()
            .unwrap();
        assert_eq!(actor.actor_id, issued.token_id);
        assert_eq!(actor.actor_kind, ActorKind::Human);
        assert_eq!(actor.tier_ceiling, Tier::Daily);

        let expired = issue_token(
            conn,
            &NewOperatorToken {
                name: "expired".to_string(),
                kind: TokenKind::Human,
                ceiling: Tier::Routine,
                is_session: true,
                expires_at: Some(5_000),
            },
            "local_cli",
            None,
            Some(&trust),
        )
        .unwrap();
        clock.set_wall(5_001);
        assert!(
            authenticate(conn, expired.plaintext.expose(), &trust)
                .unwrap()
                .is_none()
        );

        revoke_token(conn, &issued.token_id, "local_cli").unwrap();
        assert!(
            authenticate(conn, issued.plaintext.expose(), &trust)
                .unwrap()
                .is_none()
        );

        let issue_details = event_details(conn, "operator_token_issued");
        let session_details = event_details(conn, "auth_session_issued");
        let revoke_details = event_details(conn, "operator_token_revoked");
        assert_eq!(issue_details.len(), 1);
        assert_eq!(session_details.len(), 1);
        assert_eq!(revoke_details.len(), 1);
        assert!(issue_details[0].contains(&issued.token_id));
        assert!(issue_details[0].contains("daily human"));
        assert!(issue_details[0].contains("127.0.0.1"));
        assert!(session_details[0].contains(&expired.token_id));
        assert!(session_details[0].contains("expired"));
        assert!(revoke_details[0].contains(&issued.token_id));
        let issue_detail: serde_json::Value = serde_json::from_str(&issue_details[0]).unwrap();
        let session_detail: serde_json::Value = serde_json::from_str(&session_details[0]).unwrap();
        let revoke_detail: serde_json::Value = serde_json::from_str(&revoke_details[0]).unwrap();
        assert_eq!(issue_detail["actor"], "local_cli");
        assert_eq!(issue_detail["source"], "127.0.0.1");
        assert_eq!(session_detail["actor"], "local_cli");
        assert!(session_detail["source"].is_null());
        assert_eq!(revoke_detail["actor"], "local_cli");
        for detail in issue_details
            .into_iter()
            .chain(session_details)
            .chain(revoke_details)
        {
            assert!(!detail.contains(issued.plaintext.expose()));
            assert!(!detail.contains(expired.plaintext.expose()));
            assert!(!detail.contains("token_hash"));
            assert!(!detail.contains("hash"));
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn authenticate_rejects_token_expiring_at_now() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (clock, trust) = test_clock(conn, 4_000);
        let issued = issue_token(
            conn,
            &NewOperatorToken {
                name: "boundary".to_string(),
                kind: TokenKind::Human,
                ceiling: Tier::Routine,
                is_session: false,
                expires_at: Some(5_000),
            },
            "local_cli",
            None,
            Some(&trust),
        )
        .unwrap();

        clock.set_wall(5_000);
        assert!(
            authenticate(conn, issued.plaintext.expose(), &trust)
                .unwrap()
                .is_none()
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn finite_auth_floor_failure_rejects_without_updating_session_state() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (clock, trust) = test_clock(conn, 4_000);
        let issued = issue_token(
            conn,
            &NewOperatorToken {
                name: "finite".into(),
                kind: TokenKind::Human,
                ceiling: Tier::Routine,
                is_session: true,
                expires_at: Some(10_000),
            },
            "local_cli",
            None,
            Some(&trust),
        )
        .unwrap();
        let floor = ClockTrust::persisted_floor(conn).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_auth_floor BEFORE UPDATE OF clock_floor_ms ON auth_state
                 WHEN NEW.clock_floor_ms > OLD.clock_floor_ms
                 BEGIN SELECT RAISE(ABORT, 'injected auth floor failure'); END;",
        )
        .unwrap();
        clock.set_wall(5_000);
        let tx =
            rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate).unwrap();
        assert!(authenticate(&tx, issued.plaintext.expose(), &trust).is_err());
        drop(tx);
        assert_eq!(ClockTrust::persisted_floor(conn).unwrap(), floor);
        assert_eq!(token_last_used(conn, &issued.token_id), None);
        Ok(())
    })
    .unwrap();
}

#[test]
fn concurrent_finite_auth_transactions_share_one_clock_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("concurrent-auth.db");
    let db = iotkit_core_storage::init_db(&path, &all_migrations()).unwrap();
    let (plaintext, trust) = db
        .with_conn_sync(|conn| {
            let (_clock, trust) = test_clock(conn, 7_000);
            let trust = Arc::new(trust);
            let issued = issue_session_token(
                conn,
                "concurrent session",
                Tier::Routine,
                60_000,
                "local_cli",
                None,
                &trust,
            )
            .unwrap();
            Ok((issued.plaintext.expose().to_string(), trust))
        })
        .unwrap();

    let mut workers = Vec::new();
    for _ in 0..8 {
        let path = path.clone();
        let plaintext = plaintext.clone();
        let trust = trust.clone();
        workers.push(std::thread::spawn(move || {
            let conn = Connection::open(path).unwrap();
            conn.busy_timeout(Duration::from_secs(5)).unwrap();
            let tx = rusqlite::Transaction::new_unchecked(&conn, TransactionBehavior::Immediate)
                .unwrap();
            assert!(authenticate(&tx, &plaintext, &trust).unwrap().is_some());
            tx.commit().unwrap();
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    db.with_conn_sync(|conn| {
        assert_eq!(ClockTrust::persisted_floor(conn).unwrap(), 7_000);
        Ok(())
    })
    .unwrap();
}

#[test]
fn ai_tokens_above_routine_are_rejected_before_database_insert() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let result = issue_token(
            conn,
            &NewOperatorToken {
                name: "ai daily".to_string(),
                kind: TokenKind::Ai,
                ceiling: Tier::Daily,
                is_session: false,
                expires_at: None,
            },
            "local_cli",
            None,
            None,
        );
        assert!(matches!(result, Err(crate::OpsError::Validation(_))));
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM operator_tokens", [], |row| row.get(0))
            .unwrap();
        let audit_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_events WHERE kind = 'operator_token_issued'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(audit_count, 0);
        Ok(())
    })
    .unwrap();
}

#[test]
fn authenticate_throttles_last_used_updates_to_sixty_seconds() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (clock, trust) = test_clock(conn, 1_000);
        let issued = issue_token(
            conn,
            &NewOperatorToken {
                name: "routine".to_string(),
                kind: TokenKind::Human,
                ceiling: Tier::Routine,
                is_session: false,
                expires_at: None,
            },
            "local_cli",
            None,
            None,
        )
        .unwrap();

        assert!(
            authenticate(conn, issued.plaintext.expose(), &trust)
                .unwrap()
                .is_some()
        );
        assert_eq!(token_last_used(conn, &issued.token_id), Some(1_000));

        assert!(
            authenticate(conn, issued.plaintext.expose(), &trust)
                .unwrap()
                .is_some()
        );
        assert_eq!(token_last_used(conn, &issued.token_id), Some(1_000));

        clock.set_wall(61_001);
        assert!(
            authenticate(conn, issued.plaintext.expose(), &trust)
                .unwrap()
                .is_some()
        );
        assert_eq!(token_last_used(conn, &issued.token_id), Some(61_001));
        Ok(())
    })
    .unwrap();
}

#[test]
fn list_tokens_omits_hash_and_plaintext() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let issued = issue_token(
            conn,
            &NewOperatorToken {
                name: "listed".to_string(),
                kind: TokenKind::Ai,
                ceiling: Tier::Routine,
                is_session: false,
                expires_at: None,
            },
            "local_cli",
            None,
            None,
        )
        .unwrap();
        let rows = list_tokens(conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].token_id, issued.token_id);
        assert_eq!(rows[0].name, "listed");
        assert_eq!(rows[0].kind, TokenKind::Ai);
        assert_eq!(rows[0].tier_ceiling, Tier::Routine);

        let leaked_hash: Option<Vec<u8>> = conn
            .query_row(
                "SELECT token_hash FROM operator_tokens WHERE token_id = ?1",
                params![rows[0].token_id],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert!(
            leaked_hash.is_some(),
            "test must prove the DB row has a hash"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn old_passphrase_verification_cannot_issue_after_concurrent_reset() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let old_hash = hash_passphrase("old-passphrase").unwrap();
        reset_passphrase_with_hash(conn, &old_hash, "local_cli").unwrap();
        let pre_hash_authority = load_passphrase_authority(conn).unwrap().unwrap();
        assert!(verify_passphrase(&pre_hash_authority.phc, "old-passphrase"));

        let new_hash = hash_passphrase("new-passphrase").unwrap();
        reset_passphrase_with_hash(conn, &new_hash, "local_cli").unwrap();

        let tx =
            rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate).unwrap();
        assert!(matches!(
            require_passphrase_authority_unchanged(&tx, &pre_hash_authority),
            Err(OpsError::Forbidden)
        ));
        let sessions: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM operator_tokens WHERE is_session = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sessions, 0);
        Ok(())
    })
    .unwrap();
}

#[test]
fn losing_credential_after_ownership_requires_local_recovery() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let hash = hash_passphrase("owned-passphrase").unwrap();
        reset_passphrase_with_hash(conn, &hash, "local_cli").unwrap();
        conn.execute("DELETE FROM admin_credential", []).unwrap();
        assert_eq!(
            ownership_state(conn).unwrap(),
            OwnershipState::LocalRecoveryRequired
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn external_initialization_marker_distinguishes_first_init_from_database_loss() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edge.db");
    let first = iotkit_core_storage::init_db(&path, &all_migrations()).unwrap();
    first
        .with_conn_sync(|conn| {
            reconcile_database_initialization_provenance(conn, &path, false).unwrap();
            assert_eq!(ownership_state(conn).unwrap(), OwnershipState::Unowned);
            Ok(())
        })
        .unwrap();
    drop(first);
    std::fs::remove_file(&path).unwrap();

    let recreated = iotkit_core_storage::init_db(&path, &all_migrations()).unwrap();
    recreated
        .with_conn_sync(|conn| {
            reconcile_database_initialization_provenance(conn, &path, false).unwrap();
            assert_eq!(
                ownership_state(conn).unwrap(),
                OwnershipState::LocalRecoveryRequired
            );
            Ok(())
        })
        .unwrap();
}
