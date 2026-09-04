//! Delivery of accepted readings into the device-local pipelines.
//!
//! Runs inside the collector's accept transaction, so the pipeline state and
//! the outbox row commit together with the reading itself.

use std::collections::HashMap;

use iotkit_core_pipeline::{
    AcceptedReading, DeviceFaults, EngineError, InputTime, PipelineEngine, PipelineFaults,
};
use iotkit_ingest_contract::ReadingItem;
use tokio::sync::Notify;

use crate::principal::IngestPrincipal;

pub struct PipelineDelivery {
    engine: PipelineEngine,
    /// Configured source of an Input Adapter principal → instance name used
    /// by `PipelineInput::adapter`.
    adapters: HashMap<String, String>,
    faults: PipelineFaults,
    device_faults: DeviceFaults,
    committed: Notify,
}

impl PipelineDelivery {
    pub fn new(engine: PipelineEngine, faults: PipelineFaults) -> Self {
        Self {
            engine,
            adapters: HashMap::new(),
            faults,
            device_faults: DeviceFaults::default(),
            committed: Notify::new(),
        }
    }

    /// Shares the device fault record with the rest of the process; the
    /// collector reports storage failures and successes into it.
    pub fn with_device_faults(mut self, device_faults: DeviceFaults) -> Self {
        self.device_faults = device_faults;
        self
    }

    pub fn device_faults(&self) -> &DeviceFaults {
        &self.device_faults
    }

    /// Signalled after every committed accept transaction, so the MQTT Output
    /// Adapter can read the outbox without polling.
    pub fn committed(&self) -> &Notify {
        &self.committed
    }

    pub(crate) fn note_commit(&self) {
        self.device_faults.storage_write_succeeded();
        self.committed.notify_one();
    }

    pub(crate) fn note_storage_failure(&self, error: &str) {
        self.device_faults
            .storage_write_failed(error, iotkit_core_pipeline::uptime_ms());
    }

    /// Registers an Input Adapter instance. Readings from principals that are
    /// not registered here (for example authenticated devices) never reach a
    /// pipeline.
    pub fn register_adapter(
        &mut self,
        configured_source: impl Into<String>,
        instance_name: impl Into<String>,
    ) {
        self.adapters
            .insert(configured_source.into(), instance_name.into());
    }

    pub fn engine(&self) -> &PipelineEngine {
        &self.engine
    }

    pub fn faults(&self) -> &PipelineFaults {
        &self.faults
    }

    /// Delivers one durable item. Evaluator errors are recorded per pipeline
    /// and swallowed; a storage error is returned so the caller rolls back.
    pub(crate) fn deliver(
        &self,
        conn: &rusqlite::Connection,
        principal: &IngestPrincipal,
        item: &ReadingItem,
        received_at: InputTime,
    ) -> Result<(), String> {
        let Some(adapter) = self.adapters.get(principal.configured_source()) else {
            return Ok(());
        };
        let outcomes = self
            .engine
            .deliver(
                conn,
                &AcceptedReading {
                    adapter,
                    subject: item.subject_hint.as_deref(),
                    measurement_key: &item.measurement_key,
                    channel_index: item.channel_index,
                    values: &item.values,
                    received_at,
                },
            )
            .map_err(|error| error.to_string())?;
        for (pipeline_id, outcome) in outcomes {
            if let Err(error) = outcome {
                debug_assert!(
                    !matches!(error, EngineError::Sqlite(_) | EngineError::Store(_)),
                    "storage errors propagate from deliver"
                );
                tracing::warn!(pipeline = %pipeline_id, %error, "pipeline discarded an input");
                self.faults
                    .record(&pipeline_id, error.to_string(), received_at.uptime_ms);
            }
        }
        Ok(())
    }
}
