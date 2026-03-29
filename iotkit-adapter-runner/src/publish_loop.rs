use crate::inventory::InventoryTracker;
use iotkit_core_mqtt_contract::{encode_event, topic};
use iotkit_core_types::{AdapterId, AdapterEvent};
use rumqttc::{AsyncClient, QoS};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Consume events from adapter and publish to MQTT.
/// Runs until event_rx is closed.
/// Checks `connected` flag before publishing; drops events with warn! when disconnected.
pub(crate) async fn run(
    adapter_id: AdapterId,
    client: AsyncClient,
    mut event_rx: mpsc::Receiver<AdapterEvent>,
    mut inventory: InventoryTracker,
    connected: Arc<AtomicBool>,
) {
    while let Some(event) = event_rx.recv().await {
        // Check connection state - drop events when disconnected
        if !connected.load(Ordering::Relaxed) {
            tracing::warn!("MQTT disconnected, dropping event");
            continue;
        }

        // Update inventory tracking (retained publish for discovery/loss)
        inventory.process_event(&event, &client).await;

        // Encode and publish to event topic
        match encode_event(&adapter_id, &event) {
            Ok((event_type, payload)) => {
                let t = topic(&adapter_id, event_type);
                if let Err(e) = client.publish(&t, QoS::AtLeastOnce, false, payload).await {
                    tracing::warn!(
                        error = %e,
                        event_type = ?event_type,
                        "MQTT publish failed, dropping event"
                    );
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "skipping unencodable event");
            }
        }
    }

    tracing::info!("adapter event channel closed, publish loop exiting");
}
