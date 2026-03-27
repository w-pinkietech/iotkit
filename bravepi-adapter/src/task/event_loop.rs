//! async task: raw bytes → codec → AdapterEvent。
//! デバイスのライフサイクル追跡もここで行う。

use std::collections::HashMap;

use bravepi_codec::BravePiCodec;
use iotkit_core_types::{AdapterCommand, AdapterEvent, DeviceKey};
use tokio::sync::mpsc;

use crate::transport::{BytesReceiver, BytesSender};
use super::convert::frame_to_event;

struct DeviceState {
    /// Populated now; read by timeout-based DeviceLost logic (future sub-project).
    #[allow(dead_code)]
    last_seen: tokio::time::Instant,
}

pub(crate) async fn event_loop(
    port_path: String,
    mut bytes_rx: BytesReceiver,
    event_tx: mpsc::Sender<AdapterEvent>,
    mut command_rx: mpsc::Receiver<AdapterCommand>,
    write_tx: BytesSender,
) {
    let _ = write_tx;
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
                    Some(AdapterCommand::DeviceCommand(_)) => {
                        tracing::warn!("DeviceCommand not yet implemented");
                    }
                }
            }
            result = bytes_rx.recv() => {
                match result {
                    Some(Ok(data)) => {
                        codec.feed(&data);
                        while let Some(frame) = codec.decode() {
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
                                                devices.insert(
                                                    device_key.clone(),
                                                    DeviceState { last_seen: tokio::time::Instant::now() },
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
