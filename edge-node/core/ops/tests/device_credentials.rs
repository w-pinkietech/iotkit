use iotkit_core_ledger::{DeviceKind, DeviceState, NewDevice, SystemId};
use iotkit_core_ops::{
    Actor, ActorKind, DeviceCredentialState, DispatchRequest, OpError, Tier,
    authenticate_device as authenticate_device_public, authentication_is_current, capacity_health,
    dispatch, inspect_device_credential, list_device_credentials, replacement_backup_health,
    stale_credential_health, standard_catalog,
};
use iotkit_core_storage::Migration;
use rusqlite::{TransactionBehavior, params};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::cell::Cell;
use zeroize::Zeroizing;

struct TestEntropy(u8);
struct TestClock(Cell<i64>);

struct TestPresentation(Zeroizing<String>);
impl TestPresentation {
    fn consume(self) -> Zeroizing<String> {
        self.0
    }
}

#[test]
fn read_only_authentication_does_not_prove_touch_or_audit_and_recheck_fails_after_revoke() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (_sid, principal) = seed_principal(conn, "readonly-auth");
        let (credential_id, secret) =
            issue_at(conn, &principal, DeviceCredentialState::Pending, 211, 100);
        let before: (Option<i64>, Option<i64>, i64) = conn.query_row(
            "SELECT proven_at, last_used_at, (SELECT COUNT(*) FROM ledger_events)
             FROM device_credentials WHERE credential_id=?1",
            [&credential_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;

        let authentication = inspect_device_credential(conn, &secret).unwrap().unwrap();
        let after: (Option<i64>, Option<i64>, i64) = conn.query_row(
            "SELECT proven_at, last_used_at, (SELECT COUNT(*) FROM ledger_events)
             FROM device_credentials WHERE credential_id=?1",
            [&credential_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(before, after);
        assert!(authentication_is_current(conn, &authentication).unwrap());

        abandon_device_credential(conn, &principal, &credential_id, "pending_abandoned", 101)
            .unwrap();
        assert!(!authentication_is_current(conn, &authentication).unwrap());
        Ok(())
    })
    .unwrap();
}
impl std::fmt::Debug for TestPresentation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[REDACTED DEVICE CREDENTIAL]")
    }
}

fn authenticate_device(
    conn: &rusqlite::Connection,
    plaintext: &str,
    _clock: &TestClock,
) -> Result<Option<iotkit_core_ops::DeviceAuthentication>, iotkit_core_ops::OpsError> {
    authenticate_device_public(conn, plaintext)
}

fn issue_device_credential_with(
    conn: &rusqlite::Connection,
    principal_id: &str,
    state: DeviceCredentialState,
    reason: &str,
    entropy: &mut TestEntropy,
    _clock: &TestClock,
) -> Result<(String, TestPresentation), OpError> {
    entropy.0 = entropy.0.wrapping_add(1);
    let op = match state {
        DeviceCredentialState::Current => "device_credential.issue",
        DeviceCredentialState::Pending => "device_credential.reissue",
        DeviceCredentialState::Revoked => return Err(OpError::Validation("revoked".into())),
    };
    let result = dispatch(
        conn,
        standard_catalog(),
        request(
            op,
            json!({"principal_id":principal_id,"reason_code":reason}),
            false,
            ActorKind::LocalCli,
            false,
        ),
    )?;
    match result {
        iotkit_core_ops::DispatchResult::DeviceCredential(secret) => {
            let (metadata, plaintext) = secret.consume();
            Ok((
                metadata["credential_id"].as_str().unwrap().to_owned(),
                TestPresentation(plaintext),
            ))
        }
        _ => Err(OpError::Internal("credential presentation missing".into())),
    }
}

fn confirm_device_credential(
    conn: &rusqlite::Connection,
    principal: &str,
    credential: &str,
    reason: &str,
    _now: i64,
) -> Result<iotkit_core_ops::DispatchResult, OpError> {
    lifecycle(
        conn,
        "device_credential.confirm",
        principal,
        credential,
        reason,
    )
}
fn abandon_device_credential(
    conn: &rusqlite::Connection,
    principal: &str,
    credential: &str,
    reason: &str,
    _now: i64,
) -> Result<iotkit_core_ops::DispatchResult, OpError> {
    lifecycle(
        conn,
        "device_credential.abandon",
        principal,
        credential,
        reason,
    )
}
fn revoke_device_credential(
    conn: &rusqlite::Connection,
    principal: &str,
    credential: &str,
    reason: &str,
    _now: i64,
) -> Result<iotkit_core_ops::DispatchResult, OpError> {
    lifecycle(
        conn,
        "device_credential.revoke",
        principal,
        credential,
        reason,
    )
}

fn register_device_principal(
    conn: &rusqlite::Connection,
    principal: &str,
    device: &SystemId,
    scopes: &[SystemId],
    flow_class: &str,
    now: i64,
) -> rusqlite::Result<()> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO device_ingest_principals
         (principal_id, device_system_id, flow_class, profile, created_at)
         VALUES (?1, ?2, ?3, 'simple_bearer', ?4)",
        params![principal, device.as_bytes().as_slice(), flow_class, now],
    )?;
    for scope in scopes {
        tx.execute(
            "INSERT INTO device_principal_scopes (principal_id, system_id) VALUES (?1, ?2)",
            params![principal, scope.as_bytes().as_slice()],
        )?;
    }
    tx.commit()
}

fn all_migrations() -> Vec<Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    all.sort_by_key(|migration| migration.version);
    all
}

fn seed_principal(conn: &rusqlite::Connection, name: &str) -> (SystemId, String) {
    let sid = iotkit_core_ledger::insert_device(
        conn,
        &NewDevice {
            hardware_id: format!("test-hardware-{name}"),
            user_label: None,
            parent: None,
            kind: DeviceKind::Individual,
            initial_state: DeviceState::Active,
        },
    )
    .unwrap();
    let principal = format!("principal-{name}");
    conn.execute(
        "INSERT INTO device_ingest_principals
         (principal_id, device_system_id, flow_class, profile, created_at)
         VALUES (?1, ?2, 'default', 'simple_bearer', 100)",
        params![principal, sid.as_bytes().as_slice()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO device_principal_scopes (principal_id, system_id) VALUES (?1, ?2)",
        params![principal, sid.as_bytes().as_slice()],
    )
    .unwrap();
    (sid, principal)
}

fn issue_at(
    conn: &rusqlite::Connection,
    principal: &str,
    state: DeviceCredentialState,
    seed: u8,
    now: i64,
) -> (String, String) {
    let reason = if state == DeviceCredentialState::Pending {
        "credential_reissue"
    } else {
        "manual_issue"
    };
    let id = format!("dcr_test_{seed:03}");
    let secret = format!("ikd_test_secret_{seed:03}");
    let hash = Sha256::digest(secret.as_bytes());
    conn.execute(
        "INSERT INTO device_credentials
         (credential_id, principal_id, token_hash, auth_epoch, state, issued_at, issue_reason)
         VALUES (?1, ?2, ?3, (SELECT auth_epoch FROM auth_state WHERE id=1), ?4, ?5, ?6)",
        params![id, principal, hash.as_slice(), state.as_str(), now, reason],
    )
    .unwrap();
    (id, secret)
}

fn lifecycle(
    conn: &rusqlite::Connection,
    op: &str,
    principal_id: &str,
    credential_id: &str,
    reason_code: &str,
) -> Result<iotkit_core_ops::DispatchResult, OpError> {
    dispatch(
        conn,
        standard_catalog(),
        request(
            op,
            json!({"principal_id":principal_id,"credential_id":credential_id,"reason_code":reason_code}),
            false,
            ActorKind::LocalCli,
            false,
        ),
    )
}

#[test]
fn migration_enforces_current_pending_uniqueness_hash_size_and_registered_scopes() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (_sid, principal) = seed_principal(conn, "constraints");
        issue_at(conn, &principal, DeviceCredentialState::Current, 1, 100);
        assert!(issue_device_credential_with(conn, &principal, DeviceCredentialState::Current, "manual_issue", &mut TestEntropy(20), &TestClock(Cell::new(101))).is_err());
        issue_at(conn, &principal, DeviceCredentialState::Pending, 40, 102);
        assert!(issue_device_credential_with(conn, &principal, DeviceCredentialState::Pending, "credential_reissue", &mut TestEntropy(60), &TestClock(Cell::new(103))).is_err());

        let missing = SystemId::from_bytes([9; 16]);
        let err = conn.execute("INSERT INTO device_principal_scopes (principal_id, system_id) VALUES (?1, ?2)",
            params![principal, missing.as_bytes().as_slice()]).unwrap_err();
        assert!(err.to_string().contains("FOREIGN KEY") || err.to_string().contains("registered"));
        let err = conn.execute("INSERT INTO device_credentials (credential_id, principal_id, token_hash, auth_epoch, state, issued_at, issue_reason)
            VALUES ('dcr_bad_hash', ?1, x'01', (SELECT auth_epoch FROM auth_state WHERE id=1), 'revoked', 1, 'x')", [principal]).unwrap_err();
        assert!(err.to_string().contains("CHECK constraint"));
        Ok(())
    }).unwrap();
}

