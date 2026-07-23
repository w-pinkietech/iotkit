use iotkit_output_adapter_api::ObservationValue;

use crate::{
    composition::OutputAdapterRegistration,
    semantics::{RuleSpec, SemanticKind},
    storage::{Storage, StorageError},
};

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
    pub series_id: String,
    pub revision: i64,
    pub active: bool,
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
        draft
            .spec
            .validate()
            .map_err(|error| StorageError::InvalidSemantic(error.to_string()))?;
        self.storage.create_semantic_rule(draft, now).await
    }

    pub async fn revise_rule(
        &self,
        rule_id: &str,
        display_name: &str,
        spec: RuleSpec,
        now: i64,
    ) -> Result<SemanticRule, StorageError> {
        spec.validate()
            .map_err(|error| StorageError::InvalidSemantic(error.to_string()))?;
        self.storage
            .revise_semantic_rule(rule_id, display_name, spec, now)
            .await
    }

    pub async fn update_calibration(
        &self,
        signal_ref: &str,
        scale: f64,
        offset: f64,
        now: i64,
    ) -> Result<i64, StorageError> {
        crate::semantics::Calibration { scale, offset }
            .validate()
            .map_err(|error| StorageError::InvalidSemantic(error.to_string()))?;
        self.storage
            .update_semantic_calibration(signal_ref, scale, offset, now)
            .await
    }

    pub async fn retire_rule(&self, rule_id: &str, now: i64) -> Result<(), StorageError> {
        self.storage.retire_semantic_rule(rule_id, now).await
    }

    pub async fn reset_counter(&self, rule_id: &str, now: i64) -> Result<String, StorageError> {
        self.storage.reset_semantic_counter(rule_id, now).await
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
