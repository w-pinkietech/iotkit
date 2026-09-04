//! Series, sequence, and single-transaction delivery.
//!
//! Every method takes the caller's connection inside an open transaction and
//! writes the evaluation state, the current value, the next sequence, and the
//! outbox row together. Nothing here commits.

use iotkit_core_types::{EdgeNodeId, PipelineId};
use rusqlite::Connection;

use crate::definition::{PipelineDefinition, PipelineKind, ValidationError};
use crate::evaluator::{self, EvaluationState, EvaluatorError};
use crate::outbox;
use crate::store::{self, PipelineState, StoreError};
use crate::wire::{InputTime, Observation, ObservationValue, observation_topic};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("pipeline {0} already exists")]
    AlreadyExists(String),
    #[error("pipeline {0} does not exist")]
    NotFound(String),
    #[error("pipeline kind cannot change; delete the pipeline and create it again")]
    KindChanged,
    #[error("input has no value at value_index {0}")]
    MissingValue(u16),
    #[error(transparent)]
    Evaluator(#[from] EvaluatorError),
}

/// A series that was started, with the observation published for it (only
/// `accumulated-count` publishes at series start).
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesStart {
    pub pipeline_id: PipelineId,
    pub series_id: String,
    pub published: Option<Observation>,
}

/// One accepted reading as seen by the collector: the adapter instance that
/// produced it, the device subject, and the measurement identity.
#[derive(Debug, Clone, Copy)]
pub struct AcceptedReading<'a> {
    pub adapter: &'a str,
    pub subject: Option<&'a str>,
    pub measurement_key: &'a str,
    pub channel_index: Option<u16>,
    pub values: &'a [f64],
    /// When the device received the input.
    pub received_at: InputTime,
}

/// The result of delivering one reading to one matching pipeline.
pub type PipelineDelivery = (PipelineId, Result<DeliveryOutcome, EngineError>);

#[derive(Debug, Clone, PartialEq)]
pub enum DeliveryOutcome {
    /// The input advanced the state without producing a publication.
    Silent,
    Published(Observation),
}

/// Pipeline core bound to one edge-node-id (the topic prefix of every
/// publication it enqueues).
#[derive(Debug, Clone)]
pub struct PipelineEngine {
    edge_node_id: EdgeNodeId,
}

impl PipelineEngine {
    pub fn new(edge_node_id: EdgeNodeId) -> Self {
        Self { edge_node_id }
    }

    pub fn edge_node_id(&self) -> &EdgeNodeId {
        &self.edge_node_id
    }

    /// Engine bound to the edge-node-id the node recorded at its last startup,
    /// for tools that run against the database without the TOML.
    pub fn load(conn: &Connection) -> Result<Option<Self>, StoreError> {
        Ok(store::get_edge_node_id(conn)?.map(Self::new))
    }

    /// Inserts a definition and starts its first series.
    pub fn create(
        &self,
        conn: &Connection,
        definition: &PipelineDefinition,
        now: InputTime,
    ) -> Result<SeriesStart, EngineError> {
        definition.validate()?;
        if store::get_definition(conn, &definition.id)?.is_some() {
            return Err(EngineError::AlreadyExists(definition.id.to_string()));
        }
        store::insert_definition(conn, definition, now.uptime_ms)?;
        self.start_series(conn, definition, now)
    }

    /// Replaces a definition. A structural change starts a new series; a
    /// tuning change keeps the series and the evaluation state.
    pub fn update(
        &self,
        conn: &Connection,
        definition: &PipelineDefinition,
        now: InputTime,
    ) -> Result<Option<SeriesStart>, EngineError> {
        definition.validate()?;
        let existing = store::get_definition(conn, &definition.id)?
            .ok_or_else(|| EngineError::NotFound(definition.id.to_string()))?;
        if existing.kind != definition.kind {
            return Err(EngineError::KindChanged);
        }
        store::update_definition(conn, definition, now.uptime_ms)?;
        let state = store::get_state(conn, &definition.id)?;
        match state {
            Some(state) if state.structural_hash == definition.structural_hash() => Ok(None),
            _ => self.start_series(conn, definition, now).map(Some),
        }
    }