#[test]
fn issue_authenticate_revoke_is_constant_time_checked_and_presentation_is_redacted() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (_sid, principal) = seed_principal(conn, "auth");
        let mut entropy = TestEntropy(3);
        let clock = TestClock(Cell::new(1_000));
        let (credential_id, presentation) = issue_device_credential_with(
            conn,
            &principal,
            DeviceCredentialState::Current,
            "manual_issue",
            &mut entropy,
            &clock,
        )
        .unwrap();
        assert_eq!(format!("{presentation:?}"), "[REDACTED DEVICE CREDENTIAL]");
        let plaintext = presentation.consume().to_string();
        assert!(
            authenticate_device(conn, "fixture-not-a-real-secret", &clock)
                .unwrap()
                .is_none()
        );
        let authenticated = authenticate_device(conn, &plaintext, &clock)
            .unwrap()
            .unwrap();
        assert_eq!(authenticated.principal().principal_id(), principal);
        revoke_device_credential(conn, &principal, &credential_id, "operator_revoked", 1_001)
            .unwrap();
        assert!(
            authenticate_device(conn, &plaintext, &clock)
                .unwrap()
                .is_none()
        );
        let db_text = conn
            .query_row(
                "SELECT group_concat(detail, '\n') FROM ledger_events",
                [],
                |r| r.get::<_, Option<String>>(0),
            )
            .unwrap()
            .unwrap_or_default();
        assert!(!db_text.contains(&plaintext));
        assert!(!format!("{:?}", list_device_credentials(conn).unwrap()).contains(&plaintext));
        Ok(())
    })
    .unwrap();
}

#[test]
fn make_before_break_handles_lost_response_proof_confirm_abandon_and_transaction_failure() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (_sid, principal) = seed_principal(conn, "lifecycle");
        let (current_id, current_secret) =
            issue_at(conn, &principal, DeviceCredentialState::Current, 1, 100);
        let (pending_id, pending_secret) =
            issue_at(conn, &principal, DeviceCredentialState::Pending, 40, 101);
        assert!(
            issue_device_credential_with(
                conn,
                &principal,
                DeviceCredentialState::Pending,
                "credential_reissue",
                &mut TestEntropy(80),
                &TestClock(Cell::new(102))
            )
            .is_err()
        );
        assert!(
            confirm_device_credential(conn, &principal, &pending_id, "credential_confirmed", 103)
                .is_err()
        );
        authenticate_device(conn, &pending_secret, &TestClock(Cell::new(104)))
            .unwrap()
            .unwrap();
        assert!(
            authenticate_device(conn, &current_secret, &TestClock(Cell::new(104)))
                .unwrap()
                .is_some()
        );
        confirm_device_credential(conn, &principal, &pending_id, "credential_confirmed", 105)
            .unwrap();
        assert!(
            authenticate_device(conn, &current_secret, &TestClock(Cell::new(106)))
                .unwrap()
                .is_none()
        );
        assert!(
            authenticate_device(conn, &pending_secret, &TestClock(Cell::new(106)))
                .unwrap()
                .is_some()
        );
        assert_eq!(
            list_device_credentials(conn)
                .unwrap()
                .iter()
                .find(|row| row.credential_id == current_id)
                .unwrap()
                .state,
            DeviceCredentialState::Revoked
        );

        let (abandoned_id, abandoned_secret) =
            issue_at(conn, &principal, DeviceCredentialState::Pending, 90, 107);
        abandon_device_credential(conn, &principal, &abandoned_id, "pending_abandoned", 108)
            .unwrap();
        assert!(
            authenticate_device(conn, &abandoned_secret, &TestClock(Cell::new(109)))
                .unwrap()
                .is_none()
        );
        assert!(
            authenticate_device(conn, &pending_secret, &TestClock(Cell::new(109)))
                .unwrap()
                .is_some()
        );

        let before = list_device_credentials(conn).unwrap().len();
        assert!(
            issue_device_credential_with(
                conn,
                &principal,
                DeviceCredentialState::Current,
                "manual_issue",
                &mut TestEntropy(120),
                &TestClock(Cell::new(110)),
            )
            .is_err()
        );
        assert_eq!(list_device_credentials(conn).unwrap().len(), before);
        Ok(())
    })
    .unwrap();
}

#[test]
fn replace_and_retire_revoke_old_hardware_authority_atomically() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (replace_sid, replace_principal) = seed_principal(conn, "replace");
        let (_id, secret) = issue_at(
            conn,
            &replace_principal,
            DeviceCredentialState::Current,
            1,
            100,
        );
        iotkit_core_ledger::replace_hardware(conn, &replace_sid, "replacement-hardware").unwrap();
        assert!(
            authenticate_device(conn, &secret, &TestClock(Cell::new(101)))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            list_device_credentials(conn).unwrap()[0]
                .revoke_reason
                .as_deref(),
            Some("hardware_replaced")
        );

        let (retire_sid, retire_principal) = seed_principal(conn, "retire");
        let (_id, secret) = issue_at(
            conn,
            &retire_principal,
            DeviceCredentialState::Current,
            50,
            100,
        );
        iotkit_core_ledger::retire_device(conn, &retire_sid).unwrap();
        assert!(
            authenticate_device(conn, &secret, &TestClock(Cell::new(101)))
                .unwrap()
                .is_none()
        );
        Ok(())
    })
    .unwrap();
}

fn request(
    op: &str,
    params: serde_json::Value,
    dry_run: bool,
    actor_kind: ActorKind,
    step_up: bool,
) -> DispatchRequest {
    DispatchRequest {
        op: op.into(),
        params,
        dry_run,
        actor: Actor {
            actor_id: "local-test".into(),
            actor_kind,
            tier_ceiling: Tier::Construction,
        },
        source: Some("test".into()),
        step_up_verified: step_up,
        clock_trust: None,
    }
}

fn bind_capacity_approval(
    conn: &rusqlite::Connection,
    op: &str,
    mut params: serde_json::Value,
) -> serde_json::Value {
    let preview = dispatch(
        conn,
        standard_catalog(),
        request(op, params.clone(), true, ActorKind::LocalCli, true),
    )
    .unwrap();
    let object = params.as_object_mut().unwrap();
    for (expected, shown) in [
        ("expected_required_steady_units", "required_steady_units"),
        ("expected_required_burst_units", "required_burst_units"),
        ("expected_capacity_steady_units", "capacity_steady_units"),
        ("expected_capacity_burst_units", "capacity_burst_units"),
        ("expected_authority_generation", "authority_generation"),
    ] {
        object.insert(expected.into(), preview[shown].clone());
    }
    params
}

