use std::collections::HashSet;

use iotkit_core_ledger::SystemId;

/// The receiver-authenticated actor class for an ingest request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestActorKind {
    /// A trusted official adapter running in the gateway process.
    OfficialAdapter,
    /// An externally authenticated device credential.
    DeviceToken,
}

#[derive(Debug, Clone)]
enum SubjectScope {
    OfficialDiscovery,
    Restricted(HashSet<SystemId>),
}

/// Receiver-created ingest identity and authority.
///
/// This value is never sender-serialized. Collector authority, deduplication,
/// flow ownership, and sighting attribution derive from it rather than from
/// `Envelope.source`.
#[derive(Debug, Clone)]
pub struct IngestPrincipal {
    principal_id: String,
    credential_id: Option<String>,
    configured_source: String,
    scope: SubjectScope,
    flow_profile: String,
    auth_epoch: Option<String>,
    auth_generation: Option<i64>,
    principal_material_generation: Option<i64>,
    actor_kind: IngestActorKind,
}

pub struct AuthenticatedDeviceIdentity {
    principal_id: String,
    credential_id: String,
    configured_source: String,
    flow_profile: String,
}

impl AuthenticatedDeviceIdentity {
    pub fn new(
        principal_id: impl Into<String>,
        credential_id: impl Into<String>,
        configured_source: impl Into<String>,
        flow_profile: impl Into<String>,
    ) -> Self {
        Self {
            principal_id: principal_id.into(),
            credential_id: credential_id.into(),
            configured_source: configured_source.into(),
            flow_profile: flow_profile.into(),
        }
    }
}

pub struct DeviceAuthorityProof {
    auth_epoch: String,
    auth_generation: i64,
    principal_material_generation: i64,
}

impl DeviceAuthorityProof {
    pub fn new(
        auth_epoch: impl Into<String>,
        auth_generation: i64,
        principal_material_generation: i64,
    ) -> Self {
        Self {
            auth_epoch: auth_epoch.into(),
            auth_generation,
            principal_material_generation,
        }
    }
}

impl IngestPrincipal {
    pub(crate) fn trusted_official_adapter(
        principal_id: impl Into<String>,
        configured_source: impl Into<String>,
    ) -> Self {
        Self {
            principal_id: principal_id.into(),
            credential_id: None,
            configured_source: configured_source.into(),
            scope: SubjectScope::OfficialDiscovery,
            flow_profile: "trusted_in_process".into(),
            auth_epoch: None,
            auth_generation: None,
            principal_material_generation: None,
            actor_kind: IngestActorKind::OfficialAdapter,
        }
    }

    pub(crate) fn authenticated_device(
        identity: AuthenticatedDeviceIdentity,
        allowed_subjects: impl IntoIterator<Item = SystemId>,
        proof: DeviceAuthorityProof,
    ) -> Self {
        Self {
            principal_id: identity.principal_id,
            credential_id: Some(identity.credential_id),
            configured_source: identity.configured_source,
            scope: SubjectScope::Restricted(allowed_subjects.into_iter().collect()),
            flow_profile: identity.flow_profile,
            auth_epoch: Some(proof.auth_epoch),
            auth_generation: Some(proof.auth_generation),
            principal_material_generation: Some(proof.principal_material_generation),
            actor_kind: IngestActorKind::DeviceToken,
        }
    }

    #[cfg(test)]
    pub(crate) fn test_authenticated_device(
        principal_id: impl Into<String>,
        credential_id: impl Into<String>,
        configured_source: impl Into<String>,
        allowed_subjects: impl IntoIterator<Item = SystemId>,
        flow_profile: impl Into<String>,
        auth_epoch: impl Into<String>,
    ) -> Self {
        Self {
            principal_id: principal_id.into(),
            credential_id: Some(credential_id.into()),
            configured_source: configured_source.into(),
            scope: SubjectScope::Restricted(allowed_subjects.into_iter().collect()),
            flow_profile: flow_profile.into(),
            auth_epoch: Some(auth_epoch.into()),
            auth_generation: None,
            principal_material_generation: None,
            actor_kind: IngestActorKind::DeviceToken,
        }
    }

    /// Stable deduplication and audit namespace. Auth epochs never enter this ID.
    pub fn principal_id(&self) -> &str {
        &self.principal_id
    }

    /// Stable credential identifier, when the principal came from a credential.
    pub fn credential_id(&self) -> Option<&str> {
        self.credential_id.as_deref()
    }

    /// Configured diagnostic source identity expected on the envelope.
    pub fn configured_source(&self) -> &str {
        &self.configured_source
    }

    /// Receiver-owned flow profile name.
    pub fn flow_profile(&self) -> &str {
        &self.flow_profile
    }

    /// Authentication cache epoch. It is intentionally absent from dedup identity.
    pub fn auth_epoch(&self) -> Option<&str> {
        self.auth_epoch.as_deref()
    }

    /// Authenticated actor class.
    pub fn actor_kind(&self) -> IngestActorKind {
        self.actor_kind
    }

    pub(crate) fn sole_subject(&self) -> Option<SystemId> {
        match &self.scope {
            SubjectScope::Restricted(subjects) if subjects.len() == 1 => {
                subjects.iter().next().copied()
            }
            _ => None,
        }
    }

    pub(crate) fn subject_allowed(&self, subject: &SystemId) -> bool {
        match &self.scope {
            SubjectScope::OfficialDiscovery => true,
            SubjectScope::Restricted(subjects) => subjects.contains(subject),
        }
    }

    pub(crate) fn can_stage_unknown(&self) -> bool {
        matches!(self.scope, SubjectScope::OfficialDiscovery)
            && self.actor_kind == IngestActorKind::OfficialAdapter
    }

    pub(crate) fn auth_generation(&self) -> Option<i64> {
        self.auth_generation
    }

    pub(crate) fn principal_material_generation(&self) -> Option<i64> {
        self.principal_material_generation
    }
}

/// Receiver-composition capability for creating trusted local principals.
///
/// The capability has no public constructor and is not cloneable. Collector
/// composition returns it to the gateway, which binds each local principal into
/// one sender handle. Sender crates receive neither this capability nor principal
/// constructors. Task 5 must introduce a separate, authenticator-only
/// device-principal boundary; HTTP request code must receive only the resulting
/// principal and cannot use this local-authority capability.
pub struct LocalPrincipalIssuer {
    _private: (),
}

/// Non-cloneable composition capability for turning an authenticated device record into the
/// collector's receiver-owned principal. HTTP handlers receive principals, never this authority.
pub struct DevicePrincipalIssuer {
    _private: (),
}

impl DevicePrincipalIssuer {
    pub(crate) fn new() -> Self {
        Self { _private: () }
    }

    pub fn authenticated_device(
        &self,
        identity: AuthenticatedDeviceIdentity,
        allowed_subjects: impl IntoIterator<Item = SystemId>,
        proof: DeviceAuthorityProof,
    ) -> IngestPrincipal {
        IngestPrincipal::authenticated_device(identity, allowed_subjects, proof)
    }
}

impl LocalPrincipalIssuer {
    pub(crate) fn new() -> Self {
        Self { _private: () }
    }

    /// Create a principal for one trusted official in-process adapter handle.
    pub fn official_adapter(
        &self,
        principal_id: impl Into<String>,
        configured_source: impl Into<String>,
    ) -> IngestPrincipal {
        IngestPrincipal::trusted_official_adapter(principal_id, configured_source)
    }
}
