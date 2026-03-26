//! async task: raw bytes → codec → AdapterEvent。
//! デバイスのライフサイクル追跡もここで行う。

use std::collections::HashSet;

use bravepi_codec::BravePiCodec;
use iotkit_core_types::{AdapterCommand, AdapterEvent, DeviceKey};
use tokio::sync::mpsc;

use crate::transport::BytesReceiver;
use super::convert::frame_to_event;

pub(crate) async fn event_loop(
    port_path: String,
    mut bytes_rx: BytesReceiver,
    event_tx: mpsc::Sender<AdapterEvent>,
    mut command_rx: mpsc::Receiver<AdapterCommand>,
) {
    tracing::info!(port = %port_path, "BravePI adapter event loop started");

    let mut codec = BravePiCodec::new();
    // デバイスのライフサイクル追跡。adapter task 終了時に解放される。
    // BravePI は物理的に固定台数のため、実運用で数十台規模に収まる。
    let mut seen_devices: HashSet<DeviceKey> = HashSet::new();

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
                }
            }
            result = bytes_rx.recv() => {
                match result {
                    Some(Ok(data)) => {
                        codec.feed(&data);
                        while let Some(frame) = codec.decode() {
                            if let Some((event, identity)) = frame_to_event(frame, &port_path) {
                                // 初回デバイスは DeviceDiscovered を先に送信
                                if let AdapterEvent::SensorData { ref device_key, .. } = event {
                                    // seen_devices は identity の有無に関わらず記録する。
                                    // adapter 再起動時にリセットされるため、DeviceDiscovered は再送信される（意図通り）。
                                    if seen_devices.insert(device_key.clone()) {
                                        if let Some(identity) = identity {
                                            let discovered = AdapterEvent::DeviceDiscovered {
                                                device_key: device_key.clone(),
                                                identity,
                                            };
                                            if event_tx.send(discovered).await.is_err() {
                                                tracing::warn!("Event channel closed, shutting down");
                                                return;
                                            }
                                        }
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