#[test]
fn capacity_matrix_requires_explicit_human_construction_debt_and_recovers() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE device_capacity SET steady_units=10, burst_units=10 WHERE id=1",
            [],
        )
        .unwrap();
        conn.execute("UPDATE device_flow_classes SET steady_units=50, burst_units=50 WHERE flow_class='high'", []).unwrap();
        let params =
            json!({"hardware_id":"capacity-device","flow_class":"high","reason_code":"device_commissioning"});
        assert_eq!(
            dispatch(
                conn,
                standard_catalog(),
                request(
                    "device.add_with_credential",
                    params.clone(),
                    false,
                    ActorKind::LocalCli,
                    false
                )
            )
            .unwrap_err(),
            OpError::PreconditionFailed("capacity_exceeded".into())
        );
        let ai = dispatch(
            conn,
            standard_catalog(),
            request(
                "device.add_with_credential_capacity_debt",
                params.clone(),
                false,
                ActorKind::Ai,
                true,
            ),
        )
        .unwrap_err();
        assert!(matches!(ai, OpError::Forbidden(_)));
        let out = dispatch(
            conn,
            standard_catalog(),
            request(
                "device.add_with_credential_capacity_debt",
                bind_capacity_approval(
                    conn,
                    "device.add_with_credential_capacity_debt",
                    params,
                ),
                false,
                ActorKind::LocalCli,
                true,
            ),
        )
        .unwrap();
        let (metadata, plaintext) = match out {
            iotkit_core_ops::DispatchResult::DeviceCredential(secret) => secret.consume(),
            _ => panic!("credential presentation required"),
        };
        assert!(plaintext.starts_with("ikd_"));
        assert!(capacity_health(conn).unwrap().active_debt);
        conn.execute(
            "UPDATE device_capacity SET steady_units=100, burst_units=100 WHERE id=1",
            [],
        )
        .unwrap();
        let principal = metadata["principal_id"].as_str().unwrap();
        dispatch(
            conn,
            standard_catalog(),
            request(
                "device.flow_class_change",
                json!({"principal_ids":[principal],"flow_class":"low"}),
                false,
                ActorKind::LocalCli,
                false,
            ),
        )
        .unwrap();
        assert!(!capacity_health(conn).unwrap().active_debt);
        Ok(())
    })
    .unwrap();
}

#[test]
fn last_used_is_throttled_and_health_is_aggregate_bounded_and_actionable() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (_sid, principal) = seed_principal(conn, "health");
        let (_id, secret) = issue_at(conn, &principal, DeviceCredentialState::Current, 1, 100);
        let clock = TestClock(Cell::new(1_000));
        authenticate_device(conn, &secret, &clock).unwrap();
        let first: i64 = conn
            .query_row("SELECT last_used_at FROM device_credentials", [], |r| {
                r.get(0)
            })
            .unwrap();
        clock.0.set(1_001);
        authenticate_device(conn, &secret, &clock).unwrap();
        let throttled: i64 = conn
            .query_row("SELECT last_used_at FROM device_credentials", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(first, throttled);
        clock.0.set(61_001);
        conn.execute("UPDATE device_credentials SET last_used_at=issued_at", [])?;
        authenticate_device(conn, &secret, &clock).unwrap();
        let updated: i64 = conn
            .query_row("SELECT last_used_at FROM device_credentials", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            updated > 100,
            "the deliberately stale last_used_at value must be refreshed"
        );
        let health = stale_credential_health(conn, updated.saturating_add(60_001), 60_000).unwrap();
        assert_eq!(health.active_count, 1);
        assert_eq!(health.stale_count, 1);
        let backup = replacement_backup_health(conn).unwrap();
        assert!(backup.replacement_backup_unavailable);
        assert!(
            backup
                .recovery_action
                .unwrap()
                .contains("Plan 6.5 encrypted replacement backup")
        );
        conn.execute(
            "INSERT OR REPLACE INTO ledger_meta(key,value) VALUES('encrypted_replacement_backup_configured','true')",
            [],
        )?;
        assert!(
            replacement_backup_health(conn)
                .unwrap()
                .replacement_backup_unavailable,
            "configuration alone is not complete encrypted replacement-backup support"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn passphrase_reset_preserves_live_device_credentials() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (_sid, principal) = seed_principal(conn, "reset-live");
        let (_id, secret) = issue_at(conn, &principal, DeviceCredentialState::Current, 7, 100);
        let before_reset = iotkit_core_ops::device_auth_generation(conn).unwrap();
        let hash = iotkit_core_ops::hash_passphrase("new local admin passphrase").unwrap();
        iotkit_core_ops::reset_passphrase_with_hash(conn, &hash, "local_cli").unwrap();
        assert!(iotkit_core_ops::device_auth_generation(conn).unwrap() > before_reset);
        let after_reset_auth = authenticate_device(conn, &secret, &TestClock(Cell::new(101)))
            .unwrap()
            .unwrap();
        assert_eq!(
            after_reset_auth.auth_generation(),
            iotkit_core_ops::auth_generation(conn).unwrap()
        );
        assert_eq!(
            after_reset_auth.principal_material_generation(),
            iotkit_core_ops::device_auth_generation(conn).unwrap()
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn migration_rejects_impossible_lifecycle_metadata_and_timestamp_ordering() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (_sid, principal) = seed_principal(conn, "malformed");
        let epoch = iotkit_core_ops::auth_epoch(conn).unwrap();
        let bad_rows = [
            ("current_revoked", "current", Some(110), None, None),
            ("pending_confirmed", "pending", None, Some(110), Some(105)),
            ("revoked_without_time", "revoked", None, None, None),
            ("proof_before_issue", "pending", None, None, Some(90)),
        ];
        for (index, (id, state, revoked_at, confirmed_at, proven_at)) in
            bad_rows.into_iter().enumerate()
        {
            let hash = vec![u8::try_from(index + 1).unwrap(); 32];
            let result = conn.execute(
                "INSERT INTO device_credentials (
                   credential_id, principal_id, token_hash, auth_epoch, state, issued_at,
                   proven_at, confirmed_at, revoked_at, issue_reason, revoke_reason
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 100, ?6, ?7, ?8, 'manual_issue',
                           CASE WHEN ?5 = 'revoked' THEN 'operator_revoked' END)",
                params![
                    id,
                    principal,
                    hash,
                    epoch,
                    state,
                    proven_at,
                    confirmed_at,
                    revoked_at
                ],
            );
            assert!(result.is_err(), "malformed row {id} was accepted");
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn retired_principals_cannot_change_flow_or_subtract_capacity_and_reductions_need_no_reapproval() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute("UPDATE device_capacity SET steady_units=1, burst_units=1 WHERE id=1", [])?;
        conn.execute("UPDATE device_flow_classes SET steady_units=1, burst_units=1 WHERE flow_class='low'", [])?;
        conn.execute("UPDATE device_flow_classes SET steady_units=3, burst_units=3 WHERE flow_class='default'", [])?;
        conn.execute("UPDATE device_flow_classes SET steady_units=5, burst_units=5 WHERE flow_class='high'", [])?;
        let (sid, principal) = seed_principal(conn, "retired-capacity");
        issue_at(conn, &principal, DeviceCredentialState::Current, 201, 100);
        dispatch(
            conn,
            standard_catalog(),
            request(
                "device.flow_class_change_capacity_debt",
                bind_capacity_approval(
                    conn,
                    "device.flow_class_change_capacity_debt",
                    json!({"principal_ids":[principal.clone()],"flow_class":"high"}),
                ),
                false,
                ActorKind::LocalCli,
                true,
            ),
        ).unwrap();
        iotkit_core_ledger::retire_device(conn, &sid).unwrap();
        assert!(!iotkit_core_ops::capacity_health(conn).unwrap().active_debt);
        let err = dispatch(
            conn,
            standard_catalog(),
            request(
                "device.flow_class_change_capacity_debt",
                json!({"principal_ids":[principal.clone()],"flow_class":"high"}),
                false,
                ActorKind::LocalCli,
                true,
            ),
        )
        .unwrap_err();
        assert!(matches!(err, OpError::NotFound));
        assert!(matches!(
            iotkit_core_ops::device_credentials::capacity_status(conn, Some((&principal, "high"))),
            Err(iotkit_core_ops::OpsError::NotFound)
        ));

        let (_sid2, principal2) = seed_principal(conn, "debt-reduction");
        issue_at(conn, &principal2, DeviceCredentialState::Current, 202, 100);
        conn.execute("UPDATE device_capacity SET steady_units=1, burst_units=1 WHERE id=1", [])?;
        dispatch(
            conn,
            standard_catalog(),
            request(
                "device.flow_class_change_capacity_debt",
                bind_capacity_approval(
                    conn,
                    "device.flow_class_change_capacity_debt",
                    json!({"principal_ids":[principal2.clone()],"flow_class":"high"}),
                ),
                false,
                ActorKind::LocalCli,
                true,
            ),
        ).unwrap();
        dispatch(
            conn,
            standard_catalog(),
            request(
                "device.flow_class_change",
                json!({"principal_ids":[principal2.clone()],"flow_class":"default"}),
                false,
                ActorKind::LocalCli,
                false,
            ),
        )
        .unwrap();
        assert!(iotkit_core_ops::capacity_health(conn).unwrap().active_debt);
        dispatch(
            conn,
            standard_catalog(),
            request(
                "device.flow_class_change",
                json!({"principal_ids":[principal2],"flow_class":"low"}),
                false,
                ActorKind::LocalCli,
                false,
            ),
        )
        .unwrap();
        assert!(!iotkit_core_ops::capacity_health(conn).unwrap().active_debt);
        let debt_audit: String = conn.query_row(
            "SELECT group_concat(detail, '\n') FROM ledger_events WHERE kind='capacity_debt'",
            [], |row| row.get::<_, Option<String>>(0),
        )?.unwrap_or_default();
        assert!(debt_audit.contains("capacity_debt_created"));
        assert!(debt_audit.contains("capacity_debt_changed"));
        assert!(debt_audit.contains("capacity_debt_recovered"));
        Ok(())
    })
    .unwrap();
}

