//! Local-only changing sample source for the public trial profile.
//!
//! The adapter uses the same host API and custody path as hardware adapters. It is intentionally
//! compiled into Edge Node, but only runs when explicitly selected in Edge Node configuration.

use std::time::Duration;

use iotkit_ingest_contract::{ReadingItem, TimeSource};
use iotkit_input_adapter_host_api::{
    AdapterCompletion, AdapterStartContext, DiagnosticKind, InputAdapterTypeDescriptor,
    PhysicalTransportKind, QueueSubmitError, RunningInputAdapter, UnexpectedExitReason,
    runtime_channels,
};

/// Inventory model id is intentionally non-hardware so Console listings cannot
/// be confused with a physical OPT3001.
pub const MODEL_ID: &str = "trial-sample-illuminance";
pub const INVENTORY_LABEL: &str = "Trial illuminance sensor";
/// Edge Node enables this adapter only when the trial launcher sets this flag.
pub const ENABLE_ENV: &str = "IOTKIT_ENABLE_TRIAL_SAMPLE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrialSampleConfig {
    pub poll_interval_ms: u64,
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

pub fn reading(subject_namespace: &str, sequence: u64) -> ReadingItem {
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
                    match context.ingest.try_submit(vec![
                        reading(context.subject_namespace.as_str(), sequence)
                    ]) {
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