    /// Deletes a definition and enqueues the retained-value clear for its topic.
    pub fn delete(
        &self,
        conn: &Connection,
        id: &PipelineId,
        now: InputTime,
    ) -> Result<(), EngineError> {
        let definition = store::get_definition(conn, id)?
            .ok_or_else(|| EngineError::NotFound(id.to_string()))?;
        store::delete_definition(conn, id)?;
        self.enqueue_deletion(conn, &definition, now)?;
        Ok(())
    }

    /// Explicit reset: starts a new series and clears the evaluation state.
    pub fn reset(
        &self,
        conn: &Connection,
        id: &PipelineId,
        now: InputTime,
    ) -> Result<SeriesStart, EngineError> {
        let definition = store::get_definition(conn, id)?
            .ok_or_else(|| EngineError::NotFound(id.to_string()))?;
        self.start_series(conn, &definition, now)
    }

    /// Replaces every definition. Pipelines that disappear get their retained
    /// value cleared; every imported pipeline starts a new series.
    pub fn import(
        &self,
        conn: &Connection,
        definitions: &[PipelineDefinition],
        now: InputTime,
    ) -> Result<Vec<SeriesStart>, EngineError> {
        for definition in definitions {
            definition.validate()?;
        }
        let mut seen = std::collections::BTreeSet::new();
        for definition in definitions {
            if !seen.insert(definition.id.clone()) {
                return Err(ValidationError::new(format!(
                    "duplicate pipeline id {}",
                    definition.id
                ))
                .into());
            }
        }
        for existing in store::list_definitions(conn)? {
            store::delete_definition(conn, &existing.id)?;
            if !seen.contains(&existing.id) {
                self.enqueue_deletion(conn, &existing, now)?;
            }
        }
        let mut started = Vec::with_capacity(definitions.len());
        for definition in definitions {
            store::insert_definition(conn, definition, now.uptime_ms)?;
            started.push(self.start_series(conn, definition, now)?);
        }
        Ok(started)
    }

    /// Startup reconciliation: a pipeline without state, or whose state was
    /// started under a different structural hash, starts a new series.
    pub fn reconcile(
        &self,
        conn: &Connection,
        now: InputTime,
    ) -> Result<Vec<SeriesStart>, EngineError> {
        store::put_edge_node_id(conn, &self.edge_node_id)?;
        let mut started = Vec::new();
        for definition in store::list_definitions(conn)? {
            let continues = store::get_state(conn, &definition.id)?
                .is_some_and(|state| state.structural_hash == definition.structural_hash());
            if !continues {
                started.push(self.start_series(conn, &definition, now)?);
            }
        }
        Ok(started)
    }

    /// Delivers one accepted reading to every pipeline whose input matches.
    /// Returns one outcome per matched pipeline, in pipeline-id order.
    ///
    /// An evaluator error (for example the counter limit) is returned for that
    /// pipeline only; its state is left unchanged and the caller records the
    /// discarded input. A SQLite error propagates so the caller rolls back.
    pub fn deliver(
        &self,
        conn: &Connection,
        reading: &AcceptedReading<'_>,
    ) -> Result<Vec<PipelineDelivery>, EngineError> {
        let definitions = store::definitions_for_input(
            conn,
            reading.adapter,
            reading.subject,
            reading.measurement_key,
            reading.channel_index,
        )?;
        let mut outcomes = Vec::with_capacity(definitions.len());
        for definition in definitions {
            let outcome = match reading
                .values
                .get(usize::from(definition.input.value_index))
            {
                None => Err(EngineError::MissingValue(definition.input.value_index)),
                Some(value) => self.process(conn, &definition, *value, reading.received_at),
            };
            let outcome = match outcome {
                Err(error @ (EngineError::Sqlite(_) | EngineError::Store(_))) => return Err(error),
                other => other,
            };
            outcomes.push((definition.id.clone(), outcome));
        }
        Ok(outcomes)
    }