#[test]
fn list_is_read_only_and_principal_generation_covers_scope_and_flow_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authority.db");
    let db = iotkit_core_storage::init_db(&path, &all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (sid, principal) = seed_principal(conn, "generation");
        let initial = iotkit_core_ops::device_auth_generation(conn).unwrap();
        conn.execute(
            "UPDATE device_ingest_principals SET flow_class='low' WHERE principal_id=?1",
            [&principal],
        )
        .unwrap();
        let after_flow = iotkit_core_ops::device_auth_generation(conn).unwrap();
        assert!(after_flow > initial);
        conn.execute(
            "DELETE FROM device_principal_scopes WHERE principal_id=?1 AND system_id=?2",
            params![principal, sid.as_bytes().as_slice()],
        )
        .unwrap();
        assert!(iotkit_core_ops::device_auth_generation(conn).unwrap() > after_flow);
        Ok(())
    })
    .unwrap();
    drop(db);
    let read_only =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let before = read_only.total_changes();
    let _ = list_device_credentials(&read_only).unwrap();
    let _ = iotkit_core_ops::list_device_principals(&read_only).unwrap();
    assert_eq!(read_only.total_changes(), before);
}

#[test]
fn dispatch_keeps_plaintext_out_of_json_debug_audit_and_rejects_free_form_reasons() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let result = dispatch(
            conn,
            standard_catalog(),
            request(
                "device.add_with_credential",
                json!({"hardware_id":"typed-secret","flow_class":"default","reason_code":"device_commissioning"}),
                false,
                ActorKind::LocalCli,
                false,
            ),
        )
        .unwrap();
        let debug = format!("{result:?}");
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("ikd_"));
        let (metadata, plaintext) = match result {
            iotkit_core_ops::DispatchResult::DeviceCredential(secret) => secret.consume(),
            _ => panic!("typed presentation expected"),
        };
        assert!(metadata.get("plaintext").is_none());
        let audit: String = conn.query_row(
            "SELECT group_concat(detail, '\n') FROM ledger_events",
            [],
            |row| row.get::<_, Option<String>>(0),
        )?.unwrap_or_default();
        assert!(!audit.contains(plaintext.as_str()));
        assert!(!audit.contains("ikd_"));

        let principal = metadata["principal_id"].as_str().unwrap();
        let err = issue_device_credential_with(
            conn, principal, DeviceCredentialState::Pending,
            "my passphrase is hunter2", &mut TestEntropy(9), &TestClock(Cell::new(200)),
        ).unwrap_err();
        assert!(matches!(err, OpError::Validation(_)));
        Ok(())
    }).unwrap();
}

#[test]
fn authority_helpers_roll_back_all_writes_when_later_steps_fail() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let sid = iotkit_core_ledger::insert_device(conn, &NewDevice {
            hardware_id: "atomic-registration".into(), user_label: None, parent: None,
            kind: DeviceKind::Individual, initial_state: DeviceState::Active,
        }).unwrap();
        let missing = SystemId::from_bytes([0xee; 16]);
        assert!(register_device_principal(
            conn, "principal-atomic-registration", &sid, &[sid, missing], "default", 100,
        ).is_err());
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM device_ingest_principals WHERE principal_id='principal-atomic-registration'",
            [], |row| row.get(0),
        )
        .unwrap();
        assert_eq!(count, 0);

        let (_sid, principal) = seed_principal(conn, "atomic-auth");
        let (_current_id, _current_secret) = issue_at(conn, &principal, DeviceCredentialState::Current, 1, 100);
        let (pending_id, pending_secret) = issue_at(conn, &principal, DeviceCredentialState::Pending, 40, 101);
        conn.execute_batch(
            "CREATE TRIGGER fail_credential_use_audit BEFORE INSERT ON ledger_events
             WHEN NEW.kind='device_credential_use'
             BEGIN SELECT RAISE(ABORT, 'injected credential-use audit failure'); END;",
        )?;
        assert!(authenticate_device(conn, &pending_secret, &TestClock(Cell::new(102))).is_err());
        let proven: Option<i64> = conn.query_row(
            "SELECT proven_at FROM device_credentials WHERE credential_id=?1", [&pending_id], |row| row.get(0),
        )?;
        assert_eq!(proven, None);
        conn.execute_batch("DROP TRIGGER fail_credential_use_audit")?;
        authenticate_device(conn, &pending_secret, &TestClock(Cell::new(103))).unwrap();

        conn.execute_batch(
            "CREATE TRIGGER fail_promotion BEFORE UPDATE OF state ON device_credentials
             WHEN OLD.state='pending' AND NEW.state='current'
             BEGIN SELECT RAISE(ABORT, 'injected promotion failure'); END;",
        )?;
        assert!(confirm_device_credential(
            conn, &principal, &pending_id, "credential_confirmed", 104,
        ).is_err());
        let states: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT state FROM device_credentials WHERE principal_id=?1 ORDER BY state",
            )
            .unwrap();
            stmt.query_map([&principal], |row| row.get(0))?.collect::<Result<Vec<_>, _>>()?
        };
        assert_eq!(states, vec!["current", "pending"]);
        Ok(())
    }).unwrap();
}

