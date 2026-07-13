use iotkit_core_ops::{
    Actor, ActorKind, DispatchRequest, DispatchResult, OpDescriptor, OpError, Tier,
    dispatch_with_secret_dir, load_ingress_listener_config, reconcile_ingress_tls_custody,
    standard_catalog,
};
use iotkit_core_storage::Migration;
use rcgen::{CertificateParams, KeyPair};
use serde_json::{Value, json};

const _: () = assert!(iotkit_core_ops::INGRESS_READY);

fn dispatch(
    conn: &rusqlite::Connection,
    catalog: &[OpDescriptor],
    request: DispatchRequest,
) -> Result<DispatchResult, OpError> {
    let custody = tempfile::tempdir().unwrap();
    dispatch_with_secret_dir(conn, catalog, request, Some(custody.path()))
}

fn migrations() -> Vec<Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    all.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    all.sort_by_key(|migration| migration.version);
    all
}

fn request(op: &str, params: Value, step_up: bool) -> DispatchRequest {
    DispatchRequest {
        op: op.into(),
        params,
        dry_run: false,
        actor: Actor {
            actor_id: "local_cli".into(),
            actor_kind: ActorKind::LocalCli,
            tier_ceiling: Tier::Construction,
        },
        source: Some("test".into()),
        step_up_verified: step_up,
        clock_trust: None,
    }
}

fn human_request(actor_id: String, op: &str, params: Value, step_up: bool) -> DispatchRequest {
    let mut request = request(op, params, step_up);
    request.actor = Actor {
        actor_id,
        actor_kind: ActorKind::Human,
        tier_ceiling: Tier::Construction,
    };
    request
}

fn pair(name: &str) -> (String, String) {
    let key = KeyPair::generate().unwrap();
    let cert = CertificateParams::new(vec![name.into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    (cert.pem(), key.serialize_pem())
}

#[test]
fn default_is_durably_disabled_unbound_and_generation_zero() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let config = load_ingress_listener_config(conn).unwrap();
        assert!(!config.enabled);
        assert_eq!(config.desired.generation, 0);
        assert!(config.applied.is_none());
        Ok(())
    })
    .unwrap();
}

#[test]
fn enable_succeeds_only_after_bounded_ingest_schema_is_present() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (cert, key) = pair("ingress.test");
        dispatch(
            conn,
            standard_catalog(),
            request(
                "ingress.tls.rotate",
                json!({"cert_pem":cert,"key_pem":key}),
                true,
            ),
        )
        .unwrap();
        let bounded_columns: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('staged_readings') WHERE name IN ('principal_id','payload_bytes','pinned')",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(bounded_columns, 3);
        let maintenance_table: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='ingest_dedup_maintenance')",
            [],
            |row| row.get(0),
        )?;
        assert!(maintenance_table);
        for dry_run in [true, false] {
            let mut enable = request(
                "ingress.listener.configure",
                json!({
                    "enabled":true,"bind_addr":"192.168.4.2:8444","interface":"eth0",
                    "site_local_cidrs":["192.168.4.0/24"],"mode":"tls"
                }),
                true,
            );
            enable.dry_run = dry_run;
            dispatch(conn, standard_catalog(), enable).unwrap();
        }
        let after = load_ingress_listener_config(conn).unwrap();
        assert!(after.enabled);
        let audit: String = conn.query_row(
            "SELECT detail FROM ledger_events WHERE kind='r14_op' ORDER BY event_id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
        assert!(audit.contains(r#""result":"ok""#));
        assert!(!audit.contains("PRIVATE KEY"));
        Ok(())
    })
    .unwrap();
}

#[test]
fn construction_step_up_and_exposure_validation_are_enforced() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let human = iotkit_core_ops::issue_token(
            conn,
            &iotkit_core_ops::NewOperatorToken {
                name: "installer".into(),
                kind: iotkit_core_ops::TokenKind::Human,
                ceiling: Tier::Construction,
                is_session: false,
                expires_at: None,
            },
            "test",
            None,
            None,
        )
        .unwrap();
        let params = json!({"enabled":false,"bind_addr":"192.168.4.2:8444","interface":"eth0","site_local_cidrs":["192.168.4.0/24"],"mode":"private_plaintext"});
        assert_eq!(dispatch(conn, standard_catalog(), human_request(human.token_id.clone(), "ingress.listener.configure", params.clone(), false)), Err(OpError::StepUpRequired));
        dispatch(conn, standard_catalog(), human_request(human.token_id, "ingress.listener.configure", params, true)).unwrap();
        for bind in ["0.0.0.0:8444", "8.8.8.8:8444", "[::]:8444", "[::ffff:8.8.8.8]:8444"] {
            let err = dispatch(conn, standard_catalog(), request("ingress.listener.configure", json!({"enabled":false,"bind_addr":bind,"interface":"eth0","site_local_cidrs":["0.0.0.0/0"],"mode":"private_plaintext"}), true)).unwrap_err();
            assert!(matches!(err, OpError::Validation(_)));
        }
        let spanning = dispatch(conn, standard_catalog(), request("ingress.listener.configure", json!({"enabled":false,"bind_addr":"192.168.4.2:8444","interface":"eth0","site_local_cidrs":["192.168.0.0/15"],"mode":"private_plaintext"}), true)).unwrap_err();
        assert!(matches!(spanning, OpError::Validation(_)));
        Ok(())
    }).unwrap();
}

