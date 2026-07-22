use std::fs;

use super::{certificate_params, ensure_tls_material};
use iotkit_core_ops::fingerprint_of_pem;
use rcgen::{DnType, DnValue, SanType};

fn db() -> iotkit_core_storage::DbHandle {
    let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_registry::MIGRATIONS);
    migrations.extend_from_slice(iotkit_core_ops::MIGRATIONS);
    migrations.sort_by_key(|migration| migration.version);
    iotkit_core_storage::init_db_memory(&migrations).unwrap()
}

fn ensure(db: &iotkit_core_storage::DbHandle, path: &std::path::Path) -> super::TlsMaterial {
    db.with_conn_sync(|conn| {
        ensure_tls_material(conn, path).map_err(|error| {
            iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                Box::new(error),
            ))
        })
    })
    .unwrap()
}

#[test]
fn initial_generation_writes_cert_key_and_matching_fingerprint() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();

    let db = db();
    let material = ensure(&db, dir.path());

    let cert_pem = fs::read_to_string(&material.cert_pem_path).unwrap();
    assert_eq!(material.fingerprint, fingerprint_of_pem(&cert_pem).unwrap());
    db.with_conn_sync(|conn| {
        let (generation, fingerprint): (i64, String) = conn.query_row(
            "SELECT generation, fingerprint FROM tls_identity WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(generation, 1);
        assert_eq!(fingerprint, material.fingerprint);
        Ok(())
    })
    .unwrap();
    assert!(material.cert_pem_path.exists());
    assert!(material.key_pem_path.exists());
    assert!(!material.cert_pem_path.with_extension("pem.tmp").exists());
    assert!(!material.key_pem_path.with_extension("pem.tmp").exists());
    assert_eq!(material.cert_pem_path.file_name().unwrap(), "cert.pem");
    assert_eq!(material.key_pem_path.file_name().unwrap(), "key.pem");
}

#[test]
fn default_certificate_uses_edge_common_name_and_subject_alt_name() {
    let params = certificate_params().unwrap();
    assert_eq!(
        params.distinguished_name.get(&DnType::CommonName),
        Some(&DnValue::Utf8String("iotkit-edge-node".to_string()))
    );
    assert!(params.subject_alt_names.contains(&SanType::DnsName(
        "iotkit-edge-node.local".try_into().unwrap()
    )));
}

#[test]
fn second_call_reuses_existing_material() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let db = db();
    let first = ensure(&db, dir.path());
    let first_cert = fs::read_to_string(&first.cert_pem_path).unwrap();
    let first_key = fs::read_to_string(&first.key_pem_path).unwrap();

    let second = ensure(&db, dir.path());

    assert_eq!(first.fingerprint, second.fingerprint);
    assert_eq!(
        first_cert,
        fs::read_to_string(&second.cert_pem_path).unwrap()
    );
    assert_eq!(first_key, fs::read_to_string(&second.key_pem_path).unwrap());
}

#[test]
fn partial_pair_fails_closed_without_replacing_the_certificate() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let db = db();
    let first = ensure(&db, dir.path());
    let first_cert = fs::read_to_string(&first.cert_pem_path).unwrap();
    fs::remove_file(&first.key_pem_path).unwrap();

    let error = db
        .with_conn_sync(|conn| {
            Ok(match ensure_tls_material(conn, dir.path()) {
                Ok(_) => panic!("partial TLS must fail closed"),
                Err(error) => error,
            })
        })
        .unwrap();

    assert!(error.to_string().contains("partial"));
    assert_eq!(
        first_cert,
        fs::read_to_string(&first.cert_pem_path).unwrap()
    );
    assert!(!first.key_pem_path.exists());
}

#[test]
fn initialized_identity_with_both_files_lost_fails_closed() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let db = db();
    let first = ensure(&db, dir.path());
    fs::remove_file(&first.cert_pem_path).unwrap();
    fs::remove_file(&first.key_pem_path).unwrap();

    let error = db
        .with_conn_sync(|conn| {
            Ok(match ensure_tls_material(conn, dir.path()) {
                Ok(_) => panic!("initialized TLS identity must not regenerate"),
                Err(error) => error,
            })
        })
        .unwrap();

    assert!(
        error
            .to_string()
            .contains("both certificate and private key are missing")
    );
    assert!(!first.cert_pem_path.exists());
    assert!(!first.key_pem_path.exists());
}

#[test]
fn initialized_identity_rejects_a_different_complete_certificate() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let first_db = db();
    let second_db = db();
    let first = ensure(&first_db, first_dir.path());
    let second = ensure(&second_db, second_dir.path());
    fs::copy(second.cert_pem_path, &first.cert_pem_path).unwrap();
    fs::copy(second.key_pem_path, &first.key_pem_path).unwrap();

    let error = first_db
        .with_conn_sync(|conn| {
            Ok(match ensure_tls_material(conn, first_dir.path()) {
                Ok(_) => panic!("different TLS identity must fail"),
                Err(error) => error,
            })
        })
        .unwrap();
    assert!(error.to_string().contains("does not match"));
}

#[test]
fn corrupt_certificate_fails_closed_without_regeneration() {
    let dir = tempfile::tempdir().unwrap();
    let tls_dir = dir.path().join("tls");
    fs::create_dir(&tls_dir).unwrap();
    fs::write(tls_dir.join("cert.pem"), "not a certificate").unwrap();
    fs::write(tls_dir.join("key.pem"), "not a private key").unwrap();

    let db = db();
    assert!(
        db.with_conn_sync(|conn| Ok(ensure_tls_material(conn, dir.path()).is_err()))
            .unwrap()
    );
    assert_eq!(
        fs::read_to_string(tls_dir.join("cert.pem")).unwrap(),
        "not a certificate"
    );
}

#[tokio::test]
async fn mismatched_complete_pair_is_rejected_by_tls_configuration() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let first_db = db();
    let second_db = db();
    let first = ensure(&first_db, first_dir.path());
    let second = ensure(&second_db, second_dir.path());
    fs::copy(second.key_pem_path, &first.key_pem_path).unwrap();

    let result = axum_server::tls_rustls::RustlsConfig::from_pem_file(
        &first.cert_pem_path,
        &first.key_pem_path,
    )
    .await;
    assert!(result.is_err(), "mismatched TLS pair must fail closed");
}

#[cfg(unix)]
#[test]
fn uses_private_permissions_for_tls_dir_and_key() {
    use std::os::unix::fs::PermissionsExt;

    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let db = db();
    let material = ensure(&db, dir.path());

    let tls_dir = dir.path().join("tls");
    assert_eq!(
        fs::metadata(tls_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(material.key_pem_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