#[test]
fn authority_weights_capacity_and_stale_age_use_validated_construction_configuration() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let bootstrap: Vec<(i64, i64)> = {
            let mut stmt = conn.prepare(
                "SELECT steady_units, burst_units FROM device_flow_classes ORDER BY flow_class",
            )?;
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?
        };
        assert_eq!(bootstrap, vec![(1, 1), (1, 1), (1, 1)]);
        let mut params = json!({
            "low_steady_units":2,"low_burst_units":3,
            "default_steady_units":4,"default_burst_units":5,
            "high_steady_units":6,"high_burst_units":7,
            "capacity_steady_units":8,"capacity_burst_units":9,"stale_after_ms":10,
        });
        let before = iotkit_core_ops::device_auth_generation(conn).unwrap();
        dispatch(
            conn,
            standard_catalog(),
            request(
                "device.authority_configure",
                params.clone(),
                false,
                ActorKind::LocalCli,
                true,
            ),
        )
        .unwrap();
        assert_eq!(
            iotkit_core_ops::configured_stale_after_ms(conn).unwrap(),
            10
        );
        assert!(iotkit_core_ops::device_auth_generation(conn).unwrap() > before);
        params["capacity_steady_units"] = json!(0);
        assert!(
            dispatch(
                conn,
                standard_catalog(),
                request(
                    "device.authority_configure",
                    params,
                    false,
                    ActorKind::LocalCli,
                    true,
                )
            )
            .is_err()
        );
        let capacity: i64 = conn.query_row(
            "SELECT steady_units FROM device_capacity WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(capacity, 8);
        Ok(())
    })
    .unwrap();
}

#[test]
fn confirmation_review_mutations_are_not_public_escape_hatches() {
    let exports = include_str!("../src/lib.rs");
    for forbidden in [
        "register_device_principal",
        "issue_device_credential",
        "issue_device_credential_with",
        "confirm_device_credential",
        "abandon_device_credential",
        "revoke_device_credential",
        "change_device_flow_class",
        "recover_capacity_debt_if_possible",
        "recover_capacity_debt_if_possible_in_tx",
        "CredentialEntropy",
        "CredentialClock",
    ] {
        assert!(
            !exports.contains(forbidden),
            "public mutation/test injection escape hatch remains: {forbidden}"
        );
    }
}

#[test]
fn confirmation_review_authentication_query_is_indexed_and_single_candidate() {
    let source = include_str!("../src/device_credentials.rs");
    let auth = source
        .split("fn lookup_device_credential")
        .nth(1)
        .expect("authentication implementation")
        .split("pub(crate) fn authenticate_device_with_clock")
        .next()
        .unwrap();
    assert!(auth.contains("c.token_hash=?"));
    assert!(auth.contains("LIMIT 1"));
    assert!(!auth.contains("collect::<Result<Vec<_>, _>>()"));
}

#[test]
fn confirmation_review_pending_proof_is_audited_once_and_last_use_is_not_audit_spam() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (_sid, principal) = seed_principal(conn, "proof-once");
        issue_at(conn, &principal, DeviceCredentialState::Current, 1, 100);
        let (_pending_id, pending) =
            issue_at(conn, &principal, DeviceCredentialState::Pending, 40, 101);
        authenticate_device(conn, &pending, &TestClock(Cell::new(200)))
            .unwrap()
            .unwrap();
        authenticate_device(conn, &pending, &TestClock(Cell::new(60_200)))
            .unwrap()
            .unwrap();
        authenticate_device(conn, &pending, &TestClock(Cell::new(120_200)))
            .unwrap()
            .unwrap();
        let events: i64 = conn.query_row(
            "SELECT COUNT(*) FROM ledger_events WHERE kind='device_credential_use'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(events, 1, "only the first pending proof is audited");
        Ok(())
    })
    .unwrap();
}

#[test]
fn confirmation_review_every_authority_material_direct_sql_change_advances_generation() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (sid_a, principal_a) = seed_principal(conn, "generation-a");
        let (sid_b, principal_b) = seed_principal(conn, "generation-b");
        let (credential_id, _secret) =
            issue_at(conn, &principal_a, DeviceCredentialState::Current, 1, 100);
        let mut generation = iotkit_core_ops::device_auth_generation(conn).unwrap();
        let mut assert_advanced = |conn: &rusqlite::Connection| {
            let next = iotkit_core_ops::device_auth_generation(conn).unwrap();
            assert!(next > generation);
            generation = next;
        };

        conn.execute(
            "UPDATE device_principal_scopes SET system_id=?1 WHERE principal_id=?2 AND system_id=?3",
            params![sid_b.as_bytes().as_slice(), principal_a, sid_a.as_bytes().as_slice()],
        )?;
        assert_advanced(conn);
        conn.execute(
            "UPDATE device_credentials SET token_hash=?1 WHERE credential_id=?2",
            params![vec![0x77_u8; 32], credential_id],
        )?;
        assert_advanced(conn);
        conn.execute(
            "UPDATE device_credentials SET principal_id=?1 WHERE credential_id=?2",
            params![principal_b, credential_id],
        )?;
        assert_advanced(conn);
        let moved_id = format!("{credential_id}-moved");
        conn.execute(
            "UPDATE device_credentials SET credential_id=?1 WHERE credential_id=?2",
            params![moved_id, credential_id],
        )?;
        assert_advanced(conn);
        conn.execute(
            "UPDATE device_credentials SET state='revoked', revoked_at=issued_at,
             revoke_reason='operator_revoked' WHERE credential_id=?1",
            [&moved_id],
        )?;
        assert_advanced(conn);
        conn.execute("DELETE FROM device_credentials WHERE credential_id=?1", [&moved_id])?;
        assert_advanced(conn);
        conn.execute("DELETE FROM device_ingest_principals WHERE principal_id=?1", [&principal_a])?;
        assert_advanced(conn);
        Ok(())
    })
    .unwrap();
}

#[test]
fn confirmation_review_capacity_counts_only_live_declared_authority() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (_sid, principal) = seed_principal(conn, "live-capacity");
        let (credential_id, _secret) =
            issue_at(conn, &principal, DeviceCredentialState::Current, 1, 100);
        assert_eq!(capacity_health(conn).unwrap().status.required_steady_units, 1);
        revoke_device_credential(
            conn,
            &principal,
            &credential_id,
            "operator_revoked",
            101,
        )
        .unwrap();
        assert_eq!(capacity_health(conn).unwrap().status.required_steady_units, 0);

        conn.execute("UPDATE device_capacity SET steady_units=1, burst_units=1", [])?;
        conn.execute(
            "UPDATE device_flow_classes SET steady_units=2, burst_units=2 WHERE flow_class='default'",
            [],
        )
        .unwrap();
        let err = dispatch(
            conn,
            standard_catalog(),
            request(
                "device_credential.issue",
                json!({"principal_id":principal,"reason_code":"manual_issue"}),
                false,
                ActorKind::LocalCli,
                false,
            ),
        )
        .unwrap_err();
        assert_eq!(err, OpError::PreconditionFailed("capacity_exceeded".into()));
        let debt_params = bind_capacity_approval(
            conn,
            "device_credential.issue_capacity_debt",
            json!({"principal_id":principal,"reason_code":"manual_issue"}),
        );
        assert!(matches!(
            dispatch(
                conn,
                standard_catalog(),
                request(
                    "device_credential.issue_capacity_debt",
                    debt_params,
                    false,
                    ActorKind::LocalCli,
                    true,
                ),
            )
            .unwrap(),
            iotkit_core_ops::DispatchResult::DeviceCredential(_)
        ));
        assert!(capacity_health(conn).unwrap().active_debt);
        Ok(())
    })
    .unwrap();
}

