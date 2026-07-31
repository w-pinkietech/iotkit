//! Local-only changing sample source for the public trial profile.
//!
//! Emits two series on the standard Input Adapter / custody path:
//! - continuous triangle wave → `illuminance_lux`
//! - boolean square wave → `contact_state` (High/Low demo for on-off style state)
//!
//! Compiled into Edge Node, but only runs when explicitly selected and gated by
//! [`ENABLE_ENV`].

use std::time::Duration;

use iotkit_ingest_contract::{ReadingItem, TimeSource};
use iotkit_input_adapter_host_api::{
    AdapterCompletion, AdapterStartContext, DiagnosticKind, InputAdapterTypeDescriptor,
    PhysicalTransportKind, QueueSubmitError, RunningInputAdapter, UnexpectedExitReason,
    runtime_channels,
};

/// Inventory model id for the continuous trial series (non-hardware).
pub const ILLUMINANCE_MODEL_ID: &str = "trial-sample-illuminance";
/// Inventory model id for the state trial series (non-hardware).
pub const CONTACT_MODEL_ID: &str = "trial-sample-contact";
pub const ILLUMINANCE_LABEL: &str = "Trial illuminance sensor";
pub const CONTACT_LABEL: &str = "Trial contact state";
/// Edge Node enables this adapter only when the trial launcher sets this flag.
pub const ENABLE_ENV: &str = "IOTKIT_ENABLE_TRIAL_SAMPLE";
/// Polls per half-cycle of the contact square wave.
///
/// Live samples start at sequence 1: Low (`0.0`) for `1..=half`, then High (`1.0`)
/// for `half+1..=2*half`, and so on.
pub const DEFAULT_STATE_HALF_PERIOD_POLLS: u64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrialSampleConfig {
    pub poll_interval_ms: u64,
}

/// One positional inventory subject exposed by this adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryItem {
    pub hardware_id: String,
    pub model_id: String,
    pub label: String,
}

pub fn validate(config: TrialSampleConfig) -> Result<(), String> {
    if !(250..=60_000).contains(&config.poll_interval_ms) {
        return Err("poll_interval_ms must be between 250 and 60000".into());
    }
    Ok(())
}

pub fn descriptor() -> InputAdapterTypeDescriptor {
    InputAdapterTypeDescriptor {
        adapter_type_id: iotkit_input_adapter_host_api::AdapterTypeId::new("trial-sample")
            .expect("static adapter type id"),
        adapter_api_major: 1,
        config_schema_version: 1,
        implementation_version: env!("CARGO_PKG_VERSION"),
        display_name: "Trial sample",
        physical_transport_kind: PhysicalTransportKind::Other,
    }
}

/// Continuous triangle wave for the illuminance demo series.
pub fn illuminance_reading(subject_namespace: &str, sequence: u64) -> ReadingItem {
    let wave = (sequence % 20) as f64;
    let value = if wave <= 10.0 {
        120.0 + wave * 8.0
    } else {
        120.0 + (20.0 - wave) * 8.0
    };
    ReadingItem {
        subject_hint: Some(format!("{subject_namespace}:sample")),
        measurement_key: "illuminance_lux".into(),
        channel_index: None,
        series_variant: None,
        values: vec![value],
        device_time_ms: None,
        time_source: TimeSource::EdgeNode,
        age_ms: None,
        rssi: None,
        battery_pct: None,
    }
}

/// Boolean square wave for the contact / running-state demo series.
///
/// Values are `0.0` (Low) or `1.0` (High) to match catalog `contact_state` (bool).
pub fn contact_reading(subject_namespace: &str, sequence: u64) -> ReadingItem {
    let half = DEFAULT_STATE_HALF_PERIOD_POLLS.max(1);
    // sequence 1..=half → Low, half+1..=2*half → High, then repeats.
    let phase = (sequence.saturating_sub(1) / half) % 2;
    let value = if phase == 0 { 0.0 } else { 1.0 };
    ReadingItem {
        subject_hint: Some(format!("{subject_namespace}:state")),
        measurement_key: "contact_state".into(),
        channel_index: None,
        series_variant: None,
        values: vec![value],
        device_time_ms: None,
        time_source: TimeSource::EdgeNode,
        age_ms: None,
        rssi: None,
        battery_pct: None,
    }
}

/// Both series for one poll tick (default path used by the host loop).
pub fn readings(subject_namespace: &str, sequence: u64) -> Vec<ReadingItem> {
    vec![
        illuminance_reading(subject_namespace, sequence),
        contact_reading(subject_namespace, sequence),
    ]
}

/// Positional inventory for both trial virtual subjects.
pub fn inventory_items(source: &str) -> Vec<InventoryItem> {
    vec![
        InventoryItem {
            hardware_id: format!("{source}:sample"),
            model_id: ILLUMINANCE_MODEL_ID.into(),
            label: ILLUMINANCE_LABEL.into(),
        },
        InventoryItem {
            hardware_id: format!("{source}:state"),
            model_id: CONTACT_MODEL_ID.into(),
            label: CONTACT_LABEL.into(),
        },
    ]
}

pub fn start_host(
    context: AdapterStartContext,
    config: TrialSampleConfig,
) -> Result<RunningInputAdapter, String> {
    validate(config)?;
    let (runtime, running) = runtime_channels(context.instance_id.clone(), 16);
    let iotkit_input_adapter_host_api::AdapterRuntimeEndpoint {
        activity,
        diagnostics,
        completion,
        mut stop,
    } = runtime;
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(config.poll_interval_ms));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut sequence = 0_u64;
        let outcome = loop {
            tokio::select! {
                requested = stop.changed() => {
                    if requested {
                        break AdapterCompletion::RequestedStop;
                    }
                }
                _ = ticker.tick() => {
                    sequence = sequence.wrapping_add(1);
                    activity.physical_decode();
                    match context.ingest.try_submit(readings(
                        context.subject_namespace.as_str(),
                        sequence,
                    )) {
                        Ok(_) => activity.queue_admission(),
                        Err(QueueSubmitError::Full(_)) => {
                            let _ = diagnostics.try_emit(
                                iotkit_input_adapter_host_api::AdapterDiagnostic::new(
                                    DiagnosticKind::ClientQueueFull,
                                    "ingest queue is full",
                                ),
                            );
                        }
                        Err(QueueSubmitError::Closed(_)) => {
                            break AdapterCompletion::UnexpectedExit(
                                UnexpectedExitReason::ClientClosed,
                            );
                        }
                    }
                }
            }
        };
        completion.complete(outcome);
    });
    Ok(running)
}

#[cfg(test)]
#[path = "../tests/unit/lib_tests.rs"]
mod tests;
