use std::collections::BTreeMap;

use iotkit_output_adapter_api::{Observation, ObservationKind, ObservationValue};
use serde_json::{Map, Value, value::RawValue};

use crate::{
    application::semantics::SemanticRule,
    composition::OutputAdapterRegistration,
    storage::{AuditActor, Storage, StorageError},
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicationProvenance {
    Actual,
    LatestObservation,
    Sample,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDelivery {
    pub state: String,
    pub pending_count: i64,
    pub published_count: i64,
    pub oldest_pending_at: Option<i64>,
    pub last_published_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputPublicationPreview {
    pub binding_id: String,
    pub provenance: PublicationProvenance,
    pub topic: String,
    pub qos: u8,
    pub retain: bool,
    pub payload: Value,
    pub delivery: OutputDelivery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicationFailureKind {
    Preview,
    DeliveryUnavailable,
}

pub(crate) struct PublicationFailure {
    pub kind: PublicationFailureKind,
    pub delivery: Option<OutputDelivery>,
    error: StorageError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRuleActivationPreview {
    pub rule_id: String,
    pub state: String,
    pub compatible_modes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportProfileActivationPreview {
    pub adapter_id: String,
    pub automatic_count: usize,
    pub needs_configuration_count: usize,
    pub ineligible_count: usize,
    pub rules: Vec<ExportRuleActivationPreview>,
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
        self.activate_as(
            AuditActor::local_cli(),
            display_name,
            adapter_id,
            values,
            now,
        )
        .await
    }

    pub async fn activate_as(
        &self,
        actor: AuditActor,
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
            .activate_output_profile_as(actor, display_name, registration, values, now)
            .await
    }

    pub async fn preview_activation(
        &self,
        adapter_id: &str,
    ) -> Result<ExportProfileActivationPreview, StorageError> {
        let rules = self.storage.list_semantic_rules().await?;
        self.preview_activation_with_rules(adapter_id, &rules)
    }

    pub(crate) fn preview_activation_with_rules(
        &self,
        adapter_id: &str,
        rules: &[SemanticRule],
    ) -> Result<ExportProfileActivationPreview, StorageError> {
        let registration = self
            .adapters
            .iter()
            .find(|item| item.adapter.descriptor().id == adapter_id)
            .ok_or_else(|| StorageError::InvalidOutput("unknown output adapter".into()))?;
        let mut automatic_count = 0;
        let mut needs_configuration_count = 0;
        let mut ineligible_count = 0;
        let mut rule_previews = Vec::new();
        for rule in rules {
            if !rule.active {
                continue;
            }
            let kind = observation_kind(rule.kind);
            let compatible_modes: Vec<String> = registration
                .adapter
                .descriptor()
                .modes
                .iter()
                .filter(|mode| mode.accepts.contains(&kind))
                .map(|mode| mode.key.to_owned())
                .collect();
            let state = match compatible_modes.len() {
                0 => {
                    ineligible_count += 1;
                    "ineligible"
                }
                1 => {
                    automatic_count += 1;
                    "automatic"
                }
                _ => {
                    needs_configuration_count += 1;
                    "needs_configuration"
                }
            };
            rule_previews.push(ExportRuleActivationPreview {
                rule_id: rule.rule_id.clone(),
                state: state.into(),
                compatible_modes,
            });
        }
        Ok(ExportProfileActivationPreview {
            adapter_id: adapter_id.into(),
            automatic_count,
            needs_configuration_count,
            ineligible_count,
            rules: rule_previews,
        })
    }

    pub async fn publication(
        &self,
        binding_id: &str,
        now: i64,
    ) -> Result<OutputPublicationPreview, StorageError> {
        self.publication_with_failure(binding_id, now)
            .await
            .map_err(|failure| failure.error)
    }

    pub(crate) async fn publication_with_failure(
        &self,
        binding_id: &str,
        now: i64,
    ) -> Result<OutputPublicationPreview, PublicationFailure> {
        let snapshot = self
            .storage
            .output_publication_snapshot(binding_id)
            .await
            .map_err(|error| PublicationFailure {
                kind: PublicationFailureKind::DeliveryUnavailable,
                delivery: None,
                error,
            })?;
        let delivery = OutputDelivery {
            state: delivery_state(
                snapshot.pending_count,
                snapshot.published_count,
                snapshot.oldest_pending_at,
                now,
            )
            .into(),
            pending_count: snapshot.pending_count,
            published_count: snapshot.published_count,
            oldest_pending_at: snapshot.oldest_pending_at,
            last_published_at: snapshot.last_published_at,
        };
        if let Some(actual) = snapshot.actual {
            return Ok(OutputPublicationPreview {
                binding_id: binding_id.into(),
                provenance: PublicationProvenance::Actual,
                topic: actual.topic,
                qos: actual.qos,
                retain: actual.retain,
                payload: serde_json::from_slice(&actual.payload).map_err(|error| {
                    preview_failure(StorageError::InvalidOutput(error.to_string()), &delivery)
                })?,
                delivery,
            });
        }
        let registration = self
            .adapters
            .iter()
            .find(|item| item.adapter.descriptor().id == snapshot.adapter_id)
            .ok_or_else(|| {
                preview_failure(
                    StorageError::InvalidOutput("output adapter is unavailable".into()),
                    &delivery,
                )
            })?;
        let config: Box<RawValue> = serde_json::from_slice(&snapshot.config).map_err(|error| {
            preview_failure(StorageError::InvalidOutput(error.to_string()), &delivery)
        })?;
        let (observation, provenance) = match snapshot.observation {
            Some(observation) => (
                Observation::new(
                    observation.observation_id,
                    observation.series_id,
                    observation.sequence,
                    observation.observed_at,
                    observation.value,
                )
                .map_err(|error| {
                    preview_failure(StorageError::InvalidOutput(error.to_string()), &delivery)
                })?,
                PublicationProvenance::LatestObservation,
            ),
            None => (
                sample_observation(snapshot.kind, now)
                    .map_err(|error| preview_failure(error, &delivery))?,
                PublicationProvenance::Sample,
            ),
        };
        let publication = registration
            .adapter
            .transform(&config, &observation)
            .map_err(|error| {
                preview_failure(StorageError::InvalidOutput(error.to_string()), &delivery)
            })?;
        Ok(OutputPublicationPreview {
            binding_id: binding_id.into(),
            provenance,
            topic: publication.topic().into(),
            qos: publication.qos(),
            retain: publication.retain(),
            payload: serde_json::from_str(publication.payload().get()).map_err(|error| {
                preview_failure(StorageError::InvalidOutput(error.to_string()), &delivery)
            })?,
            delivery,
        })
    }

    pub async fn configure(
        &self,
        binding_id: &str,
        mode: &str,
        values: Map<String, Value>,
        now: i64,
    ) -> Result<OutputBinding, StorageError> {
        self.configure_as(AuditActor::local_cli(), binding_id, mode, values, now)
            .await
    }

    pub async fn configure_as(
        &self,
        actor: AuditActor,
        binding_id: &str,
        mode: &str,
        values: Map<String, Value>,
        now: i64,
    ) -> Result<OutputBinding, StorageError> {
        self.storage
            .configure_output_binding_as(actor, binding_id, mode, values, self.adapters, now)
            .await
    }

    pub async fn confirm(&self, binding_id: &str, now: i64) -> Result<(), StorageError> {
        self.confirm_as(AuditActor::local_cli(), binding_id, now)
            .await
    }

    pub async fn confirm_as(
        &self,
        actor: AuditActor,
        binding_id: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        self.storage
            .confirm_output_binding_as(actor, binding_id, now)
            .await
    }

    pub async fn stop(&self, profile_id: &str, now: i64) -> Result<(), StorageError> {
        self.stop_as(AuditActor::local_cli(), profile_id, now).await
    }

    pub async fn stop_as(
        &self,
        actor: AuditActor,
        profile_id: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        self.storage
            .stop_output_profile_as(actor, profile_id, now)
            .await
    }

    pub async fn list(&self) -> Result<Vec<ExportProfile>, StorageError> {
        self.storage.list_output_profiles().await
    }
}

fn preview_failure(error: StorageError, delivery: &OutputDelivery) -> PublicationFailure {
    PublicationFailure {
        kind: PublicationFailureKind::Preview,
        delivery: Some(delivery.clone()),
        error,
    }
}

fn observation_kind(kind: crate::semantics::SemanticKind) -> ObservationKind {
    match kind {
        crate::semantics::SemanticKind::Numeric => ObservationKind::Numeric,
        crate::semantics::SemanticKind::Boolean => ObservationKind::Boolean,
        crate::semantics::SemanticKind::CumulativeCounter => ObservationKind::CumulativeValue,
        crate::semantics::SemanticKind::Alarm => ObservationKind::Alarm,
    }
}

fn sample_observation(
    kind: crate::semantics::SemanticKind,
    now: i64,
) -> Result<Observation, StorageError> {
    let value = match kind {
        crate::semantics::SemanticKind::Numeric => ObservationValue::Numeric(0.0),
        crate::semantics::SemanticKind::Boolean => ObservationValue::Boolean(false),
        crate::semantics::SemanticKind::CumulativeCounter => ObservationValue::CumulativeValue(0),
        crate::semantics::SemanticKind::Alarm => ObservationValue::Alarm {
            active: false,
            reading: Some(0.0),
        },
    };
    Observation::new(
        "00000000-0000-5000-8000-000000000001",
        "00000000-0000-5000-8000-000000000002",
        1,
        now.max(0),
        value,
    )
    .map_err(|error| StorageError::InvalidOutput(error.to_string()))
}

fn delivery_state(
    pending_count: i64,
    published_count: i64,
    oldest_pending_at: Option<i64>,
    now: i64,
) -> &'static str {
    if pending_count > 0 {
        if oldest_pending_at.is_some_and(|created_at| now.saturating_sub(created_at) >= 300_000) {
            "possible_delivery_stall"
        } else {
            "delivering"
        }
    } else if published_count > 0 {
        "published"
    } else {
        "waiting_for_observation"
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

#[cfg(test)]
#[path = "../../tests/unit/output_profiles_tests.rs"]
mod tests;
