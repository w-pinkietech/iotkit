use iotkit_output_adapter_api::ObservationValue;

use crate::{
    composition::OutputAdapterRegistration,
    semantics::{
        Calibration, DefinitionSpec, Evaluation, PreviewInput, RuleSpec, SemanticKind,
        build_preview,
    },
    storage::{AuditActor, Storage, StorageError},
};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct SemanticRuleDraft {
    pub edge_node_id: String,
    pub series_key: String,
    pub display_name: String,
    pub spec: RuleSpec,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticRule {
    pub rule_id: String,
    pub signal_ref: String,
    pub edge_node_id: String,
    pub series_key: String,
    pub display_name: String,
    pub kind: SemanticKind,
    pub spec: RuleSpec,
    pub series_id: String,
    pub revision: i64,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SemanticCalibration {
    pub calibration: Calibration,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticObservation {
    pub observation_id: String,
    pub rule_id: String,
    pub series_id: String,
    pub sequence: u64,
    pub observed_at: i64,
    pub value: ObservationValue,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectionProgress {
    pub receipts: usize,
    pub observations: usize,
    pub publications: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingPreviewRequest {
    pub signal_ref: String,
    pub calibration: Calibration,
    pub rules: Vec<SemanticPreviewRule>,
    pub test_value: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticPreviewRule {
    pub rule_id: String,
    pub display_name: String,
    pub spec: RuleSpec,
}

#[derive(Debug, Clone)]
pub struct MappingPreviewResponse {
    pub calibration: Calibration,
    pub rules: Vec<SemanticRulePreview>,
    pub window_start: Option<i64>,
    pub window_end: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct SemanticRulePreview {
    pub rule_id: String,
    pub display_name: String,
    pub kind: SemanticKind,
    pub input_count: usize,
    pub plot_count: usize,
    pub points: Vec<crate::semantics::PreviewPoint>,
    pub test_result: Option<Evaluation>,
    pub error: String,
}

#[derive(Clone)]
pub struct Semantics {
    storage: Storage,
}

impl Semantics {
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    pub async fn create_rule(
        &self,
        draft: SemanticRuleDraft,
        now: i64,
    ) -> Result<SemanticRule, StorageError> {
        self.create_rule_as(AuditActor::local_cli(), draft, now)
            .await
    }

    pub async fn create_rule_as(
        &self,
        actor: AuditActor,
        draft: SemanticRuleDraft,
        now: i64,
    ) -> Result<SemanticRule, StorageError> {
        draft
            .spec
            .validate()
            .map_err(|error| StorageError::InvalidSemantic(error.to_string()))?;
        self.storage
            .create_semantic_rule_as(actor, draft, now)
            .await
    }

    pub async fn preview(
        &self,
        request: MappingPreviewRequest,
    ) -> Result<MappingPreviewResponse, StorageError> {
        request
            .calibration
            .validate()
            .map_err(|error| StorageError::InvalidSemantic(error.to_string()))?;
        if request.signal_ref.is_empty() || request.rules.is_empty() || request.rules.len() > 16 {
            return Err(StorageError::InvalidSemantic(
                "signal and between 1 and 16 preview rules are required".into(),
            ));
        }
        let stored = self
            .storage
            .recent_signal_inputs(&request.signal_ref, 2_000)
            .await?;
        let mut inputs = Vec::with_capacity(stored.len());
        for item in stored {
            let value: serde_json::Value = serde_json::from_slice(&item.record_json)
                .map_err(|error| StorageError::InvalidSemantic(error.to_string()))?;
            let Some(number) = value
                .get("values")
                .and_then(serde_json::Value::as_array)
                .filter(|values| values.len() == 1)
                .and_then(|values| values[0].as_f64())
                .filter(|value| value.is_finite())
            else {
                continue;
            };
            inputs.push(PreviewInput {
                received_at: item.received_at,
                observed_at: value.get("event_time").and_then(serde_json::Value::as_i64),
                value: number,
            });
        }
        let window_start = inputs.first().map(|input| input.received_at);
        let window_end = inputs.last().map(|input| input.received_at);
        let mut rules = Vec::with_capacity(request.rules.len());
        for draft in request.rules {
            let definition = DefinitionSpec {
                kind: draft.spec.kind,
                scale: request.calibration.scale,
                offset: request.calibration.offset,
                detector: draft.spec.detector,
                trigger: draft.spec.trigger,
            };
            let result = build_preview(definition, &inputs, 200, request.test_value);
            rules.push(match result {
                Ok(preview) => SemanticRulePreview {
                    rule_id: draft.rule_id,
                    display_name: draft.display_name,
                    kind: draft.spec.kind,
                    input_count: preview.input_count,
                    plot_count: preview.plot_count,
                    points: preview.points,
                    test_result: preview.test_result,
                    error: String::new(),
                },
                Err(error) => SemanticRulePreview {
                    rule_id: draft.rule_id,
                    display_name: draft.display_name,
                    kind: draft.spec.kind,
                    input_count: inputs.len(),
                    plot_count: 0,
                    points: Vec::new(),
                    test_result: None,
                    error: error.to_string(),
                },
            });
        }
        Ok(MappingPreviewResponse {
            calibration: request.calibration,
            rules,
            window_start,
            window_end,
        })
    }

    pub async fn revise_rule(
        &self,
        rule_id: &str,
        display_name: &str,
        spec: RuleSpec,
        now: i64,
    ) -> Result<SemanticRule, StorageError> {
        self.revise_rule_as(AuditActor::local_cli(), rule_id, display_name, spec, now)
            .await
    }

    pub async fn revise_rule_as(
        &self,
        actor: AuditActor,
        rule_id: &str,
        display_name: &str,
        spec: RuleSpec,
        now: i64,
    ) -> Result<SemanticRule, StorageError> {
        spec.validate()
            .map_err(|error| StorageError::InvalidSemantic(error.to_string()))?;
        self.storage
            .revise_semantic_rule_as(actor, rule_id, display_name, spec, now)
            .await
    }

    pub async fn update_calibration(
        &self,
        signal_ref: &str,
        scale: f64,
        offset: f64,
        now: i64,
    ) -> Result<i64, StorageError> {
        self.update_calibration_as(
            AuditActor::local_cli(),
            signal_ref,
            scale,
            offset,
            None,
            now,
        )
        .await
    }

    pub async fn update_calibration_as(
        &self,
        actor: AuditActor,
        signal_ref: &str,
        scale: f64,
        offset: f64,
        expected_revision: Option<i64>,
        now: i64,
    ) -> Result<i64, StorageError> {
        crate::semantics::Calibration { scale, offset }
            .validate()
            .map_err(|error| StorageError::InvalidSemantic(error.to_string()))?;
        self.storage
            .update_semantic_calibration_as(
                actor,
                signal_ref,
                scale,
                offset,
                expected_revision,
                now,
            )
            .await
    }

    pub async fn retire_rule(&self, rule_id: &str, now: i64) -> Result<(), StorageError> {
        self.retire_rule_as(AuditActor::local_cli(), rule_id, now)
            .await
    }

    pub async fn retire_rule_as(
        &self,
        actor: AuditActor,
        rule_id: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        self.storage
            .retire_semantic_rule_as(actor, rule_id, now)
            .await
    }

    pub async fn reset_counter(&self, rule_id: &str, now: i64) -> Result<String, StorageError> {
        self.reset_counter_as(AuditActor::local_cli(), rule_id, now)
            .await
    }

    pub async fn reset_counter_as(
        &self,
        actor: AuditActor,
        rule_id: &str,
        now: i64,
    ) -> Result<String, StorageError> {
        self.storage
            .reset_semantic_counter_as(actor, rule_id, now)
            .await
    }

    pub async fn project_pending(
        &self,
        limit: usize,
        adapters: &'static [OutputAdapterRegistration],
    ) -> Result<ProjectionProgress, StorageError> {
        if !(1..=10_000).contains(&limit) {
            return Err(StorageError::InvalidSemantic(
                "projection limit must be between 1 and 10000".into(),
            ));
        }
        let mut progress = ProjectionProgress::default();
        for _ in 0..limit {
            let Some(item) = self.storage.project_one_semantic(adapters).await? else {
                break;
            };
            progress.receipts += usize::from(item.receipt);
            progress.observations += usize::from(item.observation);
            progress.publications += item.publications;
        }
        Ok(progress)
    }
}
