//! Pipeline definition: the structural items that identify a series and the
//! tuning items that may change without starting a new series.

use iotkit_core_types::PipelineId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::calibration::Calibration;

pub const MAX_DEBOUNCE_MS: i64 = 300_000;
pub const MAX_DISPLAY_NAME_CHARS: usize = 128;
pub const MAX_UNIT_CHARS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PipelineKind {
    Measurement,
    State,
    AccumulatedCount,
}

impl PipelineKind {
    /// The `{kind-key}` topic segment of the MQTT Output Adapter v1 contract.
    pub fn key(self) -> &'static str {
        match self {
            Self::Measurement => "measurement",
            Self::State => "state",
            Self::AccumulatedCount => "accumulated-count",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Trigger {
    OnTransition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DetectorMode {
    HighActive,
    LowActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detector {
    pub mode: DetectorMode,
    pub rise_threshold: f64,
    pub fall_threshold: f64,
    #[serde(default)]
    pub rise_debounce_ms: i64,
    #[serde(default)]
    pub fall_debounce_ms: i64,
}

impl Detector {
    pub fn validate(self) -> Result<(), ValidationError> {
        if !self.rise_threshold.is_finite() || !self.fall_threshold.is_finite() {
            return Err(ValidationError::new(
                "detector.rise_threshold and detector.fall_threshold must be finite",
            ));
        }
        if !(0..=MAX_DEBOUNCE_MS).contains(&self.rise_debounce_ms)
            || !(0..=MAX_DEBOUNCE_MS).contains(&self.fall_debounce_ms)
        {
            return Err(ValidationError::new(
                "detector debounce must be between 0 and 300000 ms",
            ));
        }
        if self.fall_threshold > self.rise_threshold {
            return Err(ValidationError::new(
                "detector.fall_threshold cannot exceed detector.rise_threshold",
            ));
        }
        Ok(())
    }
}

/// Which Input Adapter output feeds the pipeline. All fields are structural.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineInput {
    /// Input Adapter instance name from `[adapters.instances.<name>]`.
    pub adapter: String,
    /// Device subject reported by the adapter; `None` matches any subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub measurement_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_index: Option<u16>,
    /// Index into the reading's `values`.
    #[serde(default)]
    pub value_index: u16,
}

impl PipelineInput {
    pub fn matches(
        &self,
        adapter: &str,
        subject: Option<&str>,
        measurement_key: &str,
        channel_index: Option<u16>,
    ) -> bool {
        self.adapter == adapter
            && self.measurement_key == measurement_key
            && self.channel_index == channel_index
            && self
                .subject
                .as_deref()
                .is_none_or(|want| Some(want) == subject)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineDefinition {
    pub id: PipelineId,
    pub kind: PipelineKind,
    pub input: PipelineInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<Trigger>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub calibration: Calibration,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detector: Option<Detector>,
}

/// The structural subset whose normalized hash decides series continuity.
#[derive(Serialize)]
struct StructuralFields<'a> {
    id: &'a PipelineId,
    kind: PipelineKind,
    input: &'a PipelineInput,
    trigger: Option<Trigger>,
    unit: Option<&'a str>,
}

impl PipelineDefinition {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.input.adapter.is_empty() {
            return Err(ValidationError::new("input.adapter must not be empty"));
        }
        if self.input.measurement_key.is_empty() {
            return Err(ValidationError::new(
                "input.measurement_key must not be empty",
            ));
        }
        if self.input.subject.as_deref() == Some("") {
            return Err(ValidationError::new("input.subject must not be empty"));
        }
        if let Some(name) = &self.display_name
            && (name.trim().is_empty() || name.chars().count() > MAX_DISPLAY_NAME_CHARS)
        {
            return Err(ValidationError::new(
                "display_name must be 1 to 128 characters",
            ));
        }
        self.calibration
            .validate()
            .map_err(|error| ValidationError::new(error.to_string()))?;

        match self.kind {
            PipelineKind::Measurement => {
                if self.detector.is_some() {
                    return Err(ValidationError::new(
                        "detector is not allowed for kind measurement",
                    ));
                }
                if self.trigger.is_some() {
                    return Err(ValidationError::new(
                        "trigger is not allowed for kind measurement",
                    ));
                }
                match &self.unit {
                    Some(unit)
                        if !unit.trim().is_empty() && unit.chars().count() <= MAX_UNIT_CHARS => {}
                    _ => {
                        return Err(ValidationError::new(
                            "unit is required for kind measurement (1 to 32 characters)",
                        ));
                    }
                }
            }
            PipelineKind::State | PipelineKind::AccumulatedCount => {
                if self.unit.is_some() {
                    return Err(ValidationError::new(
                        "unit is only allowed for kind measurement",
                    ));
                }
                let Some(detector) = self.detector else {
                    return Err(ValidationError::new(format!(
                        "detector is required for kind {}",
                        self.kind.key()
                    )));
                };
                detector.validate()?;
                match (self.kind, self.trigger) {
                    (PipelineKind::State, None) | (PipelineKind::AccumulatedCount, Some(_)) => {}
                    (PipelineKind::State, Some(_)) => {
                        return Err(ValidationError::new(
                            "trigger is not allowed for kind state",
                        ));
                    }
                    (PipelineKind::AccumulatedCount, None) => {
                        return Err(ValidationError::new(
                            "trigger is required for kind accumulated-count",
                        ));
                    }
                    (PipelineKind::Measurement, _) => unreachable!("matched above"),
                }
            }
        }
        Ok(())
    }

    /// Hex SHA-256 of the canonical JSON of the structural fields. Two
    /// definitions with the same hash continue the same series.
    pub fn structural_hash(&self) -> String {
        let fields = StructuralFields {
            id: &self.id,
            kind: self.kind,
            input: &self.input,
            trigger: self.trigger,
            unit: self.unit.as_deref(),
        };
        // serde_json::Value orders object keys, so the encoding is canonical.
        let canonical = serde_json::to_value(fields).expect("structural fields serialize");
        let bytes = serde_json::to_vec(&canonical).expect("canonical value serializes");
        let digest = Sha256::digest(bytes);
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid pipeline definition: {0}")]
pub struct ValidationError(String);

impl ValidationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

#[cfg(test)]
#[path = "../tests/unit/definition_tests.rs"]
pub(crate) mod tests;
