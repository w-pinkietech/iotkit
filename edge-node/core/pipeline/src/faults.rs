//! In-memory record of inputs a pipeline discarded. Not persisted: the
//! Console shows it and the operator clears it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use iotkit_core_types::PipelineId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultRecord {
    pub discarded: u64,
    pub last_error: String,
    /// `uptime_ms` of the last discarded input.
    pub last_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct PipelineFaults {
    inner: Arc<Mutex<BTreeMap<PipelineId, FaultRecord>>>,
}

impl PipelineFaults {
    pub fn record(&self, pipeline_id: &PipelineId, error: impl Into<String>, now: i64) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = inner.entry(pipeline_id.clone()).or_insert(FaultRecord {
            discarded: 0,
            last_error: String::new(),
            last_at: now,
        });
        entry.discarded += 1;
        entry.last_error = error.into();
        entry.last_at = now;
    }

    pub fn snapshot(&self) -> BTreeMap<PipelineId, FaultRecord> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn clear(&self, pipeline_id: &PipelineId) -> Option<FaultRecord> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(pipeline_id)
    }
}
