use std::sync::{Arc, Mutex};
use std::time::Duration;

use iotkit_gateway::health::HealthState;
use iotkit_gateway::network_authority::{NetworkAuthorityError, require_network_authority};

fn migrations() -> Vec<iotkit_core_storage::Migration> {
    let mut all = iotkit_core_storage::MIGRATIONS.to_vec();
    all.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    all.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    all.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    all.sort_by_key(|migration| migration.version);
    all
}

fn own(db: &iotkit_core_storage::DbHandle) {
    db.with_conn_sync(|conn| {
        let hash = iotkit_core_ops::hash_passphrase("test-passphrase-long-enough").unwrap();
        iotkit_core_ops::reset_passphrase_with_hash(conn, &hash, "local_cli").unwrap();
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn listener_supervisor_exit_clears_health_without_stopping_collection() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    own(&db);
    let dir = tempfile::tempdir().unwrap();
    db.with_conn_sync(|conn| {
        iotkit_gateway::api::tls::ensure_tls_material(conn, dir.path())
            .map(|_| ())
            .map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
    })
    .unwrap();
    let health = Arc::new(Mutex::new(HealthState::new(90)));
    let task = iotkit_gateway::ingress::spawn_ingress_supervisor(
        db,
        dir.path().to_path_buf(),
        health.clone(),
        Duration::from_millis(1),
    );
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(health.lock().unwrap().ingress.status, "disabled");
    task.abort();
    let _ = task.await;
    let health = health.lock().unwrap();
    assert_eq!(health.ingress.status, "error");
    assert_eq!(
        health.ingress.last_error.as_deref(),
        Some("listener_task_exited")
    );
    assert!(
        health.collector_alive,
        "listener failure must not stop in-process collection"
    );
}

#[test]
fn common_gate_closes_unowned_recovery_fences_and_restored_generation_mismatch() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::Unowned)
        );
        Ok(())
    })
    .unwrap();
    own(&db);
    db.with_conn_sync(|conn| {
        iotkit_gateway::api::tls::ensure_tls_material(conn, dir.path())
            .map(|_| ())
            .map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
    })
    .unwrap();
    db.with_conn_sync(|conn| {
        conn.execute("DELETE FROM admin_credential", [])?;
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::LocalRecoveryRequired)
        );
        Ok(())
    })
    .unwrap();
    own(&db);
    std::fs::write(dir.path().join("restore-in-progress"), b"fence").unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::RestoreInProgress)
        );
        Ok(())
    })
    .unwrap();
    std::fs::remove_file(dir.path().join("restore-in-progress")).unwrap();
    std::fs::write(dir.path().join("reset-in-progress"), b"fence").unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::ResetInProgress)
        );
        Ok(())
    })
    .unwrap();
    std::fs::remove_file(dir.path().join("reset-in-progress")).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE ingress_listener_config SET enabled=1,desired_generation=1,
             applied_generation=1,bind_addr='192.168.1.2:8444',interface='eth0',
             site_local_cidrs='[\"192.168.1.0/24\"]',mode='private_plaintext',
             applied_bind_addr='192.168.1.2:8444',applied_interface='eth0',
             applied_site_local_cidrs='[\"192.168.1.0/24\"]',
             applied_mode='private_plaintext' WHERE id=1",
            [],
        )?;
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
        iotkit_core_ops::enter_restored_local_recovery(
            &tx,
            &iotkit_core_ops::new_auth_epoch().unwrap(),
        )
        .unwrap();
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    own(&db);
    db.with_conn_sync(|conn| {
        assert!(
            iotkit_gateway::network_authority::require_common_network_authority(conn, dir.path())
                .is_ok(),
            "local recovery must restore control-plane authority even while ingress awaits reapply"
        );
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::UnsafeIngressGeneration)
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn common_gate_rejects_partial_corrupt_and_mismatched_control_tls_for_both_listeners() {
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    own(&db);
    let dir = tempfile::tempdir().unwrap();
    db.with_conn_sync(|conn| {
        iotkit_gateway::api::tls::ensure_tls_material(conn, dir.path())
            .map(|_| ())
            .map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
    })
    .unwrap();
    let key = dir.path().join("tls/key.pem");
    let original = std::fs::read(&key).unwrap();
    std::fs::remove_file(&key).unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::TlsNotReady)
        );
        Ok(())
    })
    .unwrap();
    std::fs::write(&key, b"corrupt private key").unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::TlsNotReady)
        );
        Ok(())
    })
    .unwrap();
    let mismatched = rcgen::KeyPair::generate().unwrap().serialize_pem();
    std::fs::write(&key, mismatched).unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::TlsNotReady)
        );
        Ok(())
    })
    .unwrap();
    std::fs::write(&key, original).unwrap();
    db.with_conn_sync(|conn| {
        assert!(require_network_authority(conn, dir.path()).is_ok());
        Ok(())
    })
    .unwrap();
}

#[test]
fn ingress_gate_requires_the_exact_approved_tls_generation_bytes() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let db = iotkit_core_storage::init_db_memory(&migrations()).unwrap();
    own(&db);
    let dir = tempfile::tempdir().unwrap();
    db.with_conn_sync(|conn| {
        iotkit_gateway::api::tls::ensure_tls_material(conn, dir.path())
            .map(|_| ())
            .map_err(|error| {
                iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                    Box::new(error),
                ))
            })
    })
    .unwrap();
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["ingress.test".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let cert_pem = cert.pem();
    let key_pem = key.serialize_pem();
    let fingerprint = iotkit_core_ops::fingerprint_of_pem(&cert_pem).unwrap();
    let generation = dir.path().join("ingress-tls/generation-1");
    std::fs::create_dir_all(&generation).unwrap();
    std::fs::write(generation.join("cert.pem"), cert_pem).unwrap();
    std::fs::write(generation.join("key.pem"), key_pem).unwrap();
    db.with_conn_sync(|conn| {
        conn.execute(
            "UPDATE ingress_listener_config SET enabled=1,desired_generation=1,
             applied_generation=1,bind_addr='192.168.1.2:8444',interface='eth0',
             site_local_cidrs='[\"192.168.1.0/24\"]',mode='tls',
             desired_tls_generation=1,desired_tls_fingerprint=?1,
             applied_bind_addr='192.168.1.2:8444',applied_interface='eth0',
             applied_site_local_cidrs='[\"192.168.1.0/24\"]',applied_mode='tls',
             applied_tls_generation=1,applied_tls_fingerprint=?1 WHERE id=1",
            [&fingerprint],
        )?;
        assert!(require_network_authority(conn, dir.path()).is_ok());
        Ok(())
    })
    .unwrap();
    std::fs::remove_file(generation.join("key.pem")).unwrap();
    db.with_conn_sync(|conn| {
        assert_eq!(
            require_network_authority(conn, dir.path()),
            Err(NetworkAuthorityError::TlsNotReady)
        );
        Ok(())
    })
    .unwrap();
}
