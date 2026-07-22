use super::*;

impl IngestPrincipal {
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
}
