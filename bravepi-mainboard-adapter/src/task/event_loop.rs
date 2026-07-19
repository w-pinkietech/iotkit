//! Supervision- and ingest-free BravePI decoded runtime.

use std::collections::HashMap;

use bravepi_codec::{BravePiCodec, DownlinkCommand};
use iotkit_core_types::{DeviceKey, SensorIdentity, SensorReading, SensorType};
use tokio::sync::mpsc;

use super::convert::{DecodedEvent, frame_to_event};
use crate::registry::lookup_handler;
use crate::transport::{BytesReceiver, BytesSender};

#[derive(Debug)]
pub(crate) struct DecodedObservation {
    pub device_key: DeviceKey,
    pub reading: SensorReading,
    pub rssi: Option<i16>,
    pub battery_pct: Option<u8>,
    pub observed_at: std::time::SystemTime,
}

#[derive(Debug)]
pub(crate) struct RuntimeDeviceConfig {
    pub firmware_version: String,
    pub uplink_interval_secs: u32,
    pub timezone: i64,
    pub ble_mode: i64,
    pub tx_power: i64,
    pub advertise_interval: i64,
}

#[derive(Debug)]
pub(crate) enum RuntimeEvent {
    DeviceDiscovered {
        device_key: DeviceKey,
        identity: SensorIdentity,
    },
    Observation(DecodedObservation),
    DeviceConfig {
        device_key: DeviceKey,
        config: RuntimeDeviceConfig,
    },
    Error {
        device_key: Option<DeviceKey>,
        error: String,
    },
}

#[derive(Debug)]
pub(crate) struct RuntimeDeviceCommand {
    pub device_key: DeviceKey,
    pub payload: RuntimeDeviceCommandPayload,
}

#[derive(Debug)]
pub(crate) enum RuntimeDeviceCommandPayload {
    RequestReading,
    QueryConfig,
    SetOutput {
        value: bool,
        duration_ms: Option<u32>,
    },
}

#[derive(Debug)]
pub(crate) enum RuntimeCommand {
    Shutdown,
    DeviceCommand(RuntimeDeviceCommand),
}

struct DeviceTarget {
    device_number_hex: String,
    raw_sensor_type: u16,
}

struct DeviceState {
    #[allow(dead_code)]
    last_seen: tokio::time::Instant,
    target: DeviceTarget,
}

