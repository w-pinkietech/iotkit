//! Reversible compatibility views for the retired Go operator CLI.
//!
//! These identifiers never become domain identifiers: current semantic rules
//! and output routes remain the only persisted source of truth.

use serde::Serialize;
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::{
    application::semantics::SemanticRuleDraft,
    composition::OutputAdapterRegistration,
    semantics::{Detector, DetectorMode, RuleSpec, SemanticKind, TriggerMode},
    storage::{CliRouteDraft, Storage, StorageError},
};
use iotkit_output_adapter_api::ObservationKind;

#[derive(Debug, thiserror::Error)]
pub enum CliCompatibilityError {
    #[error("invalid semantic mapping ID")]
    InvalidMappingId,
    #[error("invalid MQTT route ID")]
    InvalidRouteId,
    #[error("{0}")]
    InvalidMapping(String),
    #[error("{0}")]
    InvalidRoute(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyTriggerMode {
    ActiveSample,
    ActiveEdge,
}

#[derive(Debug, Clone)]
pub struct LegacyMappingSpec {
    pub edge_node_id: String,
    pub series_key: String,
    pub meaning: String,
    pub trigger_mode: LegacyTriggerMode,
    pub active_value: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyMapping {
    pub mapping_id: String,
    pub revision: i64,
    pub edge_node_id: String,
    pub series_key: String,
    pub meaning: String,
    pub trigger_mode: LegacyTriggerMode,
    pub active_value: i32,
    pub active: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyRoute {
    pub route_id: String,
    pub mapping_id: String,
    pub topic: String,
    pub qos: u8,
    pub start_after_event_row_id: i64,
    pub active: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyRouteStatus {
    pub route_id: String,
    pub mapping_id: String,
    pub topic: String,
    pub qos: u8,
    pub start_after_event_row_id: i64,
    pub active: bool,
    pub created_at: i64,
    pub pending_count: i64,
    pub published_count: i64,
    pub oldest_pending_at: Option<i64>,
}

#[derive(Clone)]
pub struct LegacyRoutes {
    storage: Storage,
    registration: &'static OutputAdapterRegistration,
}

impl LegacyRoutes {
    #[must_use]
    pub fn new(storage: Storage, registration: &'static OutputAdapterRegistration) -> Self {
        Self {
            storage,
            registration,
        }
    }

    pub async fn add(
        &self,
        mapping_id: &str,
        topic: &str,
        now: i64,
    ) -> Result<LegacyRoute, CliCompatibilityError> {
        validate_topic(topic)?;
        let rule_id = rule_id_from_legacy_mapping(mapping_id)?;
        let config = serde_json::json!({"schema_version": 1, "topic": topic});
        let raw = serde_json::value::to_raw_value(&config)
            .map_err(|error| StorageError::InvalidOutput(error.to_string()))?;
        self.registration
            .adapter
            .validate_config(&raw, ObservationKind::CumulativeValue)
            .map_err(|error| StorageError::InvalidOutput(error.to_string()))?;
        let descriptor = self.registration.adapter.descriptor();
        let status = self
            .storage
            .add_cli_output_route(
                &CliRouteDraft {
                    rule_id,
                    adapter_id: descriptor.id.into(),
                    config_schema_version: i64::from(descriptor.config_schema_version),
                    config,
                },
                now,
            )
            .await?;
        Ok(LegacyRoute {
            route_id: legacy_route_id(&status.route_id)?,
            mapping_id: legacy_mapping_id(&status.rule_id)?,
            topic: route_topic(&status.config)?,
            qos: 1,
            start_after_event_row_id: status.start_after_observation_row_id,
            active: status.active,
            created_at: status.created_at,
        })
    }

    pub async fn list(&self) -> Result<Vec<LegacyRouteStatus>, CliCompatibilityError> {
        self.storage
            .list_cli_route_statuses()
            .await?
            .into_iter()
            .map(|status| {
                Ok(LegacyRouteStatus {
                    route_id: legacy_route_id(&status.route_id)?,
                    mapping_id: legacy_mapping_id(&status.rule_id)?,
                    topic: route_topic(&status.config)?,
                    qos: 1,
                    start_after_event_row_id: status.start_after_observation_row_id,
                    active: status.active,
                    created_at: status.created_at,
                    pending_count: status.pending_count,
                    published_count: status.published_count,
                    oldest_pending_at: status.oldest_pending_at,
                })
            })
            .collect()
    }
}

fn validate_topic(topic: &str) -> Result<(), CliCompatibilityError> {
    if topic.trim().is_empty() {
        return Err(CliCompatibilityError::InvalidRoute(
            "MQTT topic must not be empty".into(),
        ));
    }
    if topic.starts_with('/') || topic.ends_with('/') {
        return Err(CliCompatibilityError::InvalidRoute(
            "MQTT topic must not start or end with /".into(),
        ));
    }
    if topic.contains(['+', '#']) {
        return Err(CliCompatibilityError::InvalidRoute(
            "MQTT topic must not contain wildcards".into(),
        ));
    }
    if topic.chars().any(char::is_control) {
        return Err(CliCompatibilityError::InvalidRoute(
            "legacy MQTT route must not contain control characters".into(),
        ));
    }
    Ok(())
}

fn route_topic(config: &serde_json::Value) -> Result<String, CliCompatibilityError> {
    config
        .get("topic")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            StorageError::InvalidOutput("route configuration has no topic".into()).into()
        })
}

#[derive(Clone)]
pub struct LegacyMappings {
    storage: Storage,
}

impl LegacyMappings {
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    pub async fn put(
        &self,
        spec: LegacyMappingSpec,
        now: i64,
    ) -> Result<LegacyMapping, CliCompatibilityError> {
        validate_mapping_spec(&spec, now)?;
        let rule_spec = rule_spec(&spec);
        let existing = self.list().await?.into_iter().find(|mapping| {
            mapping.active
                && mapping.edge_node_id == spec.edge_node_id
                && mapping.series_key == spec.series_key
        });
        let rule = if let Some(mapping) = existing {
            let rule_id = rule_id_from_legacy_mapping(&mapping.mapping_id)?;
            self.storage
                .revise_cli_compat_semantic_rule(&rule_id, rule_spec, now)
                .await?
        } else {
            self.storage
                .create_cli_compat_semantic_rule(
                    SemanticRuleDraft {
                        edge_node_id: spec.edge_node_id.clone(),
                        series_key: spec.series_key.clone(),
                        display_name: "production_pulse".into(),
                        spec: rule_spec,
                    },
                    now,
                )
                .await?
        };
        Ok(LegacyMapping {
            mapping_id: legacy_mapping_id(&rule.rule_id)?,
            revision: rule.revision,
            edge_node_id: spec.edge_node_id,
            series_key: spec.series_key,
            meaning: spec.meaning,
            trigger_mode: spec.trigger_mode,
            active_value: spec.active_value,
            active: true,
            created_at: now,
        })
    }

    pub async fn deactivate(
        &self,
        edge_node_id: &str,
        series_key: &str,
        now: i64,
    ) -> Result<LegacyMapping, CliCompatibilityError> {
        let mapping = self
            .list()
            .await?
            .into_iter()
            .find(|mapping| {
                mapping.active
                    && mapping.edge_node_id == edge_node_id
                    && mapping.series_key == series_key
            })
            .ok_or(StorageError::SemanticNotFound)?;
        let rule_id = rule_id_from_legacy_mapping(&mapping.mapping_id)?;
        self.storage
            .retire_cli_compat_semantic_rule(&rule_id, now)
            .await?;
        Ok(LegacyMapping {
            active: false,
            ..mapping
        })
    }

    pub async fn list(&self) -> Result<Vec<LegacyMapping>, CliCompatibilityError> {
        let mut mappings = Vec::new();
        for row in self.storage.list_cli_semantic_revisions().await? {
            let Ok((trigger_mode, active_value)) = legacy_spec(row.spec) else {
                continue;
            };
            mappings.push(LegacyMapping {
                mapping_id: legacy_mapping_id(&row.rule_id)?,
                revision: row.revision,
                edge_node_id: row.edge_node_id,
                series_key: row.series_key,
                meaning: "production_pulse".into(),
                trigger_mode,
                active_value,
                active: row.active,
                created_at: row.created_at,
            });
        }
        Ok(mappings)
    }
}

fn validate_mapping_spec(spec: &LegacyMappingSpec, now: i64) -> Result<(), CliCompatibilityError> {
    if spec.edge_node_id.trim().is_empty() {
        return Err(CliCompatibilityError::InvalidMapping(
            "edge_node_id must not be empty".into(),
        ));
    }
    if spec.edge_node_id.contains(['/', '+', '#']) {
        return Err(CliCompatibilityError::InvalidMapping(
            "edge_node_id must not contain /, +, or #".into(),
        ));
    }
    if spec.series_key.trim().is_empty() {
        return Err(CliCompatibilityError::InvalidMapping(
            "series_key must not be empty".into(),
        ));
    }
    if spec.meaning != "production_pulse" {
        return Err(CliCompatibilityError::InvalidMapping(
            "unsupported semantic meaning".into(),
        ));
    }
    if !matches!(spec.active_value, 0 | 1) {
        return Err(CliCompatibilityError::InvalidMapping(
            "active_value must be 0 or 1".into(),
        ));
    }
    if now < 0 {
        return Err(CliCompatibilityError::InvalidMapping(
            "timestamp must be non-negative".into(),
        ));
    }
    Ok(())
}

fn rule_spec(spec: &LegacyMappingSpec) -> RuleSpec {
    RuleSpec {
        kind: SemanticKind::CumulativeCounter,
        detector: Detector {
            mode: if spec.active_value == 1 {
                DetectorMode::BooleanHighActive
            } else {
                DetectorMode::BooleanLowActive
            },
            ..Detector::default()
        },
        trigger: match spec.trigger_mode {
            LegacyTriggerMode::ActiveSample => TriggerMode::OnNotification,
            LegacyTriggerMode::ActiveEdge => TriggerMode::OnTransition,
        },
    }
}

fn legacy_spec(spec: RuleSpec) -> Result<(LegacyTriggerMode, i32), CliCompatibilityError> {
    if spec.kind != SemanticKind::CumulativeCounter {
        return Err(CliCompatibilityError::InvalidMapping(
            "semantic rule is not a production pulse mapping".into(),
        ));
    }
    let trigger_mode = match spec.trigger {
        TriggerMode::OnNotification => LegacyTriggerMode::ActiveSample,
        TriggerMode::OnTransition => LegacyTriggerMode::ActiveEdge,
        TriggerMode::None => {
            return Err(CliCompatibilityError::InvalidMapping(
                "semantic rule has an incompatible trigger".into(),
            ));
        }
    };
    let active_value = match spec.detector.mode {
        DetectorMode::BooleanHighActive => 1,
        DetectorMode::BooleanLowActive => 0,
        _ => {
            return Err(CliCompatibilityError::InvalidMapping(
                "semantic rule has an incompatible detector".into(),
            ));
        }
    };
    Ok((trigger_mode, active_value))
}

#[derive(Debug, Serialize)]
pub struct CliRawRecord {
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub pub_seq: i64,
    pub publication_id: String,
    pub record: Box<RawValue>,
    pub received_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacySemanticEvent {
    pub event_id: String,
    pub mapping_id: String,
    pub mapping_revision: i64,
    pub event_sequence: i64,
    pub meaning: String,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub source_pub_seq: i64,
    pub source_series_key: String,
    pub occurred_at: i64,
    pub created_at: i64,
}

#[derive(Clone)]
pub struct CliQueries {
    storage: Storage,
}

impl CliQueries {
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    pub async fn raw_records(&self, limit: usize) -> Result<Vec<CliRawRecord>, StorageError> {
        self.storage
            .list_cli_raw_records(limit)
            .await?
            .into_iter()
            .map(|row| {
                let record =
                    RawValue::from_string(String::from_utf8(row.record_json).map_err(|_| {
                        StorageError::InvalidRecord("stored JSON is not UTF-8".into())
                    })?)
                    .map_err(StorageError::EncodeRecord)?;
                Ok(CliRawRecord {
                    edge_node_id: row.edge_node_id,
                    ledger_epoch: row.ledger_epoch,
                    pub_seq: row.pub_seq,
                    publication_id: row.publication_id,
                    record,
                    received_at: row.received_at,
                })
            })
            .collect()
    }

    pub async fn semantic_events(
        &self,
        limit: usize,
    ) -> Result<Vec<LegacySemanticEvent>, CliCompatibilityError> {
        self.storage
            .list_cli_semantic_events(limit)
            .await?
            .into_iter()
            .map(|row| {
                Ok(LegacySemanticEvent {
                    event_id: row.event_id,
                    mapping_id: legacy_mapping_id(&row.rule_id)?,
                    mapping_revision: row.mapping_revision,
                    event_sequence: row.event_sequence,
                    meaning: "production_pulse".into(),
                    edge_node_id: row.edge_node_id,
                    ledger_epoch: row.ledger_epoch,
                    source_pub_seq: row.source_pub_seq,
                    source_series_key: row.source_series_key,
                    occurred_at: row.occurred_at,
                    created_at: row.created_at,
                })
            })
            .collect()
    }
}

pub fn legacy_mapping_id(rule_id: &str) -> Result<String, CliCompatibilityError> {
    let id = Uuid::parse_str(rule_id).map_err(|_| CliCompatibilityError::InvalidMappingId)?;
    Ok(format!("sm-{}", id.simple()))
}

pub fn rule_id_from_legacy_mapping(mapping_id: &str) -> Result<String, CliCompatibilityError> {
    let encoded = mapping_id
        .strip_prefix("sm-")
        .filter(|value| value.len() == 32 && value.bytes().all(is_lower_hex))
        .ok_or(CliCompatibilityError::InvalidMappingId)?;
    let id = Uuid::parse_str(encoded).map_err(|_| CliCompatibilityError::InvalidMappingId)?;
    Ok(id.hyphenated().to_string())
}

pub fn legacy_route_id(route_id: &str) -> Result<String, CliCompatibilityError> {
    let encoded = route_id
        .strip_prefix("route_")
        .ok_or(CliCompatibilityError::InvalidRouteId)?;
    let id = Uuid::parse_str(encoded).map_err(|_| CliCompatibilityError::InvalidRouteId)?;
    Ok(format!("mr-{}", id.simple()))
}

pub fn route_id_from_legacy_route(route_id: &str) -> Result<String, CliCompatibilityError> {
    let encoded = route_id
        .strip_prefix("mr-")
        .filter(|value| value.len() == 32 && value.bytes().all(is_lower_hex))
        .ok_or(CliCompatibilityError::InvalidRouteId)?;
    let id = Uuid::parse_str(encoded).map_err(|_| CliCompatibilityError::InvalidRouteId)?;
    Ok(format!("route_{}", id.hyphenated()))
}

const fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
}