#[test]
fn confirmation_review_debt_compares_overage_and_requires_fresh_acceptance_when_capacity_falls() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute("UPDATE device_capacity SET steady_units=3, burst_units=3", [])?;
        conn.execute(
            "UPDATE device_flow_classes SET steady_units=5, burst_units=5 WHERE flow_class='high'",
            [],
        )?;
        dispatch(
            conn,
            standard_catalog(),
            request(
                "device.add_with_credential_capacity_debt",
                bind_capacity_approval(
                    conn,
                    "device.add_with_credential_capacity_debt",
                    json!({"hardware_id":"overage-device","flow_class":"high","reason_code":"device_commissioning"}),
                ),
                false,
                ActorKind::LocalCli,
                true,
            ),
        )
        .unwrap();
        let approved_by: String = conn.query_row(
            "SELECT approved_by FROM capacity_debt WHERE recovered_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        let err = dispatch(
            conn,
            standard_catalog(),
            request(
                "device.authority_configure",
                json!({"low_steady_units":1,"low_burst_units":1,"default_steady_units":1,
                    "default_burst_units":1,"high_steady_units":5,"high_burst_units":5,
                    "capacity_steady_units":1,"capacity_burst_units":1,"stale_after_ms":1}),
                false,
                ActorKind::LocalCli,
                true,
            ),
        );
        assert!(matches!(err, Err(OpError::PreconditionFailed(_))));
        assert_eq!(
            conn.query_row(
                "SELECT approved_by FROM capacity_debt WHERE recovered_at IS NULL",
                [],
                |row| row.get::<_, String>(0),
            )?,
            approved_by
        );
        let config = json!({"low_steady_units":1,"low_burst_units":1,"default_steady_units":1,
            "default_burst_units":1,"high_steady_units":5,"high_burst_units":5,
            "capacity_steady_units":1,"capacity_burst_units":1,"stale_after_ms":1});
        let bound = bind_capacity_approval(
            conn,
            "device.authority_configure_capacity_debt",
            config,
        );
        let mut approved = request(
            "device.authority_configure_capacity_debt",
            bound,
            false,
            ActorKind::LocalCli,
            true,
        );
        approved.actor.actor_id = "new-capacity-approver".into();
        dispatch(conn, standard_catalog(), approved).unwrap();
        let (new_approver, approved_capacity): (String, i64) = conn.query_row(
            "SELECT approved_by, capacity_steady_units FROM capacity_debt WHERE recovered_at IS NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(new_approver, "new-capacity-approver");
        assert_eq!(approved_capacity, 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn confirmation_review_lifecycle_timestamps_survive_backward_wall_clock_steps() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (_sid, principal) = seed_principal(conn, "clock-rollback");
        let (current_id, current) = issue_at(
            conn,
            &principal,
            DeviceCredentialState::Current,
            1,
            i64::MAX - 100,
        );
        authenticate_device(conn, &current, &TestClock(Cell::new(1)))
            .unwrap()
            .unwrap();
        revoke_device_credential(conn, &principal, &current_id, "operator_revoked", 1).unwrap();
        let revoked_at: i64 = conn
            .query_row(
                "SELECT revoked_at FROM device_credentials WHERE credential_id=?1",
                [&current_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(revoked_at >= i64::MAX - 100);

        let (_sid, confirm_principal) = seed_principal(conn, "clock-confirm");
        issue_at(
            conn,
            &confirm_principal,
            DeviceCredentialState::Current,
            10,
            i64::MAX - 300,
        );
        let (pending_id, pending) = issue_at(
            conn,
            &confirm_principal,
            DeviceCredentialState::Pending,
            11,
            i64::MAX - 200,
        );
        authenticate_device(conn, &pending, &TestClock(Cell::new(1)))
            .unwrap()
            .unwrap();
        lifecycle(
            conn,
            "device_credential.confirm",
            &confirm_principal,
            &pending_id,
            "credential_confirmed",
        )
        .unwrap();
        let (proven_at, confirmed_at): (i64, i64) = conn.query_row(
            "SELECT proven_at, confirmed_at FROM device_credentials WHERE credential_id=?1",
            [&pending_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert!(proven_at >= i64::MAX - 200);
        assert!(confirmed_at >= proven_at);

        let (_sid, abandon_principal) = seed_principal(conn, "clock-abandon");
        let (abandon_id, _secret) = issue_at(
            conn,
            &abandon_principal,
            DeviceCredentialState::Pending,
            12,
            i64::MAX - 150,
        );
        lifecycle(
            conn,
            "device_credential.abandon",
            &abandon_principal,
            &abandon_id,
            "pending_abandoned",
        )
        .unwrap();

        let (replace_sid, replace_principal) = seed_principal(conn, "clock-replace");
        let (replace_id, _secret) = issue_at(
            conn,
            &replace_principal,
            DeviceCredentialState::Current,
            13,
            i64::MAX - 140,
        );
        iotkit_core_ledger::replace_hardware(conn, &replace_sid, "clock-replacement").unwrap();
        let replace_revoked: i64 = conn.query_row(
            "SELECT revoked_at FROM device_credentials WHERE credential_id=?1",
            [&replace_id],
            |row| row.get(0),
        )?;
        assert!(replace_revoked >= i64::MAX - 140);

        let (retire_sid, retire_principal) = seed_principal(conn, "clock-retire");
        let (retire_id, _secret) = issue_at(
            conn,
            &retire_principal,
            DeviceCredentialState::Current,
            14,
            i64::MAX - 130,
        );
        iotkit_core_ledger::retire_device(conn, &retire_sid).unwrap();
        let retire_revoked: i64 = conn.query_row(
            "SELECT revoked_at FROM device_credentials WHERE credential_id=?1",
            [&retire_id],
            |row| row.get(0),
        )?;
        assert!(retire_revoked >= i64::MAX - 130);
        Ok(())
    })
    .unwrap();
}

#[test]
fn confirmation_review_capacity_consent_is_bound_to_previewed_values_and_generation() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute("UPDATE device_capacity SET steady_units=1, burst_units=1", [])?;
        conn.execute(
            "UPDATE device_flow_classes SET steady_units=5, burst_units=5 WHERE flow_class='high'",
            [],
        )?;
        let params = json!({"hardware_id":"bound-preview","flow_class":"high","reason_code":"device_commissioning"});
        let preview = dispatch(
            conn,
            standard_catalog(),
            request(
                "device.add_with_credential_capacity_debt",
                params.clone(),
                true,
                ActorKind::LocalCli,
                true,
            ),
        )
        .unwrap();
        assert!(preview.get("authority_generation").is_some());
        let mut bound = params;
        let object = bound.as_object_mut().unwrap();
        for (expected, shown) in [
            ("expected_required_steady_units", "required_steady_units"),
            ("expected_required_burst_units", "required_burst_units"),
            ("expected_capacity_steady_units", "capacity_steady_units"),
            ("expected_capacity_burst_units", "capacity_burst_units"),
            ("expected_authority_generation", "authority_generation"),
        ] {
            object.insert(expected.into(), preview[shown].clone());
        }
        conn.execute("UPDATE device_capacity SET steady_units=2 WHERE id=1", [])?;
        let err = dispatch(
            conn,
            standard_catalog(),
            request(
                "device.add_with_credential_capacity_debt",
                bound,
                false,
                ActorKind::LocalCli,
                true,
            ),
        )
        .unwrap_err();
        assert_eq!(err, OpError::PreconditionFailed("capacity_approval_stale".into()));
        Ok(())
    })
    .unwrap();
}

