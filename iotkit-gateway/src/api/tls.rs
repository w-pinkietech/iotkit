use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use iotkit_core_ops::{OpsError, fingerprint_of_pem};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};

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
}

pub fn ensure_tls_material(data_dir: &Path) -> Result<TlsMaterial, TlsError> {
    let tls_dir = data_dir.join("tls");
    let cert_pem_path = tls_dir.join("cert.pem");
    let key_pem_path = tls_dir.join("key.pem");

    ensure_tls_dir(&tls_dir)?;

    if cert_pem_path.exists() && key_pem_path.exists() {
        let cert_pem = fs::read_to_string(&cert_pem_path)?;
        return Ok(TlsMaterial {
            cert_pem_path,
            key_pem_path,
            fingerprint: fingerprint_of_pem(&cert_pem)?,
        });
    }

    let (cert_pem, key_pem) = generate_tls_pair()?;
    write_atomic(&cert_pem_path, cert_pem.as_bytes(), 0o644)?;
    write_atomic(&key_pem_path, key_pem.as_bytes(), 0o600)?;
    let fingerprint = fingerprint_of_pem(&cert_pem)?;

    Ok(TlsMaterial {
        cert_pem_path,
        key_pem_path,
        fingerprint,
    })
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

    #[test]
    fn initial_generation_writes_cert_key_and_matching_fingerprint() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = tempfile::tempdir().unwrap();

        let material = ensure_tls_material(dir.path()).unwrap();

        let cert_pem = fs::read_to_string(&material.cert_pem_path).unwrap();
        assert_eq!(material.fingerprint, fingerprint_of_pem(&cert_pem).unwrap());
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
        let first = ensure_tls_material(dir.path()).unwrap();
        let first_cert = fs::read_to_string(&first.cert_pem_path).unwrap();
        let first_key = fs::read_to_string(&first.key_pem_path).unwrap();

        let second = ensure_tls_material(dir.path()).unwrap();

        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(
            first_cert,
            fs::read_to_string(&second.cert_pem_path).unwrap()
        );
        assert_eq!(first_key, fs::read_to_string(&second.key_pem_path).unwrap());
    }

    #[test]
    fn partial_pair_regenerates_both_files() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = tempfile::tempdir().unwrap();
        let first = ensure_tls_material(dir.path()).unwrap();
        let first_cert = fs::read_to_string(&first.cert_pem_path).unwrap();
        fs::remove_file(&first.key_pem_path).unwrap();

        let second = ensure_tls_material(dir.path()).unwrap();

        assert_ne!(first.fingerprint, second.fingerprint);
        assert_ne!(
            first_cert,
            fs::read_to_string(&second.cert_pem_path).unwrap()
        );
        assert!(second.cert_pem_path.exists());
        assert!(second.key_pem_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn uses_private_permissions_for_tls_dir_and_key() {
        use std::os::unix::fs::PermissionsExt;

        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = tempfile::tempdir().unwrap();
        let material = ensure_tls_material(dir.path()).unwrap();

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
