//! Frozen BravePI care/engine projection.
//!
//! The decoded runtime does not know these supervision types. This module is
//! retained only for the legacy `start` API.

use std::collections::BTreeMap;

use iotkit_core_supervision::{
    AdapterCommand, AdapterEvent, ConfigValue, DeviceCommandPayload, DeviceConfigData,
};
use tokio::sync::mpsc;

use super::event_loop::{
    RuntimeCommand, RuntimeDeviceCommand, RuntimeDeviceCommandPayload, RuntimeEvent,
    decoded_event_loop,
};
use crate::transport::{BytesReceiver, BytesSender};

pub(crate) async fn event_loop(
    adapter_id: String,
    port_path: String,
    bytes_rx: BytesReceiver,
    event_tx: mpsc::Sender<AdapterEvent>,
    command_rx: mpsc::Receiver<AdapterCommand>,
    write_tx: BytesSender,
    ingest: Option<iotkit_ingest_client::IngestClient>,
) {
    let (runtime_event_tx, runtime_event_rx) = mpsc::channel(256);
    let (runtime_command_tx, runtime_command_rx) = mpsc::channel(32);
    let runtime_handle = tokio::spawn(decoded_event_loop(
        port_path,
        bytes_rx,
        runtime_event_tx,
        runtime_command_rx,
        write_tx,
    ));
    let projection_handle = tokio::spawn(project_events(
        adapter_id,
        runtime_event_rx,
        runtime_command_tx.clone(),
        event_tx,
        command_rx,
        ingest,
    ));

    if let Err(error) = projection_handle.await {
        tracing::error!(%error, "BravePI legacy projection panicked");
    }
    let _ = runtime_command_tx.send(RuntimeCommand::Shutdown).await;
    if let Err(error) = runtime_handle.await {
        tracing::error!(%error, "BravePI decoded runtime panicked");
    }
}

async fn project_events(
    adapter_id: String,
    mut runtime_events: mpsc::Receiver<RuntimeEvent>,
    runtime_commands: mpsc::Sender<RuntimeCommand>,
    event_tx: mpsc::Sender<AdapterEvent>,
    mut command_rx: mpsc::Receiver<AdapterCommand>,
    ingest: Option<iotkit_ingest_client::IngestClient>,
) {
    loop {
        tokio::select! {
            command = command_rx.recv() => {
                let Some(command) = command else {
                    return;
                };
                let command = match command {
                    AdapterCommand::Shutdown => RuntimeCommand::Shutdown,
                    AdapterCommand::DeviceCommand(command) => {
                        RuntimeCommand::DeviceCommand(RuntimeDeviceCommand {
                            device_key: command.device_key,
                            payload: match command.payload {
                                DeviceCommandPayload::RequestReading => {
                                    RuntimeDeviceCommandPayload::RequestReading
                                }
                                DeviceCommandPayload::QueryConfig => {
                                    RuntimeDeviceCommandPayload::QueryConfig
                                }
                                DeviceCommandPayload::SetOutput { value, duration_ms } => {
                                    RuntimeDeviceCommandPayload::SetOutput { value, duration_ms }
                                }
                            },
                        })
                    }
                };
                let shutdown = matches!(command, RuntimeCommand::Shutdown);
                if runtime_commands.send(command).await.is_err() || shutdown {
                    return;
                }
            }
            event = runtime_events.recv() => {
                let Some(event) = event else {
                    return;
                };
                let projected = match event {
                    RuntimeEvent::DeviceDiscovered { device_key, identity } => {
                        AdapterEvent::DeviceDiscovered { device_key, identity }
                    }
                    RuntimeEvent::Observation(observation) => {
                        if let Some(client) = &ingest {
                            submit_legacy_ingest(client, &adapter_id, &observation);
                        }
                        AdapterEvent::SensorData {
                            device_key: observation.device_key,
                            reading: observation.reading,
                            rssi: observation.rssi,
                            battery_pct: observation.battery_pct,
                            ingested_at: observation.observed_at,
                        }
                    }
                    RuntimeEvent::DeviceConfig { device_key, config } => {
                        AdapterEvent::DeviceConfig {
                            device_key,
                            config: DeviceConfigData {
                                firmware_version: Some(config.firmware_version),
                                uplink_interval_secs: Some(config.uplink_interval_secs),
                                properties: BTreeMap::from([
                                    ("timezone".into(), ConfigValue::Integer(config.timezone)),
                                    ("ble_mode".into(), ConfigValue::Integer(config.ble_mode)),
                                    ("tx_power".into(), ConfigValue::Integer(config.tx_power)),
                                    (
                                        "advertise_interval".into(),
                                        ConfigValue::Integer(config.advertise_interval),
                                    ),
                                ]),
                            },
                        }
                    }
                    RuntimeEvent::Error { device_key, error } => {
                        AdapterEvent::AdapterError { device_key, error }
                    }
                };
                if event_tx.send(projected).await.is_err() {
                    return;
                }
            }
        }
    }
}

fn submit_legacy_ingest(
    client: &iotkit_ingest_client::IngestClient,
    adapter_id: &str,
    observation: &super::event_loop::DecodedObservation,
) {
    let Some(items) = super::ingest_map::to_items(
        &observation.device_key,
        &observation.reading,
        observation.rssi,
        observation.battery_pct,
    ) else {
        tracing::warn!(device_key = %observation.device_key, "no measurement mapping");
        return;
    };
    for chunk in items.chunks(super::ingest_map::MAX_ITEMS_PER_ENVELOPE) {
        let envelope = iotkit_ingest_client::new_envelope(adapter_id, chunk.to_vec());
        if let Err(error) = client.try_submit(envelope) {
            tracing::warn!(?error, "legacy ingest queue rejected observation");
        }
    }
}
