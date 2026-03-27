//! async task: raw bytes → codec → AdapterEvent。
//! デバイスのライフサイクル追跡もここで行う。

use std::collections::HashMap;

use bravepi_codec::{BravePiCodec, DownlinkCommand};
use iotkit_core_types::{AdapterCommand, AdapterEvent, DeviceCommandPayload, DeviceKey, SensorType};
use tokio::sync::mpsc;

use crate::registry::lookup_handler;
use crate::transport::{BytesReceiver, BytesSender};
use super::convert::frame_to_event;

struct DeviceTarget {
    device_number_hex: String,
    raw_sensor_type: u16,
}

struct DeviceState {
    #[allow(dead_code)]
    last_seen: tokio::time::Instant,
    target: DeviceTarget,
}

pub(crate) async fn event_loop(
    port_path: String,
    mut bytes_rx: BytesReceiver,
    event_tx: mpsc::Sender<AdapterEvent>,
    mut command_rx: mpsc::Receiver<AdapterCommand>,
    write_tx: BytesSender,
) {
    tracing::info!(port = %port_path, "BravePI adapter event loop started");

    let mut codec = BravePiCodec::new();
    // デバイスのライフサイクル追跡。adapter task 終了時に解放される。
    // BravePI は物理的に固定台数のため、実運用で数十台規模に収まる。
    let mut devices: HashMap<DeviceKey, DeviceState> = HashMap::new();

    loop {
        tokio::select! {
            // Shutdown コマンドを優先処理する
            biased;

            cmd = command_rx.recv() => {
                match cmd {
                    Some(AdapterCommand::Shutdown) | None => {
                        tracing::info!("BravePI adapter shutting down");
                        return;
                    }
                    Some(AdapterCommand::DeviceCommand(cmd)) => {
                        handle_device_command(cmd, &devices, &write_tx, &event_tx).await;
                    }
                }
            }
            result = bytes_rx.recv() => {
                match result {
                    Some(Ok(data)) => {
                        codec.feed(&data);
                        while let Some(frame) = codec.decode() {
                            // Extract target info from Sensor frames before frame_to_event consumes the frame
                            let target_info = match &frame {
                                bravepi_codec::BravePiFrame::Sensor(s) => {
                                    Some((s.device_number.clone(), s.sensor_type_raw))
                                }
                                _ => None,
                            };

                            if let Some((event, identity)) = frame_to_event(frame, &port_path) {
                                if let AdapterEvent::SensorData { ref device_key, .. } = event {
                                    if !devices.contains_key(device_key) {
                                        match identity {
                                            Some(identity) => {
                                                let discovered = AdapterEvent::DeviceDiscovered {
                                                    device_key: device_key.clone(),
                                                    identity,
                                                };
                                                if event_tx.send(discovered).await.is_err() {
                                                    tracing::warn!("Event channel closed, shutting down");
                                                    return;
                                                }
                                                let target = target_info.map(|(dn, rst)| DeviceTarget {
                                                    device_number_hex: dn,
                                                    raw_sensor_type: rst,
                                                }).expect("SensorData always comes from Sensor frame");
                                                devices.insert(
                                                    device_key.clone(),
                                                    DeviceState {
                                                        last_seen: tokio::time::Instant::now(),
                                                        target,
                                                    },
                                                );
                                            }
                                            None => {
                                                tracing::warn!(
                                                    device_key = %device_key,
                                                    "New device without identity, skipping"
                                                );
                                                continue;
                                            }
                                        }
                                    } else {
                                        devices.get_mut(device_key).unwrap().last_seen =
                                            tokio::time::Instant::now();
                                    }
                                }

                                if event_tx.send(event).await.is_err() {
                                    tracing::warn!("Event channel closed, shutting down");
                                    return;
                                }
                            }
                        }
                    }
                    Some(Err(error)) => {
                        tracing::error!(%error, "Serial reader reported error");
                        let _ = event_tx.send(AdapterEvent::AdapterError {
                            device_key: None,
                            error: error.to_string(),
                        }).await;
                        return;
                    }
                    None => {
                        tracing::warn!("Serial reader thread exited without error report");
                        let _ = event_tx.send(AdapterEvent::AdapterError {
                            device_key: None,
                            error: format!("Serial reader thread for {} exited unexpectedly", port_path),
                        }).await;
                        return;
                    }
                }
            }
        }
    }
}

async fn handle_device_command(
    cmd: iotkit_core_types::DeviceCommand,
    devices: &HashMap<DeviceKey, DeviceState>,
    write_tx: &BytesSender,
    event_tx: &mpsc::Sender<AdapterEvent>,
) {
    let state = match devices.get(&cmd.device_key) {
        Some(s) => s,
        None => {
            let _ = event_tx.send(AdapterEvent::AdapterError {
                device_key: Some(cmd.device_key),
                error: "unknown device".to_string(),
            }).await;
            return;
        }
    };

    let target = &state.target;

    // Validate SetOutput constraints
    if let DeviceCommandPayload::SetOutput { duration_ms, .. } = &cmd.payload {
        if let Some(handler) = lookup_handler(target.raw_sensor_type) {
            if handler.sensor_type != SensorType::ContactOutput {
                let _ = event_tx.send(AdapterEvent::AdapterError {
                    device_key: Some(cmd.device_key),
                    error: "SetOutput sent to non-ContactOutput device".to_string(),
                }).await;
                return;
            }
        } else {
            let _ = event_tx.send(AdapterEvent::AdapterError {
                device_key: Some(cmd.device_key),
                error: "SetOutput: unknown sensor type in registry".to_string(),
            }).await;
            return;
        }

        if let Some(ms) = duration_ms {
            if *ms > u16::MAX as u32 {
                let _ = event_tx.send(AdapterEvent::AdapterError {
                    device_key: Some(cmd.device_key),
                    error: format!("duration_ms {} exceeds u16 range (max {})", ms, u16::MAX),
                }).await;
                return;
            }
        }
    }

    let downlink_cmd = match cmd.payload {
        DeviceCommandPayload::RequestReading => {
            DownlinkCommand::ImmediateUplink { sensor_type: target.raw_sensor_type }
        }
        DeviceCommandPayload::QueryConfig => {
            DownlinkCommand::ParameterGet
        }
        DeviceCommandPayload::SetOutput { value, duration_ms } => {
            DownlinkCommand::ContactOutput {
                signal_mode: if value { 1 } else { 0 },
                signal_out_time: duration_ms.map(|ms| ms as u16).unwrap_or(0),
            }
        }
    };

    let bytes = match BravePiCodec::encode_downlink(&target.device_number_hex, &downlink_cmd) {
        Ok(b) => b,
        Err(e) => {
            let _ = event_tx.send(AdapterEvent::AdapterError {
                device_key: Some(cmd.device_key),
                error: format!("encode_downlink failed: {}", e),
            }).await;
            return;
        }
    };

    match write_tx.try_send(bytes) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            let _ = event_tx.send(AdapterEvent::AdapterError {
                device_key: Some(cmd.device_key),
                error: "downlink queue full".to_string(),
            }).await;
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            let _ = event_tx.send(AdapterEvent::AdapterError {
                device_key: None,
                error: "write channel closed (transport failure)".to_string(),
            }).await;
        }
    }
}
