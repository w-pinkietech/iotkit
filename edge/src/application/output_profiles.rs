use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::{
    composition::OutputAdapterRegistration,
    storage::{Storage, StorageError},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileState {
    Preparing,
    Active,
    Draining,
    Stopped,
}

impl ProfileState {
    pub(crate) fn parse(value: &str) -> Result<Self, StorageError> {
        match value {
            "preparing" => Ok(Self::Preparing),
            "active" => Ok(Self::Active),
            "draining" => Ok(Self::Draining),
            "stopped" => Ok(Self::Stopped),
            _ => Err(StorageError::InvalidOutput(
                "database contains an invalid profile state".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputBinding {
    pub binding_id: String,
    pub rule_id: String,
    pub external_id: String,
    pub mode: Option<String>,
    pub active: bool,
    pub needs_configuration: bool,
    pub ineligible_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportProfile {
    pub profile_id: String,
    pub display_name: String,
    pub adapter_id: String,
    pub state: ProfileState,
    pub revision: i64,
    pub bindings: Vec<OutputBinding>,
}

#[derive(Clone)]
pub struct OutputProfiles {
    storage: Storage,
    adapters: &'static [OutputAdapterRegistration],
}

impl OutputProfiles {
    #[must_use]
    pub fn new(storage: Storage, adapters: &'static [OutputAdapterRegistration]) -> Self {
        Self { storage, adapters }
    }

    pub async fn activate(
        &self,
        display_name: &str,
        adapter_id: &str,
        values: Map<String, Value>,
        now: i64,
    ) -> Result<ExportProfile, StorageError> {
        if display_name.is_empty() || display_name.len() > 128 {
            return Err(StorageError::InvalidOutput(
                "profile display name is required".into(),
            ));
        }
        let registration = self
            .adapters
            .iter()
            .find(|item| item.adapter.descriptor().id == adapter_id)
            .ok_or_else(|| StorageError::InvalidOutput("unknown output adapter".into()))?;
        validate_setup(registration, &values)?;
        self.storage
            .activate_output_profile(display_name, registration, values, now)
            .await
    }

    pub async fn configure(
        &self,
        binding_id: &str,
        mode: &str,
        values: Map<String, Value>,
        now: i64,
    ) -> Result<OutputBinding, StorageError> {
        self.storage
            .configure_output_binding(binding_id, mode, values, self.adapters, now)
            .await
    }

    pub async fn confirm(&self, binding_id: &str, now: i64) -> Result<(), StorageError> {
        self.storage.confirm_output_binding(binding_id, now).await
    }

    pub async fn stop(&self, profile_id: &str, now: i64) -> Result<(), StorageError> {
        self.storage.stop_output_profile(profile_id, now).await
    }

    pub async fn list(&self) -> Result<Vec<ExportProfile>, StorageError> {
        self.storage.list_output_profiles().await
    }
}

fn validate_setup(
    registration: &OutputAdapterRegistration,
    values: &Map<String, Value>,
) -> Result<(), StorageError> {
    let setup = registration.profile_policy.setup();
    let allowed: BTreeMap<_, _> = setup
        .fields
        .iter()
        .map(|field| (field.key, field))
        .collect();
    if values.keys().any(|key| !allowed.contains_key(key.as_str())) {
        return Err(StorageError::InvalidOutput(
            "profile setup contains an unknown field".into(),
        ));
    }
    if setup
        .fields
        .iter()
        .any(|field| field.required && !values.contains_key(field.key))
    {
        return Err(StorageError::InvalidOutput(
            "profile setup is missing a required field".into(),
        ));
    }
    Ok(())
}
