use base64::Engine;
use sha2::{Digest, Sha256};

use crate::OpsError;

pub fn fingerprint_of_pem(cert_pem: &str) -> Result<String, OpsError> {
    let mut base64_body = String::new();
    let mut in_cert = false;

    for line in cert_pem.lines() {
        let trimmed = line.trim();
        match trimmed {
            "-----BEGIN CERTIFICATE-----" => {
                in_cert = true;
            }
            "-----END CERTIFICATE-----" => {
                in_cert = false;
            }
            _ if in_cert => base64_body.push_str(trimmed),
            _ => {}
        }
    }

    if base64_body.is_empty() {
        return Err(OpsError::Validation(
            "certificate PEM block not found".to_string(),
        ));
    }

    let der = base64::engine::general_purpose::STANDARD
        .decode(base64_body)
        .map_err(|e| OpsError::Validation(format!("invalid certificate PEM base64: {e}")))?;
    let digest = Sha256::digest(&der);
    Ok(digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(":"))
}
