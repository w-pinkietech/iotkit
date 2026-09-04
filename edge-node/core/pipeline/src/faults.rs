//! In-memory fault records. Neither is persisted.
//!
//! [`PipelineFaults`] counts the inputs one pipeline discarded; the Console
//! shows it and the operator clears it. [`DeviceFaults`] holds the device
//! faults that the status topic publishes as `faults` (contract section 7.1);
//! a fault is present while it lasts and disappears when it recovers.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use iotkit_core_types::PipelineId;

use crate::status::{Fault, FaultKind, InterfaceOpenReason};
use crate::wire::InputTime;

/// Longest `detail` text kept for a fault; the rest is truncated.
pub const MAX_FAULT_DETAIL_BYTES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageWriteFault {
    pub since_uptime_ms: i64,
    /// Inputs discarded since the fault started.
    pub count: u64,
    pub last_error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceOpenFault {
    pub since_uptime_ms: i64,
    pub reason: InterfaceOpenReason,
    pub detail: Option<String>,
}

/// Point-in-time copy of the device faults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceFaultSnapshot {
    pub storage: Option<StorageWriteFault>,
    /// Keyed by Input Adapter instance name.
    pub interfaces: BTreeMap<String, InterfaceOpenFault>,
}

impl DeviceFaultSnapshot {
    /// `degraded` is exactly "new input is being discarded".
    pub fn degraded(&self) -> bool {
        self.storage.is_some()
    }

    /// The contract's `faults` list, in a stable order. `since_unix_epoch_ms`
    /// is derived from `since_uptime_ms` through `now`: within one boot the
    /// two clocks keep a constant offset, so the wall-clock start is known
    /// exactly when the wall clock is trusted now and unknown otherwise.
    pub fn faults(&self, now: InputTime) -> Vec<Fault> {
        let since = |since_uptime_ms: i64| InputTime {
            uptime_ms: since_uptime_ms,
            unix_epoch_ms: now
                .unix_epoch_ms
                .map(|wall| wall - (now.uptime_ms - since_uptime_ms)),
        };
        let mut faults = Vec::with_capacity(self.interfaces.len() + 1);
        if let Some(storage) = &self.storage {
            faults.push(Fault {
                kind: FaultKind::StorageWriteFailed {
                    count: storage.count,
                },
                since: since(storage.since_uptime_ms),
                detail: Some(storage.last_error.clone()),
            });
        }
        for (adapter, fault) in &self.interfaces {
            faults.push(Fault {
                kind: FaultKind::InterfaceOpenFailed {
                    adapter: adapter.clone(),
                    reason: fault.reason,
                },
                since: since(fault.since_uptime_ms),
                detail: fault.detail.clone(),
            });
        }
        faults
    }
}

type ChangeListener = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct DeviceFaultState {
    snapshot: DeviceFaultSnapshot,
    listener: Option<ChangeListener>,
}

/// Shared between the collector (storage failures), the adapter supervisor
/// (interface open failures), and the MQTT Output Adapter (publishes them).
#[derive(Clone, Default)]
pub struct DeviceFaults {
    inner: Arc<Mutex<DeviceFaultState>>,
}

impl DeviceFaults {
    /// Registers the one callback invoked after every change to the fault
    /// set, outside the lock. The MQTT Output Adapter uses it to publish the
    /// status without waiting for the heartbeat interval.
    pub fn set_listener(&self, listener: impl Fn() + Send + Sync + 'static) {
        self.lock().listener = Some(Arc::new(listener));
    }

    pub fn snapshot(&self) -> DeviceFaultSnapshot {
        self.lock().snapshot.clone()
    }

    /// One input transaction failed. Starts the fault or counts one more
    /// discarded input; the status changes only when the fault starts.
    pub fn storage_write_failed(&self, error: impl Into<String>, uptime_ms: i64) {
        let error = truncate_detail(error.into());
        let listener = {
            let mut state = self.lock();
            match &mut state.snapshot.storage {
                Some(fault) => {
                    fault.count += 1;
                    fault.last_error = error;
                    None
                }
                None => {
                    state.snapshot.storage = Some(StorageWriteFault {
                        since_uptime_ms: uptime_ms,
                        count: 1,
                        last_error: error,
                    });
                    state.listener.clone()
                }
            }
        };
        notify(listener);
    }

    /// One input transaction committed: the storage fault, if any, recovers.
    pub fn storage_write_succeeded(&self) {
        let listener = {
            let mut state = self.lock();
            state
                .snapshot
                .storage
                .take()
                .and_then(|_| state.listener.clone())
        };
        notify(listener);
    }

    /// An Input Adapter could not open its interface. A repeated failure of
    /// the same adapter keeps the original start time.
    pub fn interface_open_failed(
        &self,
        adapter: impl Into<String>,
        reason: InterfaceOpenReason,
        detail: Option<String>,
        uptime_ms: i64,
    ) {
        let adapter = adapter.into();
        let detail = detail.map(truncate_detail);
        let listener = {
            let mut state = self.lock();
            match state.snapshot.interfaces.get_mut(&adapter) {
                Some(fault) if fault.reason == reason && fault.detail == detail => None,
                Some(fault) => {
                    fault.reason = reason;
                    fault.detail = detail;
                    state.listener.clone()
                }
                None => {
                    state.snapshot.interfaces.insert(
                        adapter,
                        InterfaceOpenFault {
                            since_uptime_ms: uptime_ms,
                            reason,
                            detail,
                        },
                    );
                    state.listener.clone()
                }
            }
        };
        notify(listener);
    }

    /// The adapter opened its interface: its fault, if any, recovers.
    pub fn interface_opened(&self, adapter: &str) {
        let listener = {
            let mut state = self.lock();
            state
                .snapshot
                .interfaces
                .remove(adapter)
                .and_then(|_| state.listener.clone())
        };
        notify(listener);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, DeviceFaultState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn notify(listener: Option<ChangeListener>) {
    if let Some(listener) = listener {
        listener();
    }
}

fn truncate_detail(mut text: String) -> String {
    if text.len() > MAX_FAULT_DETAIL_BYTES {
        let mut end = MAX_FAULT_DETAIL_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

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

#[cfg(test)]
#[path = "../tests/unit/faults_tests.rs"]
mod tests;
