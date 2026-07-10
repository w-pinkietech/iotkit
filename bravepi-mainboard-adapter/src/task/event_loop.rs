//! async task: raw bytes → codec → AdapterEvent。
//! デバイスのライフサイクル追跡もここで行う。

use std::collections::BTreeMap;
use std::collections::HashMap;

use bravepi_codec::{BravePiCodec, DownlinkCommand};
use iotkit_core_supervision::{
    AdapterCommand, AdapterEvent, ConfigValue, DeviceCommand, DeviceCommandPayload,
    DeviceConfigData,
};
use iotkit_core_types::{DeviceKey, SensorType};
use tokio::sync::mpsc;

use super::convert::frame_to_event;
use crate::registry::lookup_handler;
use crate::transport::{BytesReceiver, BytesSender};

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
    adapter_id: String,
    port_path: String,
    mut bytes_rx: BytesReceiver,
    event_tx: mpsc::Sender<AdapterEvent>,
    mut command_rx: mpsc::Receiver<AdapterCommand>,
    write_tx: BytesSender,
    ingest: Option<iotkit_ingest_client::IngestClient>,
) {
    tracing::info!(port = %port_path, "BravePI adapter event loop started");

    let mut codec = BravePiCodec::new();
    // デバイスのライフサイクル追跡。adapter task 終了時に解放される。
    // BravePI は物理的に固定台数のため、実運用で数十台規模に収まる。
    let mut devices: HashMap<DeviceKey, DeviceState> = HashMap::new();

    loop {
        tokio::select! {
            cmd = command_rx.recv() => {
                match cmd {
                    Some(AdapterCommand::Shutdown) | None => {
                        tracing::info!("BravePI adapter shutting down");
                        return;
                    }
                    Some(AdapterCommand::DeviceCommand(cmd)) => {
                        if handle_device_command(cmd, &devices, &write_tx, &event_tx).await {
                            tracing::warn!("Event channel closed during device command handling");
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
                            // Handle ConfigFrame directly (needs devices map)
                            if let bravepi_codec::BravePiFrame::Config(ref cfg) = frame {
                                if handle_config_frame(cfg, &devices, &event_tx).await {
                                    tracing::warn!("Event channel closed during config frame handling");
                                    return;
                                }
                                continue;
                            }

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

                                if let Some(client) = &ingest
                                    && let AdapterEvent::SensorData {
                                        device_key, reading, rssi, battery_pct, ..
                                    } = &event
                                {
                                    match super::ingest_map::to_items(device_key, reading, *rssi, *battery_pct) {
                                        Some(items) => {
                                            for chunk in items.chunks(super::ingest_map::MAX_ITEMS_PER_ENVELOPE) {
                                                let envelope = iotkit_ingest_client::new_envelope(
                                                    adapter_id.as_str(),
                                                    chunk.to_vec(),
                                                );
                                                if let Err(e) = client.try_submit(envelope) {
                                                    match e {
                                                        iotkit_ingest_client::IngestClientError::Full => {
                                                            tracing::warn!("ingest queue full; dropping reading");
                                                        }
                                                        iotkit_ingest_client::IngestClientError::Closed => {
                                                            tracing::warn!("ingest client closed; dropping reading");
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        None => tracing::warn!(
                                            device_key = %device_key,
                                            "no measurement mapping; reading not ingested"
                                        ),
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
                        let _closed = event_tx.send(AdapterEvent::AdapterError {
                            device_key: None,
                            error: error.to_string(),
                        }).await.is_err();
                        return;
                    }
                    None => {
                        tracing::warn!("Serial reader thread exited without error report");
                        let _closed = event_tx.send(AdapterEvent::AdapterError {
                            device_key: None,
                            error: format!("Serial reader thread for {} exited unexpectedly", port_path),
                        }).await.is_err();
                        return;
                    }
                }
            }
        }
    }
}

/// Returns true if the event channel is closed.
async fn handle_config_frame(
    cfg: &bravepi_codec::ConfigFrame,
    devices: &HashMap<DeviceKey, DeviceState>,
    event_tx: &mpsc::Sender<AdapterEvent>,
) -> bool {
    let handler = match lookup_handler(cfg.true_sensor_type) {
        Some(h) => h,
        None => {
            tracing::warn!(
                raw = cfg.true_sensor_type,
                device = %cfg.device_number,
                "ConfigFrame with unknown sensor type, dropping"
            );
            return false;
        }
    };

    let device_key = DeviceKey::new(format!(
        "bravepi-mainboard:{}:{}",
        cfg.device_number, handler.key_suffix
    ));

    if !devices.contains_key(&device_key) {
        tracing::warn!(
            device_key = %device_key,
            "ConfigFrame for undiscovered device, dropping"
        );
        return false;
    }

    let config = DeviceConfigData {
        firmware_version: Some(cfg.firmware_version.clone()),
        uplink_interval_secs: Some(cfg.uplink_interval),
        properties: BTreeMap::from([
            ("timezone".into(), ConfigValue::Integer(cfg.timezone as i64)),
            ("ble_mode".into(), ConfigValue::Integer(cfg.ble_mode as i64)),
            ("tx_power".into(), ConfigValue::Integer(cfg.tx_power as i64)),
            (
                "advertise_interval".into(),
                ConfigValue::Integer(cfg.advertise_interval as i64),
            ),
        ]),
    };

    tracing::info!(
        device = %cfg.device_number,
        firmware = %cfg.firmware_version,
        "Config frame received, sending DeviceConfig event"
    );

    event_tx
        .send(AdapterEvent::DeviceConfig { device_key, config })
        .await
        .is_err()
}

/// Returns true if the event channel is closed.
async fn handle_device_command(
    cmd: DeviceCommand,
    devices: &HashMap<DeviceKey, DeviceState>,
    write_tx: &BytesSender,
    event_tx: &mpsc::Sender<AdapterEvent>,
) -> bool {
    let state = match devices.get(&cmd.device_key) {
        Some(s) => s,
        None => {
            return event_tx
                .send(AdapterEvent::AdapterError {
                    device_key: Some(cmd.device_key),
                    error: "unknown device".to_string(),
                })
                .await
                .is_err();
        }
    };

    let target = &state.target;

    // Validate SetOutput constraints
    if let DeviceCommandPayload::SetOutput { duration_ms, .. } = &cmd.payload {
        if let Some(handler) = lookup_handler(target.raw_sensor_type) {
            if handler.sensor_type != SensorType::ContactOutput {
                return event_tx
                    .send(AdapterEvent::AdapterError {
                        device_key: Some(cmd.device_key),
                        error: "SetOutput sent to non-ContactOutput device".to_string(),
                    })
                    .await
                    .is_err();
            }
        } else {
            return event_tx
                .send(AdapterEvent::AdapterError {
                    device_key: Some(cmd.device_key),
                    error: "SetOutput: unknown sensor type in registry".to_string(),
                })
                .await
                .is_err();
        }

        if let Some(ms) = duration_ms
            && *ms > u16::MAX as u32
        {
            return event_tx
                .send(AdapterEvent::AdapterError {
                    device_key: Some(cmd.device_key),
                    error: format!("duration_ms {} exceeds u16 range (max {})", ms, u16::MAX),
                })
                .await
                .is_err();
        }
    }

    let downlink_cmd = match cmd.payload {
        DeviceCommandPayload::RequestReading => DownlinkCommand::ImmediateUplink {
            sensor_type: target.raw_sensor_type,
        },
        DeviceCommandPayload::QueryConfig => DownlinkCommand::ParameterGet,
        DeviceCommandPayload::SetOutput { value, duration_ms } => DownlinkCommand::ContactOutput {
            signal_mode: if value { 1 } else { 0 },
            signal_out_time: duration_ms.map(|ms| ms as u16).unwrap_or(0),
        },
    };

    let bytes = match BravePiCodec::encode_downlink(&target.device_number_hex, &downlink_cmd) {
        Ok(b) => b,
        Err(e) => {
            return event_tx
                .send(AdapterEvent::AdapterError {
                    device_key: Some(cmd.device_key),
                    error: format!("encode_downlink failed: {}", e),
                })
                .await
                .is_err();
        }
    };

    match write_tx.try_send(bytes) {
        Ok(()) => false,
        Err(mpsc::error::TrySendError::Full(_)) => event_tx
            .send(AdapterEvent::AdapterError {
                device_key: Some(cmd.device_key),
                error: "downlink queue full".to_string(),
            })
            .await
            .is_err(),
        Err(mpsc::error::TrySendError::Closed(_)) => event_tx
            .send(AdapterEvent::AdapterError {
                device_key: None,
                error: "write channel closed (transport failure)".to_string(),
            })
            .await
            .is_err(),
    }
}