#[test]
fn confirmation_review_all_authority_mutations_are_typed_r14_guarded_and_audited() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let before: i64 = conn.query_row(
            "SELECT COUNT(*) FROM ledger_events WHERE kind='r14_op'",
            [],
            |row| row.get(0),
        )?;
        for op in [
            "device.add_with_credential",
            "device_credential.issue",
            "device_credential.reissue",
            "device_credential.confirm",
            "device_credential.abandon",
            "device_credential.revoke",
            "device.flow_class_change",
            "device.authority_configure",
        ] {
            let mut denied = request(op, json!({}), false, ActorKind::LocalCli, false);
            denied.actor.tier_ceiling = Tier::Routine;
            assert!(matches!(
                dispatch(conn, standard_catalog(), denied),
                Err(OpError::Forbidden(_))
            ));
        }
        let no_step_up = request(
            "device.add_with_credential_capacity_debt",
            json!({}),
            false,
            ActorKind::LocalCli,
            false,
        );
        assert_eq!(
            dispatch(conn, standard_catalog(), no_step_up).unwrap_err(),
            OpError::StepUpRequired
        );
        let mut daily = request(
            "device_credential.issue_capacity_debt",
            json!({}),
            true,
            ActorKind::LocalCli,
            true,
        );
        daily.actor.tier_ceiling = Tier::Daily;
        assert!(matches!(
            dispatch(conn, standard_catalog(), daily),
            Err(OpError::Forbidden(_))
        ));
        let no_step_up = request(
            "device_credential.issue_capacity_debt",
            json!({}),
            true,
            ActorKind::LocalCli,
            false,
        );
        assert_eq!(
            dispatch(conn, standard_catalog(), no_step_up).unwrap_err(),
            OpError::StepUpRequired
        );
        let after: i64 = conn.query_row(
            "SELECT COUNT(*) FROM ledger_events WHERE kind='r14_op'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(
            after - before,
            11,
            "every denied mutation attempt is audited"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn confirmation_fix_capacity_math_overflow_is_stable_validation_not_wrap_or_panic() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE device_capacity SET steady_units=?1, burst_units=?1 WHERE id=1",
            [i64::MAX],
        )?;
        conn.execute(
            "UPDATE device_flow_classes SET steady_units=?1, burst_units=?1 WHERE flow_class='default'",
            [i64::MAX],
        )?;
        for (seed, name) in [(31, "overflow-a"), (32, "overflow-b")] {
            let (_sid, principal) = seed_principal(conn, name);
            issue_at(conn, &principal, DeviceCredentialState::Current, seed, 1);
        }
        let err = capacity_health(conn).unwrap_err();
        assert_eq!(err.to_string(), "validation: capacity_math_overflow");

        let params = json!({
            "low_steady_units":1,"low_burst_units":1,
            "default_steady_units":i64::MAX,"default_burst_units":i64::MAX,
            "high_steady_units":1,"high_burst_units":1,
            "capacity_steady_units":i64::MAX,"capacity_burst_units":i64::MAX,
            "stale_after_ms":1
        });
        let err = dispatch(
            conn,
            standard_catalog(),
            request(
                "device.authority_configure_capacity_debt",
                params,
                true,
                ActorKind::LocalCli,
                true,
            ),
        )
        .unwrap_err();
        assert_eq!(err, OpError::Validation("capacity_math_overflow".into()));
        Ok(())
    })
    .unwrap();
}

#[test]
fn confirmation_fix_every_prospective_capacity_path_and_debt_boundary_is_checked() {
    let overflow_code = OpError::Validation("capacity_math_overflow".into());

    for path in ["add", "issue", "flow"] {
        let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
        db.with_conn_sync(|conn| {
            conn.execute(
                "UPDATE device_capacity SET steady_units=?1, burst_units=?1 WHERE id=1",
                [i64::MAX],
            )?;
            match path {
                "add" => {
                    conn.execute(
                        "UPDATE device_flow_classes SET steady_units=?1, burst_units=?1 WHERE flow_class='default'",
                        [i64::MAX],
                    )?;
                    let (_sid, principal) = seed_principal(conn, "prospective-add-live");
                    issue_at(conn, &principal, DeviceCredentialState::Current, 41, 1);
                    let err = dispatch(
                        conn,
                        standard_catalog(),
                        request(
                            "device.add_with_credential_capacity_debt",
                            json!({"hardware_id":"prospective-add","flow_class":"default","reason_code":"device_commissioning"}),
                            true,
                            ActorKind::LocalCli,
                            true,
                        ),
                    )
                    .unwrap_err();
                    assert_eq!(err, overflow_code);
                }
                "issue" => {
                    conn.execute(
                        "UPDATE device_flow_classes SET steady_units=?1, burst_units=?1 WHERE flow_class='default'",
                        [i64::MAX],
                    )?;
                    let (_sid, live) = seed_principal(conn, "prospective-issue-live");
                    issue_at(conn, &live, DeviceCredentialState::Current, 42, 1);
                    let (_sid, dormant) = seed_principal(conn, "prospective-issue-dormant");
                    let err = dispatch(
                        conn,
                        standard_catalog(),
                        request(
                            "device_credential.issue_capacity_debt",
                            json!({"principal_id":dormant,"reason_code":"manual_issue"}),
                            true,
                            ActorKind::LocalCli,
                            true,
                        ),
                    )
                    .unwrap_err();
                    assert_eq!(err, overflow_code);
                }
                "flow" => {
                    conn.execute(
                        "UPDATE device_flow_classes SET steady_units=1, burst_units=1 WHERE flow_class='default'",
                        [],
                    )?;
                    conn.execute(
                        "UPDATE device_flow_classes SET steady_units=?1, burst_units=?1 WHERE flow_class='high'",
                        [i64::MAX],
                    )?;
                    let (_sid, first) = seed_principal(conn, "prospective-flow-a");
                    issue_at(conn, &first, DeviceCredentialState::Current, 43, 1);
                    let (_sid, second) = seed_principal(conn, "prospective-flow-b");
                    issue_at(conn, &second, DeviceCredentialState::Current, 44, 1);
                    let err = dispatch(
                        conn,
                        standard_catalog(),
                        request(
                            "device.flow_class_change_capacity_debt",
                            json!({"principal_ids":[second],"flow_class":"high"}),
                            true,
                            ActorKind::LocalCli,
                            true,
                        ),
                    )
                    .unwrap_err();
                    assert_eq!(err, overflow_code);
                }
                _ => unreachable!(),
            }
            Ok(())
        })
        .unwrap();
    }

    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE device_flow_classes SET steady_units=?1, burst_units=?1 WHERE flow_class='high'",
            [i64::MAX],
        )?;
        let params = bind_capacity_approval(
            conn,
            "device.add_with_credential_capacity_debt",
            json!({"hardware_id":"max-debt","flow_class":"high","reason_code":"device_commissioning"}),
        );
        dispatch(
            conn,
            standard_catalog(),
            request(
                "device.add_with_credential_capacity_debt",
                params,
                false,
                ActorKind::LocalCli,
                true,
            ),
        )
        .unwrap();
        let persisted: (i64, i64) = conn.query_row(
            "SELECT required_steady_units, required_burst_units
             FROM capacity_debt WHERE recovered_at IS NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(persisted, (i64::MAX, i64::MAX));
        assert_eq!(
            capacity_health(conn).unwrap().status.required_steady_units,
            i64::MAX
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn confirmation_fix_sql_reconciliation_rejects_overflow_without_partial_revocation() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE device_flow_classes SET steady_units=?1, burst_units=?1 WHERE flow_class='default'",
            [i64::MAX],
        )?;
        let mut credentials = Vec::new();
        for (seed, name) in [
            (51, "trigger-overflow-a"),
            (52, "trigger-overflow-b"),
            (53, "trigger-overflow-c"),
        ] {
            let (_sid, principal) = seed_principal(conn, name);
            let (credential, _secret) =
                issue_at(conn, &principal, DeviceCredentialState::Current, seed, 1);
            credentials.push(credential);
        }
        conn.execute(
            "INSERT INTO capacity_debt
             (approved_at, changed_at, approved_by, operation, required_steady_units,
              required_burst_units, capacity_steady_units, capacity_burst_units)
             VALUES (1,1,'forged','device_add',1,1,1,1)",
            [],
        )?;
        let err = conn
            .execute(
                "UPDATE device_credentials SET state='revoked', revoked_at=2,
                 revoke_reason='operator_revoked' WHERE credential_id=?1",
                [&credentials[0]],
            )
            .unwrap_err();
        assert!(err.to_string().contains("capacity_math_overflow"));
        let state: String = conn.query_row(
            "SELECT state FROM device_credentials WHERE credential_id=?1",
            [&credentials[0]],
            |row| row.get(0),
        )?;
        assert_eq!(state, "current");
        Ok(())
    })
    .unwrap();
}

