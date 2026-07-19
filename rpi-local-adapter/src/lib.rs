//! rpi-local-adapter: RPi local I2C sensor adapter.
//! Thin wrapper over iotkit-polling-adapter-runtime with MCP9600 and OPT3001 drivers.

pub mod drivers;

pub use iotkit_polling_adapter_runtime::{AdapterHandle, PollingEvent};
pub use iotkit_sensor_drivers::mcp9600::ThermocoupleType;

use std::future::Future;
use std::sync::Arc;

use iotkit_core_types::AdapterId;
use iotkit_core_types::{DeviceKey, SensorReading, SensorType};
use iotkit_ingest_contract::{ReadingItem, TimeSource};
use iotkit_input_adapter_host_api::{
    runtime_channels, AdapterCompletion, AdapterDiagnostic, AdapterStartContext,
    CompletionReporter, DiagnosticKind, InputAdapterTypeDescriptor, PhysicalTransportKind,
    QueueSubmitError, RunningInputAdapter, UnexpectedExitReason,
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
    subject_namespace: &str,
    device_key: &DeviceKey,
    reading: &SensorReading,
) -> Option<Vec<ReadingItem>> {
    let measurement_key = match reading.sensor_type {
        SensorType::Temperature => "temperature_c",
        SensorType::Illuminance => "illuminance_lux",
        _ => return None,
    };
    if reading.values.is_empty() {
        return None;
    }
    let parts: Vec<&str> = device_key.as_str().split(':').collect();
    let subject_hint = match parts.as_slice() {
        ["i2c", address, _suffix] => format!("{subject_namespace}:i2c:{address}"),
        _ => return None,
    };
    Some(vec![ReadingItem {
        subject_hint: Some(subject_hint),
        measurement_key: measurement_key.into(),
        channel_index: None,
        series_variant: None,
        values: reading.values.clone(),
        device_time_ms: None,
        time_source: TimeSource::Edge,
        age_ms: None,
        rssi: None,
        battery_pct: None,
    }])
}

/// Validate an `RpiLocalConfig` without starting the adapter.
///
/// Converts to `PollingAdapterConfig` internally and delegates to
/// `iotkit_polling_adapter_runtime::validate_config()`. Used for
/// preflight validation in the Edge before `start()`.
pub fn validate(config: &RpiLocalConfig) -> Result<(), String> {
    let polling_config = to_polling_config(config);
    iotkit_polling_adapter_runtime::validate_config(&polling_config)
}

fn to_polling_config(config: &RpiLocalConfig) -> PollingAdapterConfig {
    let targets = config
        .targets
        .iter()
        .map(|t| match t {
            RpiLocalTarget::MCP9600 {
                address,
                thermocouple_type,
            } => SensorTargetConfig {
                address: *address,
                driver: Arc::new(drivers::mcp9600::Mcp9600Driver {
                    thermocouple_type: *thermocouple_type,
                }),
                key_suffix: None,
            },
            RpiLocalTarget::OPT3001 { address } => SensorTargetConfig {
                address: *address,
                driver: Arc::new(drivers::opt3001::Opt3001Driver),
                key_suffix: None,
            },
        })
        .collect();
    PollingAdapterConfig {
        bus_path: config.bus_path.clone(),
        poll_interval_ms: config.poll_interval_ms,
        targets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tokio runtime が無い状態で start() を呼ぶと panic せず Err を返す。
    #[test]
    fn start_without_runtime_returns_error() {
        let config = RpiLocalConfig {
            bus_path: "/dev/i2c-1".to_string(),
            poll_interval_ms: 1000,
            targets: vec![RpiLocalTarget::MCP9600 {
                address: 0x60,
                thermocouple_type: ThermocoupleType::K,
            }],
        };
        let result = start(config, None);
        assert!(
            result.is_err(),
            "start() should return Err without tokio runtime"
        );
    }

    /// Config validation runs before runtime check, so this test verifies
    /// that invalid config produces a config-specific error message even
    /// without a tokio runtime.
    #[test]
    fn start_with_invalid_config_returns_config_error() {
        let config = RpiLocalConfig {
            bus_path: "/dev/i2c-1".to_string(),
            poll_interval_ms: 0,
            targets: vec![],
        };
        let err = start(config, None).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("poll_interval_ms"),
            "expected config validation error, got: {}",
            msg,
        );
    }

    #[test]
    fn validate_rejects_short_poll_interval_for_opt3001() {
        let cfg = RpiLocalConfig {
            bus_path: "/dev/i2c-1".into(),
            poll_interval_ms: 50,
            targets: vec![RpiLocalTarget::OPT3001 { address: 0x44 }],
        };
        let err = validate(&cfg).unwrap_err();
        assert!(err.contains("poll_interval_ms"), "unexpected error: {err}");
    }

    #[test]
    fn validate_accepts_valid_config() {
        let cfg = RpiLocalConfig {
            bus_path: "/dev/i2c-1".into(),
            poll_interval_ms: 1000,
            targets: vec![
                RpiLocalTarget::MCP9600 {
                    address: 0x60,
                    thermocouple_type: ThermocoupleType::K,
                },
                RpiLocalTarget::OPT3001 { address: 0x44 },
            ],
        };
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn mapping_preserves_legacy_positional_identity_and_canonical_units() {
        let items = to_items(
            "rpi-local:default",
            &DeviceKey::new("i2c:0x44:OPT3001"),
            &SensorReading::new(SensorType::Illuminance, vec![512.0], vec!["lux".into()]),
        )
        .unwrap();
        assert_eq!(
            items[0].subject_hint.as_deref(),
            Some("rpi-local:default:i2c:0x44")
        );
        assert_eq!(items[0].measurement_key, "illuminance_lux");
    }

    #[tokio::test]
    async fn panic_completion_waits_for_lower_runtime_cleanup() {
        let instance_id =
            iotkit_input_adapter_host_api::AdapterInstanceId::new("cleanup_test").unwrap();
        let (runtime, running) = runtime_channels(instance_id, 1);
        let completion = runtime.completion;
        let composition = tokio::spawn(async {
            panic!("test composition panic");
            #[allow(unreachable_code)]
            AdapterCompletion::RequestedStop
        });
        let (cleanup_tx, cleanup_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(finalize_after_cleanup(
            composition,
            async move {
                cleanup_rx
                    .await
                    .map_err(|_| "cleanup signal dropped".to_string())
            },
            completion,
        ));

        let completion_wait = running.completion.wait();
        tokio::pin!(completion_wait);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut completion_wait)
                .await
                .is_err(),
            "completion must remain pending until lower runtime cleanup finishes"
        );
        cleanup_tx.send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(1), &mut completion_wait)
                .await
                .expect("completion after cleanup"),
            AdapterCompletion::Panic
        );
    }

    /// OPT3001 driver rejects poll intervals shorter than 200ms.
    #[test]
    fn opt3001_rejects_short_poll_interval() {
        let config = RpiLocalConfig {
            bus_path: "/dev/i2c-1".to_string(),
            poll_interval_ms: 50,
            targets: vec![RpiLocalTarget::OPT3001 { address: 0x44 }],
        };
        let err = start(config, None).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("OPT3001"),
            "expected OPT3001 validation error, got: {}",
            msg,
        );
    }
}