#[test]
fn tls_rotation_rejects_corrupt_and_mismatched_pairs_then_commits_and_audits() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (cert_a, key_a) = pair("a.test");
        let (_cert_b, key_b) = pair("b.test");
        for (cert, key) in [("bad".to_string(), "bad".to_string()), (cert_a.clone(), key_b)] {
            assert!(matches!(dispatch(conn, standard_catalog(), request("ingress.tls.rotate", json!({"cert_pem":cert,"key_pem":key}), true)), Err(OpError::Validation(_))));
        }
        let result = dispatch(conn, standard_catalog(), request("ingress.tls.rotate", json!({"cert_pem":cert_a,"key_pem":key_a}), true)).unwrap();
        assert_eq!(result["generation"], 1);
        let generation: i64 = conn.query_row("SELECT generation FROM ingress_tls_material", [], |row| row.get(0))?;
        assert_eq!(generation, 1);
        let schema: String = conn.query_row("SELECT sql FROM sqlite_schema WHERE name='ingress_tls_material'", [], |row| row.get(0))?;
        assert!(!schema.contains("key_pem"));
        let audit: String = conn.query_row("SELECT detail FROM ledger_events WHERE kind='r14_op' AND detail LIKE '%ingress.tls.rotate%' ORDER BY event_id DESC LIMIT 1", [], |row| row.get(0))?;
        assert!(audit.contains("[REDACTED]"));
        assert!(!audit.contains("PRIVATE KEY"));
        Ok(())
    }).unwrap();
}