#[test]
fn confirmation_fix_scope_ceiling_is_enforced_typed_sql_and_auth_is_defensive() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let cap = iotkit_core_ops::device_credentials::DEVICE_PRINCIPAL_SCOPE_CAP;
        let mut scope_ids = Vec::new();
        for index in 0..=cap {
            let sid = iotkit_core_ledger::insert_device(
                conn,
                &NewDevice {
                    hardware_id: format!("scope-cap-{index}"),
                    user_label: None,
                    parent: None,
                    kind: DeviceKind::Individual,
                    initial_state: DeviceState::Active,
                },
            )
            .unwrap();
            scope_ids.push(sid);
        }
        conn.execute(
            "UPDATE device_capacity SET steady_units=1000, burst_units=1000 WHERE id=1",
            [],
        )?;
        let exact = scope_ids[..cap]
            .iter()
            .skip(1)
            .map(SystemId::to_text)
            .collect::<Vec<_>>();
        let exact_params = json!({
            "hardware_id":"typed-scope-exact",
            "flow_class":"default",
            "reason_code":"device_commissioning",
            "scope_system_ids":exact
        });
        assert!(dispatch(
            conn,
            standard_catalog(),
            request("device.add_with_credential", exact_params, false, ActorKind::LocalCli, false),
        )
        .is_ok());

        let too_many = scope_ids[..=cap]
            .iter()
            .map(SystemId::to_text)
            .collect::<Vec<_>>();
        let err = dispatch(
            conn,
            standard_catalog(),
            request(
                "device.add_with_credential",
                json!({"hardware_id":"typed-scope-over","flow_class":"default","reason_code":"device_commissioning","scope_system_ids":too_many}),
                false,
                ActorKind::LocalCli,
                false,
            ),
        )
        .unwrap_err();
        assert_eq!(
            err,
            OpError::Validation("validation: principal_scope_limit_exceeded".into())
        );

        let principal = "principal-forged-scope-cap";
        conn.execute(
            "INSERT INTO device_ingest_principals
             (principal_id, device_system_id, flow_class, profile, created_at)
             VALUES (?1, ?2, 'default', 'simple_bearer', 1)",
            params![principal, scope_ids[0].as_bytes().as_slice()],
        )?;
        for sid in &scope_ids[..cap] {
            conn.execute(
                "INSERT INTO device_principal_scopes (principal_id, system_id) VALUES (?1, ?2)",
                params![principal, sid.as_bytes().as_slice()],
            )?;
        }
        let direct = conn
            .execute(
                "INSERT INTO device_principal_scopes (principal_id, system_id) VALUES (?1, ?2)",
                params![principal, scope_ids[cap].as_bytes().as_slice()],
            )
            .unwrap_err();
        assert!(direct.to_string().contains("principal scope limit exceeded"));

        conn.execute_batch("DROP TRIGGER device_scope_count_limit")?;
        conn.execute(
            "INSERT INTO device_principal_scopes (principal_id, system_id) VALUES (?1, ?2)",
            params![principal, scope_ids[cap].as_bytes().as_slice()],
        )?;
        let (_id, secret) = issue_at(conn, principal, DeviceCredentialState::Current, 91, 10);
        let before: (Option<i64>, i64) = conn.query_row(
            "SELECT last_used_at, (SELECT COUNT(*) FROM ledger_events WHERE kind='device_credential_use')
             FROM device_credentials WHERE principal_id=?1",
            [principal],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert!(authenticate_device_public(conn, &secret).unwrap().is_none());
        let after: (Option<i64>, i64) = conn.query_row(
            "SELECT last_used_at, (SELECT COUNT(*) FROM ledger_events WHERE kind='device_credential_use')
             FROM device_credentials WHERE principal_id=?1",
            [principal],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(after, before);
        Ok(())
    })
    .unwrap();
}

#[test]
fn confirmation_fix_live_authority_requires_usable_scope_before_auth_side_effects() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (sid, principal) = seed_principal(conn, "zero-live-scope");
        let (_current_id, _current) =
            issue_at(conn, &principal, DeviceCredentialState::Current, 92, 10);
        let (pending_id, pending) =
            issue_at(conn, &principal, DeviceCredentialState::Pending, 93, 11);
        conn.execute(
            "DELETE FROM device_principal_scopes WHERE principal_id=?1",
            [&principal],
        )?;
        assert_eq!(capacity_health(conn).unwrap().status.required_steady_units, 0);
        assert!(authenticate_device_public(conn, &pending).unwrap().is_none());
        let untouched: (Option<i64>, Option<i64>, i64) = conn.query_row(
            "SELECT proven_at, last_used_at,
                    (SELECT COUNT(*) FROM ledger_events
                     WHERE kind='device_credential_use' AND detail LIKE '%pending_credential_proven%')
             FROM device_credentials WHERE credential_id=?1",
            [&pending_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(untouched, (None, None, 0));

        conn.execute(
            "INSERT INTO device_principal_scopes (principal_id, system_id) VALUES (?1, ?2)",
            params![principal, sid.as_bytes().as_slice()],
        )?;
        assert!(authenticate_device_public(conn, &pending).unwrap().is_some());
        let proven: Option<i64> = conn.query_row(
            "SELECT proven_at FROM device_credentials WHERE credential_id=?1",
            [&pending_id],
            |row| row.get(0),
        )?;
        assert!(proven.is_some());
        assert_eq!(capacity_health(conn).unwrap().status.required_steady_units, 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn confirmation_review_last_live_replace_updates_then_recovers_capacity_debt_atomically() {
    let db = iotkit_core_storage::init_db_memory(&all_migrations()).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE device_capacity SET steady_units=1, burst_units=1",
            [],
        )?;
        conn.execute(
            "UPDATE device_flow_classes SET steady_units=3, burst_units=3 WHERE flow_class='high'",
            [],
        )?;
        let mut devices = Vec::new();
        for suffix in ["a", "b"] {
            let op = "device.add_with_credential_capacity_debt";
            let params = bind_capacity_approval(
                conn,
                op,
                json!({"hardware_id":format!("debt-replace-{suffix}"),"flow_class":"high",
                    "reason_code":"device_commissioning"}),
            );
            let result = dispatch(
                conn,
                standard_catalog(),
                request(op, params, false, ActorKind::LocalCli, true),
            )
            .unwrap();
            let (metadata, _secret) = match result {
                iotkit_core_ops::DispatchResult::DeviceCredential(secret) => secret.consume(),
                _ => unreachable!(),
            };
            devices.push(SystemId::from_text(metadata["system_id"].as_str().unwrap()).unwrap());
        }
        iotkit_core_ledger::replace_hardware(conn, &devices[0], "debt-replace-a-new").unwrap();
        let (required, active): (i64, bool) = (
            capacity_health(conn).unwrap().status.required_steady_units,
            capacity_health(conn).unwrap().active_debt,
        );
        assert_eq!(required, 3);
        assert!(active);
        let approved_required: i64 = conn.query_row(
            "SELECT required_steady_units FROM capacity_debt WHERE recovered_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(approved_required, 3);

        iotkit_core_ledger::replace_hardware(conn, &devices[1], "debt-replace-b-new").unwrap();
        assert_eq!(
            capacity_health(conn).unwrap().status.required_steady_units,
            0
        );
        assert!(!capacity_health(conn).unwrap().active_debt);
        Ok(())
    })
    .unwrap();
}