    /// Evaluates one input for one pipeline and persists the result.
    pub fn process(
        &self,
        conn: &Connection,
        definition: &PipelineDefinition,
        input: f64,
        received_at: InputTime,
    ) -> Result<DeliveryOutcome, EngineError> {
        let mut state = match store::get_state(conn, &definition.id)? {
            Some(state) if state.structural_hash == definition.structural_hash() => state,
            _ => {
                // No usable series yet (state lost or definition edited without
                // reconciliation): start one and continue with this input.
                self.start_series(conn, definition, received_at)?;
                store::get_state(conn, &definition.id)?
                    .ok_or_else(|| EngineError::NotFound(definition.id.to_string()))?
            }
        };
        // Debounce runs on the monotonic clock so a wall-clock correction
        // cannot lengthen or shorten a window.
        let (evaluation, next_evaluation) =
            evaluator::evaluate(definition, state.evaluation, input, received_at.uptime_ms)?;
        state.evaluation = next_evaluation;

        let value = if !evaluation.emitted {
            None
        } else {
            match definition.kind {
                PipelineKind::Measurement => {
                    let number = evaluation.number.expect("measurement emits a number");
                    let unchanged = matches!(
                        state.last_value,
                        Some(ObservationValue::Measurement(last)) if last == number
                    );
                    (!unchanged).then_some(ObservationValue::Measurement(number))
                }
                PipelineKind::State => evaluation.boolean.map(ObservationValue::State),
                PipelineKind::AccumulatedCount => {
                    evaluation.integer.map(ObservationValue::AccumulatedCount)
                }
            }
        };

        let outcome = match value {
            None => DeliveryOutcome::Silent,
            Some(value) => {
                let observation = self.publish(conn, definition, &mut state, value, received_at)?;
                DeliveryOutcome::Published(observation)
            }
        };
        store::put_state(conn, &definition.id, &state, received_at.uptime_ms)?;
        Ok(outcome)
    }

    fn start_series(
        &self,
        conn: &Connection,
        definition: &PipelineDefinition,
        now: InputTime,
    ) -> Result<SeriesStart, EngineError> {
        let mut state = PipelineState {
            structural_hash: definition.structural_hash(),
            series_id: uuid::Uuid::now_v7().to_string(),
            next_sequence: 1,
            evaluation: EvaluationState::default(),
            last_value: None,
            last_published_at: None,
        };
        let published = if definition.kind == PipelineKind::AccumulatedCount {
            Some(self.publish(
                conn,
                definition,
                &mut state,
                ObservationValue::AccumulatedCount(0),
                now,
            )?)
        } else {
            None
        };
        store::put_state(conn, &definition.id, &state, now.uptime_ms)?;
        Ok(SeriesStart {
            pipeline_id: definition.id.clone(),
            series_id: state.series_id,
            published,
        })
    }

    /// Builds the Observation, enqueues its wire form, and advances the state.
    fn publish(
        &self,
        conn: &Connection,
        definition: &PipelineDefinition,
        state: &mut PipelineState,
        value: ObservationValue,
        at: InputTime,
    ) -> Result<Observation, EngineError> {
        let observation = Observation {
            pipeline_id: definition.id.clone(),
            series_id: state.series_id.clone(),
            sequence: state.next_sequence,
            at,
            value,
        };
        outbox::enqueue(
            conn,
            &definition.id,
            &observation.topic(&self.edge_node_id),
            &observation.payload(),
            true,
            at,
        )?;
        state.next_sequence += 1;
        state.last_value = Some(value);
        state.last_published_at = Some(at);
        Ok(observation)
    }

    fn enqueue_deletion(
        &self,
        conn: &Connection,
        definition: &PipelineDefinition,
        now: InputTime,
    ) -> Result<(), EngineError> {
        let topic = observation_topic(&self.edge_node_id, &definition.id, definition.kind);
        outbox::enqueue(conn, &definition.id, &topic, &[], true, now)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/engine_tests.rs"]
mod tests;
