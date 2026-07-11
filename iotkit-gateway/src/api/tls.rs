use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use iotkit_core_ops::{OpsError, fingerprint_of_pem};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
use rusqlite::{Connection, OptionalExtension, params};

pub struct TlsMaterial {
    pub cert_pem_path: PathBuf,
    pub key_pem_path: PathBuf,
    pub fingerprint: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Rcgen(#[from] rcgen::Error),
    #[error(transparent)]
    Fingerprint(#[from] OpsError),
    #[error("partial TLS material: certificate and private key must both exist")]
    PartialMaterial,
    #[error("TLS identity was initialized but both certificate and private key are missing")]
    MissingInitializedMaterial,
    #[error("TLS certificate fingerprint does not match the initialized identity")]
    IdentityMismatch,
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub fn ensure_tls_material(conn: &Connection, data_dir: &Path) -> Result<TlsMaterial, TlsError> {
    let tls_dir = data_dir.join("tls");
    let cert_pem_path = tls_dir.join("cert.pem");
    let key_pem_path = tls_dir.join("key.pem");

    ensure_tls_dir(&tls_dir)?;

    let cert_exists = cert_pem_path.exists();
    let key_exists = key_pem_path.exists();
    let expected: Option<(i64, String)> = conn
        .query_row(
            "SELECT generation, fingerprint FROM tls_identity WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if cert_exists != key_exists {
        return Err(TlsError::PartialMaterial);
    }
    if cert_exists {
        let cert_pem = fs::read_to_string(&cert_pem_path)?;
        let fingerprint = fingerprint_of_pem(&cert_pem)?;
        if let Some((_, expected_fingerprint)) = expected {
            if fingerprint != expected_fingerprint {
                return Err(TlsError::IdentityMismatch);
            }
        } else {
            persist_initial_identity(conn, &fingerprint)?;
        }
        return Ok(TlsMaterial {
            cert_pem_path,
            key_pem_path,
            fingerprint,
        });
    }
    if expected.is_some() {
        return Err(TlsError::MissingInitializedMaterial);
    }

    let (cert_pem, key_pem) = generate_tls_pair()?;
    write_atomic(&cert_pem_path, cert_pem.as_bytes(), 0o644)?;
    write_atomic(&key_pem_path, key_pem.as_bytes(), 0o600)?;
    let fingerprint = fingerprint_of_pem(&cert_pem)?;
    persist_initial_identity(conn, &fingerprint)?;

    Ok(TlsMaterial {
        cert_pem_path,
        key_pem_path,
        fingerprint,
    })
}

fn persist_initial_identity(conn: &Connection, fingerprint: &str) -> Result<(), TlsError> {
    conn.execute(
        "INSERT INTO tls_identity (id, generation, fingerprint, initialized_at)
         VALUES (1, 1, ?1, ?2)",
        params![fingerprint, now_ms()],
    )?;
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn generate_tls_pair() -> Result<(String, String), TlsError> {
    let key_pair = KeyPair::generate()?;
    let mut params = CertificateParams::new(subject_alt_names())?;
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, "iotkit-gateway");
    params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(365 * 100);

    let cert = params.self_signed(&key_pair)?;
    Ok((cert.pem(), key_pair.serialize_pem()))
}

fn subject_alt_names() -> Vec<String> {
    let mut names = vec!["iotkit-gateway.local".to_string()];
    if let Some(hostname) = hostname() {
        let hostname = hostname.trim();
        if !hostname.is_empty() && hostname != "iotkit-gateway.local" {
            names.push(hostname.to_string());
        }
    }
    names
}

pub(crate) fn hostname() -> Option<String> {
    let mut buf = [0_u8; 256];
    // SAFETY: `buf` is a valid writable byte buffer and its length is passed unchanged.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return None;
    }
    let len = buf.iter().position(|byte| *byte == 0).unwrap_or(buf.len());
    let hostname = String::from_utf8_lossy(&buf[..len]).into_owned();
    (!hostname.trim().is_empty()).then_some(hostname)
}

fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), TlsError> {
    let tmp = temp_path(path);
    let mut file = fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    set_permissions(&tmp, mode)?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, path)?;
    Ok(())
}

fn temp_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.tmp", path.display()))
}

fn ensure_tls_dir(path: &Path) -> Result<(), TlsError> {
    fs::create_dir_all(path)?;
    set_permissions(path, 0o700)?;
    Ok(())
}

#[cfg(unix)]
fn set_permissions(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_permissions(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::ensure_tls_material;
    use iotkit_core_ops::fingerprint_of_pem;

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
}