#[test]
fn tls_rotation_audit_failure_rolls_back_approved_fingerprint_and_generation() {
    let dir = tempfile::tempdir().unwrap();
    let db = iotkit_core_storage::init_db(&dir.path().join("audit.db"), &migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (cert, key) = pair("first.test");
        dispatch_with_secret_dir(
            conn,
            standard_catalog(),
            request(
                "ingress.tls.rotate",
                json!({"cert_pem":cert,"key_pem":key}),
                true,
            ),
            Some(dir.path()),
        )
        .unwrap();
        let before: (i64, String) = conn.query_row(
            "SELECT generation,fingerprint FROM ingress_tls_material",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        conn.execute_batch(
            "CREATE TRIGGER fail_ingress_rotation_audit BEFORE INSERT ON ledger_events
             WHEN NEW.kind='r14_op' AND NEW.detail LIKE '%ingress.tls.rotate%'
             BEGIN SELECT RAISE(ABORT, 'injected audit failure'); END;",
        )?;
        let (cert, key) = pair("second.test");
        assert!(
            dispatch_with_secret_dir(
                conn,
                standard_catalog(),
                request(
                    "ingress.tls.rotate",
                    json!({"cert_pem":cert,"key_pem":key}),
                    true,
                ),
                Some(dir.path()),
            )
            .is_err()
        );
        let after: (i64, String) = conn.query_row(
            "SELECT generation,fingerprint FROM ingress_tls_material",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(after, before);
        assert!(dir.path().join("ingress-tls/generation-1").is_dir());
        assert!(!dir.path().join("ingress-tls/generation-2").exists());
        assert!(
            !dir.path()
                .join("ingress-tls/.generation-2.staging")
                .exists()
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn tls_rotation_commit_failure_removes_unsettled_material_and_keeps_last_safe() {
    let dir = tempfile::tempdir().unwrap();
    let db = iotkit_core_storage::init_db(&dir.path().join("commit.db"), &migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (cert, key) = pair("first.test");
        dispatch_with_secret_dir(
            conn,
            standard_catalog(),
            request(
                "ingress.tls.rotate",
                json!({"cert_pem":cert,"key_pem":key}),
                true,
            ),
            Some(dir.path()),
        )
        .unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE commit_probe_parent (id INTEGER PRIMARY KEY);
             CREATE TABLE commit_probe_child (
               id INTEGER REFERENCES commit_probe_parent(id) DEFERRABLE INITIALLY DEFERRED
             );
             CREATE TRIGGER fail_ingress_rotation_commit AFTER UPDATE ON ingress_tls_material
             BEGIN INSERT INTO commit_probe_child VALUES (1); END;",
        )?;

        let (cert, key) = pair("second.test");
        assert!(
            dispatch_with_secret_dir(
                conn,
                standard_catalog(),
                request(
                    "ingress.tls.rotate",
                    json!({"cert_pem":cert,"key_pem":key}),
                    true,
                ),
                Some(dir.path()),
            )
            .is_err()
        );
        let generation: i64 = conn.query_row(
            "SELECT generation FROM ingress_tls_material WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(generation, 1);
        assert!(dir.path().join("ingress-tls/generation-1").is_dir());
        assert!(!dir.path().join("ingress-tls/generation-2").exists());
        assert!(
            !dir.path()
                .join("ingress-tls/.generation-2.staging")
                .exists()
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn startup_reconciliation_promotes_only_settled_and_retains_referenced_last_safe_generation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("crash.db");
    let db = iotkit_core_storage::init_db(&db_path, &migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (cert, key) = pair("first.test");
        dispatch_with_secret_dir(
            conn,
            standard_catalog(),
            request(
                "ingress.tls.rotate",
                json!({"cert_pem":cert,"key_pem":key}),
                true,
            ),
            Some(dir.path()),
        )
        .unwrap();
        conn.execute(
            "UPDATE ingress_listener_config SET applied_generation=1,
             applied_bind_addr='192.168.1.2:8444',applied_interface='eth0',
             applied_site_local_cidrs='[\"192.168.1.0/24\"]',applied_mode='tls',
             applied_tls_generation=1,
             applied_tls_fingerprint=(SELECT fingerprint FROM ingress_tls_material WHERE id=1)
             WHERE id=1",
            [],
        )?;
        let (cert, key) = pair("second.test");
        dispatch_with_secret_dir(
            conn,
            standard_catalog(),
            request(
                "ingress.tls.rotate",
                json!({"cert_pem":cert,"key_pem":key}),
                true,
            ),
            Some(dir.path()),
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    let root = dir.path().join("ingress-tls");
    std::fs::rename(
        root.join("generation-2"),
        root.join(".generation-2.staging"),
    )
    .unwrap();
    std::fs::create_dir(root.join("generation-77")).unwrap();
    std::fs::create_dir(root.join(".generation-78.staging")).unwrap();
    drop(db);

    let reopened = iotkit_core_storage::init_db(&db_path, &migrations()).unwrap();
    reopened
        .with_conn_sync(|conn| {
            reconcile_ingress_tls_custody(conn, dir.path()).map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
        })
        .unwrap();
    assert!(root.join("generation-1").is_dir());
    assert!(root.join("generation-2").is_dir());
    assert!(!root.join(".generation-2.staging").exists());
    assert!(!root.join("generation-77").exists());
    assert!(!root.join(".generation-78.staging").exists());
}

#[test]
fn approved_tls_generation_survives_restart_with_exact_private_custody() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("ops.db");
    let db = iotkit_core_storage::init_db(&db_path, &migrations()).unwrap();
    let (cert, key) = pair("restart.test");
    db.with_conn_sync(|conn| {
        dispatch_with_secret_dir(
            conn,
            standard_catalog(),
            request(
                "ingress.tls.rotate",
                json!({"cert_pem":cert.clone(),"key_pem":key.clone()}),
                true,
            ),
            Some(dir.path()),
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    drop(db);

    let generation_dir = dir.path().join("ingress-tls/generation-1");
    assert_eq!(
        std::fs::read_to_string(generation_dir.join("cert.pem")).unwrap(),
        cert
    );
    assert_eq!(
        std::fs::read_to_string(generation_dir.join("key.pem")).unwrap(),
        key
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(generation_dir.join("key.pem"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let reopened = iotkit_core_storage::init_db(&db_path, &migrations()).unwrap();
    reopened
        .with_conn_sync(|conn| {
            let approved: (i64, String) = conn.query_row(
                "SELECT generation,fingerprint FROM ingress_tls_material WHERE id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            assert_eq!(approved.0, 1);
            assert_eq!(
                approved.1,
                iotkit_core_ops::fingerprint_of_pem(&cert).unwrap()
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn r14_chain_approval_fingerprints_the_applicable_leaf() {
    let dir = tempfile::tempdir().unwrap();
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    let (leaf, key) = pair("leaf.test");
    let (issuer, _) = pair("issuer.test");
    let chain = format!("{leaf}{issuer}");
    db.with_conn_sync(|conn| {
        dispatch_with_secret_dir(
            conn,
            standard_catalog(),
            request(
                "ingress.tls.rotate",
                json!({"cert_pem":chain,"key_pem":key}),
                true,
            ),
            Some(dir.path()),
        )
        .unwrap();
        let approved: String = conn.query_row(
            "SELECT fingerprint FROM ingress_tls_material WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(
            approved,
            iotkit_core_ops::fingerprint_of_pem(&leaf).unwrap()
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn disabled_apply_never_claims_uninstalled_tls_generation() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (cert, key) = pair("disabled.test");
        dispatch(
            conn,
            standard_catalog(),
            request(
                "ingress.tls.rotate",
                json!({"cert_pem":cert,"key_pem":key}),
                true,
            ),
        )
        .unwrap();
        iotkit_core_ops::mark_ingress_applied(conn, 1, None).unwrap();
        let applied = load_ingress_listener_config(conn).unwrap().applied.unwrap();
        assert_eq!(applied.tls_generation, None);
        assert_eq!(applied.tls_fingerprint, None);
        Ok(())
    })
    .unwrap();
}

#[test]
fn r14_rejects_undeclared_secret_aliases_and_redacts_nested_pem_values() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let (cert, key) = pair("adversarial.test");
        for (name, value) in [
            ("private_key", json!(key.clone())),
            ("backup_key", json!(key.clone())),
            ("nested", json!({"material": key.clone()})),
            ("array", json!(["safe", key.clone()])),
        ] {
            let mut params = serde_json::Map::from_iter([
                ("cert_pem".into(), json!(cert.clone())),
                ("key_pem".into(), json!(key.clone())),
            ]);
            params.insert(name.into(), value);
            let error = dispatch(
                conn,
                standard_catalog(),
                request("ingress.tls.rotate", Value::Object(params), true),
            )
            .unwrap_err();
            assert_eq!(
                error,
                OpError::Validation("undeclared operation parameter".into())
            );
        }
        let audits = conn
            .prepare("SELECT detail FROM ledger_events WHERE kind='r14_op'")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for audit in audits {
            assert!(!audit.contains("PRIVATE KEY"));
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn apply_failure_retains_last_safe_generation_and_later_exact_apply_converges() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    db.with_conn_sync(|conn| {
        let first = json!({"enabled":false,"bind_addr":"192.168.4.2:8444","interface":"eth0","site_local_cidrs":["192.168.4.0/24"],"mode":"private_plaintext"});
        dispatch(conn, standard_catalog(), request("ingress.listener.configure", first, true)).unwrap();
        iotkit_core_ops::mark_ingress_applied(conn, 1, None).unwrap();
        let second = json!({"enabled":false,"bind_addr":"10.2.3.4:8444","interface":"eth1","site_local_cidrs":["10.0.0.0/8"],"mode":"private_plaintext"});
        dispatch(conn, standard_catalog(), request("ingress.listener.configure", second, true)).unwrap();
        iotkit_core_ops::mark_ingress_apply_error(conn, 2, "bind_failed").unwrap();
        let failed = load_ingress_listener_config(conn).unwrap();
        assert_eq!(failed.desired.generation, 2);
        assert_eq!(failed.applied.as_ref().unwrap().generation, 1);
        assert_eq!(failed.applied.as_ref().unwrap().bind_addr, "192.168.4.2:8444");
        assert_eq!(failed.last_error.as_deref(), Some("bind_failed"));
        assert!(matches!(iotkit_core_ops::mark_ingress_applied(conn, 1, None), Err(iotkit_core_ops::OpsError::Conflict)));
        conn.execute_batch(
            "CREATE TRIGGER ignore_ingress_applied_update
             BEFORE UPDATE OF applied_generation ON ingress_listener_config
             WHEN NEW.applied_generation > OLD.applied_generation
             BEGIN SELECT RAISE(IGNORE); END;",
        )?;
        assert!(matches!(
            iotkit_core_ops::mark_ingress_applied(conn, 2, None),
            Err(iotkit_core_ops::OpsError::Conflict)
        ));
        conn.execute_batch("DROP TRIGGER ignore_ingress_applied_update")?;
        iotkit_core_ops::mark_ingress_applied(conn, 2, None).unwrap();
        let applied = load_ingress_listener_config(conn).unwrap();
        assert_eq!(applied.applied.as_ref().unwrap().generation, 2);
        assert_eq!(applied.applied.as_ref().unwrap().bind_addr, "10.2.3.4:8444");
        assert!(applied.last_error.is_none());
        Ok(())
    }).unwrap();
}
