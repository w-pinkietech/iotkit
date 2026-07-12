use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use sha2::{Digest, Sha256};

use crate::OpsError;

pub fn fingerprint_of_pem(cert_pem: &str) -> Result<String, OpsError> {
    let certificates = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| OpsError::Validation("invalid certificate PEM".to_string()))?;
    let leaf = certificates
        .first()
        .ok_or_else(|| OpsError::Validation("certificate PEM block not found".to_string()))?;
    let digest = Sha256::digest(leaf.as_ref());
    Ok(digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(":"))
}
