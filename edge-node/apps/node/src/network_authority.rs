use std::path::Path;

use iotkit_core_ops::{IngressListenerConfig, OwnershipState};
use rusqlite::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NetworkAuthorityError {
    #[error("unowned")]
    Unowned,
    #[error("local_recovery_required")]
    LocalRecoveryRequired,
    #[error("unsafe_ingress_generation_state")]
    UnsafeIngressGeneration,
    #[error("authority_state_unknown")]
    Unknown,
    #[error("tls_not_ready")]
    TlsNotReady,
}

impl NetworkAuthorityError {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Unowned => "unowned",
            Self::LocalRecoveryRequired => "local_recovery_required",
            Self::UnsafeIngressGeneration => "unsafe_ingress_generation_state",
            Self::Unknown => "authority_state_unknown",
            Self::TlsNotReady => "tls_not_ready",
        }
    }
}

/// Common prerequisite for every network listener. TLS material remains listener-specific, but
/// ownership and desired/applied generation safety are shared and fail closed.
pub fn require_network_authority(
    conn: &Connection,
    data_dir: &Path,
) -> Result<IngressListenerConfig, NetworkAuthorityError> {
    require_common_network_authority(conn, data_dir)?;
    crate::api::tls::validate_existing_tls_material(conn, data_dir)
        .map_err(|_| NetworkAuthorityError::TlsNotReady)?;
    let config = iotkit_core_ops::load_ingress_listener_config(conn)
        .map_err(|_| NetworkAuthorityError::UnsafeIngressGeneration)?;
    if config.enabled
        && config.applied.as_ref().map(|state| state.generation) != Some(config.desired.generation)
    {
        return Err(NetworkAuthorityError::UnsafeIngressGeneration);
    }
    if config.enabled {
        crate::ingress::validate_ingress_tls_material(&config, data_dir)
            .map_err(|_| NetworkAuthorityError::TlsNotReady)?;
    }
    Ok(config)
}

pub fn require_common_network_authority(
    conn: &Connection,
    _data_dir: &Path,
) -> Result<(), NetworkAuthorityError> {
    match iotkit_core_ops::ownership_state(conn).map_err(|_| NetworkAuthorityError::Unknown)? {
        OwnershipState::Owned => {}
        OwnershipState::Unowned => return Err(NetworkAuthorityError::Unowned),
        OwnershipState::LocalRecoveryRequired => {
            return Err(NetworkAuthorityError::LocalRecoveryRequired);
        }
    }
    Ok(())
}