pub(crate) async fn decoded_event_loop(
    port_path: String,
    mut bytes_rx: BytesReceiver,
    event_tx: mpsc::Sender<RuntimeEvent>,
    mut command_rx: mpsc::Receiver<RuntimeCommand>,
    write_tx: BytesSender,
) {
    tracing::info!(port = %port_path, "BravePI decoded runtime started");

    let mut codec = BravePiCodec::new();
    let mut devices: HashMap<DeviceKey, DeviceState> = HashMap::new();

    loop {
        tokio::select! {
            command = command_rx.recv() => {
                match command {
                    Some(RuntimeCommand::Shutdown) | None => {
                        tracing::info!("BravePI decoded runtime shutting down");
                        return;
                    }
                    Some(RuntimeCommand::DeviceCommand(command)) => {
                        if handle_device_command(command, &devices, &write_tx, &event_tx).await {
                            return;
                        }
                    }
                }
            }
            result = bytes_rx.recv() => {
                match result {
                    Some(Ok(data)) => {
                        codec.feed(&data);
                        while let Some(frame) = codec.decode() {
                            if let bravepi_codec::BravePiFrame::Config(ref config) = frame {
                                if handle_config_frame(config, &devices, &event_tx).await {
                                    return;
                                }
                                continue;
                            }

                            let target_info = match &frame {
                                bravepi_codec::BravePiFrame::Sensor(sensor) => {
                                    Some((sensor.device_number.clone(), sensor.sensor_type_raw))
                                }
                                _ => None,
                            };

                            let Some((decoded, identity)) = frame_to_event(frame, &port_path) else {
                                continue;
                            };
                            match decoded {
                                DecodedEvent::SensorData {
                                    device_key,
                                    reading,
                                    rssi,
                                    battery_pct,
                                    observed_at,
                                } => {
                                    if !devices.contains_key(&device_key) {
                                        let Some(identity) = identity else {
                                            tracing::warn!(%device_key, "new device has no identity");
                                            continue;
                                        };
                                        if event_tx
                                            .send(RuntimeEvent::DeviceDiscovered {
                                                device_key: device_key.clone(),
                                                identity,
                                            })
                                            .await
                                            .is_err()
                                        {
                                            return;
                                        }
                                        let (device_number_hex, raw_sensor_type) = target_info
                                            .expect("decoded sensor observation has target info");
                                        devices.insert(
                                            device_key.clone(),
                                            DeviceState {
                                                last_seen: tokio::time::Instant::now(),
                                                target: DeviceTarget {
                                                    device_number_hex,
                                                    raw_sensor_type,
                                                },
                                            },
                                        );
                                    } else if let Some(device) = devices.get_mut(&device_key) {
                                        device.last_seen = tokio::time::Instant::now();
                                    }
                                    if event_tx
                                        .send(RuntimeEvent::Observation(DecodedObservation {
                                            device_key,
                                            reading,
                                            rssi,
                                            battery_pct,
                                            observed_at,
                                        }))
                                        .await
                                        .is_err()
                                    {
                                        return;
                                    }
                                }
                                DecodedEvent::AdapterError { device_key, error } => {
                                    if event_tx
                                        .send(RuntimeEvent::Error { device_key, error })
                                        .await
                                        .is_err()
                                    {
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    Some(Err(error)) => {
                        let _ = event_tx
                            .send(RuntimeEvent::Error {
                                device_key: None,
                                error: error.to_string(),
                            })
                            .await;
                        return;
                    }
                    None => {
                        let _ = event_tx
                            .send(RuntimeEvent::Error {
                                device_key: None,
                                error: format!(
                                    "Serial reader thread for {port_path} exited unexpectedly"
                                ),
                            })
                            .await;
                        return;
                    }
                }
            }
        }
    }
}

async fn handle_config_frame(
    config: &bravepi_codec::ConfigFrame,
    devices: &HashMap<DeviceKey, DeviceState>,
    event_tx: &mpsc::Sender<RuntimeEvent>,
) -> bool {
    let Some(handler) = lookup_handler(config.true_sensor_type) else {
        tracing::warn!(
            raw = config.true_sensor_type,
            device = %config.device_number,
            "ConfigFrame with unknown sensor type, dropping"
        );
        return false;
    };
    let device_key = DeviceKey::new(format!(
        "bravepi-mainboard:{}:{}",
        config.device_number, handler.key_suffix
    ));
    if !devices.contains_key(&device_key) {
        tracing::warn!(%device_key, "ConfigFrame for undiscovered device, dropping");
        return false;
    }
    event_tx
        .send(RuntimeEvent::DeviceConfig {
            device_key,
            config: RuntimeDeviceConfig {
                firmware_version: config.firmware_version.clone(),
                uplink_interval_secs: config.uplink_interval,
                timezone: i64::from(config.timezone),
                ble_mode: i64::from(config.ble_mode),
                tx_power: i64::from(config.tx_power),
                advertise_interval: i64::from(config.advertise_interval),
            },
        })
        .await
        .is_err()
}

async fn emit_error(
    event_tx: &mpsc::Sender<RuntimeEvent>,
    device_key: Option<DeviceKey>,
    error: impl Into<String>,
) -> bool {
    event_tx
        .send(RuntimeEvent::Error {
            device_key,
            error: error.into(),
        })
        .await
        .is_err()
}

async fn handle_device_command(
    command: RuntimeDeviceCommand,
    devices: &HashMap<DeviceKey, DeviceState>,
    write_tx: &BytesSender,
    event_tx: &mpsc::Sender<RuntimeEvent>,
) -> bool {
    let Some(state) = devices.get(&command.device_key) else {
        return emit_error(event_tx, Some(command.device_key), "unknown device").await;
    };
    let target = &state.target;

    if let RuntimeDeviceCommandPayload::SetOutput { duration_ms, .. } = &command.payload {
        if lookup_handler(target.raw_sensor_type)
            .is_none_or(|handler| handler.sensor_type != SensorType::ContactOutput)
        {
            return emit_error(
                event_tx,
                Some(command.device_key),
                "SetOutput sent to non-ContactOutput device",
            )
            .await;
        }
        if duration_ms.is_some_and(|duration| duration > u32::from(u16::MAX)) {
            return emit_error(
                event_tx,
                Some(command.device_key),
                format!("duration_ms exceeds u16 range (max {})", u16::MAX),
            )
            .await;
        }
    }

    let downlink = match command.payload {
        RuntimeDeviceCommandPayload::RequestReading => DownlinkCommand::ImmediateUplink {
            sensor_type: target.raw_sensor_type,
        },
        RuntimeDeviceCommandPayload::QueryConfig => DownlinkCommand::ParameterGet,
        RuntimeDeviceCommandPayload::SetOutput { value, duration_ms } => {
            DownlinkCommand::ContactOutput {
                signal_mode: u8::from(value),
                signal_out_time: duration_ms.map(|value| value as u16).unwrap_or(0),
            }
        }
    };
    let bytes = match BravePiCodec::encode_downlink(&target.device_number_hex, &downlink) {
        Ok(bytes) => bytes,
        Err(error) => {
            return emit_error(
                event_tx,
                Some(command.device_key),
                format!("encode_downlink failed: {error}"),
            )
            .await;
        }
    };
    match write_tx.try_send(bytes) {
        Ok(()) => false,
        Err(mpsc::error::TrySendError::Full(_)) => {
            emit_error(event_tx, Some(command.device_key), "downlink queue full").await
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            emit_error(event_tx, None, "write channel closed (transport failure)").await
        }
    }
}
