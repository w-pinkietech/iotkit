//! iotkit-core-pipeline: device-local processing pipelines.
//!
//! A pipeline converts one Input Adapter output into one Observation through
//! calibration, thresholding with hysteresis, debounce, and accumulated
//! counting. This crate owns the pipeline definitions, the evaluation state,
//! series and sequence, the Observation wire form of the MQTT Output Adapter
//! v1 contract, and the outbox rows that the MQTT Output Adapter drains.
//!
//! Everything that mutates state takes a `rusqlite::Transaction` so that the
//! collector can write the evaluation state, the accumulated value, the next
//! sequence, and the outbox row in one commit.

use iotkit_core_storage::Migration;

pub mod calibration;
pub mod definition;
pub mod engine;
pub mod evaluator;
pub mod export;
pub mod faults;
pub mod outbox;
pub mod store;
pub mod wire;

pub use calibration::Calibration;
pub use definition::{
    Detector, DetectorMode, PipelineDefinition, PipelineInput, PipelineKind, Trigger,
    ValidationError,
};
pub use engine::{
    AcceptedReading, DeliveryOutcome, EngineError, PipelineDelivery, PipelineEngine, SeriesStart,
};
pub use evaluator::{Evaluation, EvaluationState, EvaluatorError};
pub use export::{
    DEFAULT_EXPORT_FILE_NAME, ExportError, ImportError, default_export_path, export_definitions,
    read_definitions,
};
pub use faults::{FaultRecord, PipelineFaults};
pub use outbox::OutboxRow;
pub use store::{PipelineState, StoreError};
pub use wire::{Observation, ObservationValue};

pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 25,
    label: "pipeline",
    sql: include_str!("../migrations/0025_pipeline.sql"),
}];
