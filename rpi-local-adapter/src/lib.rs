//! rpi-local-adapter: RPi local I2C sensor adapter.
//! Thin wrapper over iotkit-polling-adapter-runtime with MCP9600 and OPT3001 drivers.

mod devices;
pub mod drivers;

pub use iotkit_polling_adapter_runtime::{AdapterHandle, PollingEvent};
pub use iotkit_sensor_drivers::mcp9600::ThermocoupleType;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use iotkit_core_types::AdapterId;
use iotkit_core_types::{DeviceKey, SensorReading};
use iotkit_ingest_contract::{ReadingItem, TimeSource};
use iotkit_input_adapter_host_api::{
    runtime_channels, AdapterCompletion, AdapterConfigScalar, AdapterDiagnostic,
    AdapterStartContext, CompletionReporter, DiagnosticKind, InputAdapterTypeDescriptor,
    PhysicalTransportKind, QueueSubmitError, RunningInputAdapter, UnexpectedExitReason,
};
use iotkit_polling_adapter_runtime::{PollingAdapterConfig, SensorTargetConfig};

/// Adapter configuration. Passed to [`start`].
#[derive(Debug, Clone)]
pub struct RpiLocalConfig {
    /// I2C bus path, e.g. "/dev/i2c-1".
    pub bus_path: String,
    /// Polling interval in milliseconds. Must be > 0.
    pub poll_interval_ms: u64,
    /// Sensor targets to probe and poll.
    pub targets: Vec<RpiLocalTarget>,
}

/// A single sensor target on the I2C bus.
#[derive(Debug, Clone)]
pub enum RpiLocalTarget {
    MCP9600 {
        address: u8,
        thermocouple_type: ThermocoupleType,
    },
    OPT3001 {
        address: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionalDeviceMetadata {
    pub locator: String,
    pub model_id: String,
    pub label: String,
}

pub fn built_in_targets() -> Vec<RpiLocalTarget> {
    devices::built_in_targets()
}

pub fn configured_target(
    model: &str,
    address: u8,
    settings: &BTreeMap<String, AdapterConfigScalar>,
) -> Result<RpiLocalTarget, String> {
    devices::configured_target(model, address, settings)
}

pub fn positional_inventory(config: &RpiLocalConfig) -> Vec<PositionalDeviceMetadata> {
    config
        .targets
        .iter()
        .map(|target| PositionalDeviceMetadata {
            locator: format!("i2c:0x{:02x}", target.address()),
            model_id: target.device_model_id().to_string(),
            label: target.inventory_label().to_string(),
        })
        .collect()
}

/// Start the rpi-local-adapter.
///
/// Validates config (including per-driver validation), then delegates to
/// `iotkit_polling_adapter_runtime::start`.
pub fn start(
    config: RpiLocalConfig,
    _legacy_ingest_removed: Option<()>,
) -> Result<AdapterHandle, std::io::Error> {
    let polling_config = to_polling_config(&config);
    iotkit_polling_adapter_runtime::start(AdapterId::new("rpi-local:default"), polling_config, None)
}

pub fn descriptor() -> InputAdapterTypeDescriptor {
    InputAdapterTypeDescriptor {
        adapter_type_id: iotkit_input_adapter_host_api::AdapterTypeId::new("rpi-local")
            .expect("static adapter type id"),
        adapter_api_major: 1,
        config_schema_version: 1,
        implementation_version: env!("CARGO_PKG_VERSION"),
        display_name: "Raspberry Pi local I2C",
        physical_transport_kind: PhysicalTransportKind::I2c,
    }
}

pub fn start_host(
    context: AdapterStartContext,
    config: RpiLocalConfig,
) -> Result<RunningInputAdapter, std::io::Error> {
    let instance_id = context.instance_id.clone();
    let handle = iotkit_polling_adapter_runtime::start(
        AdapterId::new(context.instance_id.as_str()),
        to_polling_config(&config),
        None,
    )?;
    let mapping_config = config;
    let parts = handle.into_parts();
    let (runtime, running) = runtime_channels(instance_id, 64);
    let iotkit_input_adapter_host_api::AdapterRuntimeEndpoint {
        activity,
        diagnostics,
        completion,
        mut stop,
    } = runtime;
    let mut event_rx = parts.event_rx;
    let shutdown = parts.shutdown;
    let composition_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                requested = stop.changed() => {
                    if requested {
                        break AdapterCompletion::RequestedStop;
                    }
                }
                event = event_rx.recv() => match event {
                    Some(PollingEvent::SensorData { device_key, reading, .. }) => {
                        activity.physical_decode();
                        match to_items(
                            &mapping_config,
                            context.subject_namespace.as_str(),
                            &device_key,
                            &reading,
                        ) {
                            Some(items) => match context.ingest.try_submit(items) {
                                Ok(_enqueued) => activity.queue_admission(),
                                Err(QueueSubmitError::Full(_)) => {
                                    let _ = diagnostics.try_emit(AdapterDiagnostic::new(
                                        DiagnosticKind::ClientQueueFull,
                                        "ingest queue is full",
                                    ));
                                }
                                Err(QueueSubmitError::Closed(_)) => {
                                    let _ = diagnostics.try_emit(AdapterDiagnostic::new(
                                        DiagnosticKind::ClientClosed,
                                        "ingest client is closed",
                                    ));
                                    break AdapterCompletion::UnexpectedExit(
                                        UnexpectedExitReason::ClientClosed,
                                    );
                                }
                            },
                            None => {
                                let _ = diagnostics.try_emit(AdapterDiagnostic::new(
                                    DiagnosticKind::MeasurementMapping,
                                    "observation has no declared measurement mapping",
                                ));
                            }
                        }
                    }
                    Some(PollingEvent::AdapterError { error, .. }) => {
                        let _ = diagnostics.try_emit(AdapterDiagnostic::new(
                            DiagnosticKind::DeviceUnavailable,
                            error,
                        ));
                    }
                    Some(PollingEvent::DeviceLost { reason, .. }) => {
                        let _ = diagnostics.try_emit(AdapterDiagnostic::new(
                            DiagnosticKind::DeviceUnavailable,
                            reason,
                        ));
                    }
                    Some(PollingEvent::DeviceDiscovered { .. }) => {}
                    None => break AdapterCompletion::UnexpectedExit(
                        UnexpectedExitReason::WorkerReturned,
                    ),
                }
            }
        }
    });
    tokio::spawn(async move {
        finalize_after_cleanup(composition_handle, shutdown.shutdown(), completion).await;
    });
    Ok(running)
}

async fn finalize_after_cleanup<F>(
    composition_handle: tokio::task::JoinHandle<AdapterCompletion>,
    cleanup: F,
    completion: CompletionReporter,
) where
    F: Future<Output = Result<(), String>>,
{
    let outcome = match composition_handle.await {
        Ok(outcome) => outcome,
        Err(_) => AdapterCompletion::Panic,
    };
    let outcome = if cleanup.await.is_err() {
        AdapterCompletion::Panic
    } else {
        outcome
    };
    completion.complete(outcome);
}

/// Map one polling observation to canonical ingest items.
pub fn to_items(
    config: &RpiLocalConfig,
    subject_namespace: &str,
    device_key: &DeviceKey,
    reading: &SensorReading,
) -> Option<Vec<ReadingItem>> {
    let target = config
        .targets
        .iter()
        .find(|target| target.matches_device_key(device_key.as_str()))?;
    let projection = target.project(reading).ok()?;
    let subject_hint = format!("{subject_namespace}:i2c:0x{:02x}", target.address());
    Some(vec![ReadingItem {
        subject_hint: Some(subject_hint),
        measurement_key: projection.measurement_key.into(),
        channel_index: projection.channel_index,
        series_variant: None,
        values: projection.values,
        device_time_ms: None,
        time_source: TimeSource::EdgeNode,
        age_ms: None,
        rssi: None,
        battery_pct: None,
    }])
}

/// Validate an `RpiLocalConfig` without starting the adapter.
///
/// Converts to `PollingAdapterConfig` internally and delegates to
/// `iotkit_polling_adapter_runtime::validate_config()`. Used for
/// preflight validation in the Edge Node before `start()`.
pub fn validate(config: &RpiLocalConfig) -> Result<(), String> {
    let polling_config = to_polling_config(config);
    iotkit_polling_adapter_runtime::validate_config(&polling_config)
}

fn to_polling_config(config: &RpiLocalConfig) -> PollingAdapterConfig {
    let device_factory: Arc<dyn rpi4b_transport::i2c::I2cDeviceFactory> =
        Arc::new(rpi4b_transport::i2c::LinuxI2cDeviceFactory);
    let targets = config
        .targets
        .iter()
        .map(|target| SensorTargetConfig {
            address: target.address(),
            driver: target.build_driver(Arc::clone(&device_factory)),
            key_suffix: Some(target.device_model_id().to_string()),
        })
        .collect();
    PollingAdapterConfig {
        bus_path: config.bus_path.clone(),
        poll_interval_ms: config.poll_interval_ms,
        targets,
    }
}

#[cfg(test)]
#[path = "../tests/unit/lib_tests.rs"]
mod tests;
