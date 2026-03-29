use crate::inventory::InventoryTracker;
use iotkit_core_mqtt_contract::{encode_event, topic};
use iotkit_core_types::{AdapterId, AdapterEvent};
use rumqttc::{AsyncClient, QoS};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Notify};

/// Consume events from adapter and publish to MQTT.
/// Runs until event_rx is closed.
/// Checks `connected` flag before publishing; buffers inventory events when disconnected.
/// Listens on `reconnect_notify` to republish all inventory on MQTT reconnect.
pub(crate) async fn run(
    adapter_id: AdapterId,
    client: AsyncClient,
    mut event_rx: mpsc::Receiver<AdapterEvent>,
    mut inventory: InventoryTracker,
    connected: Arc<AtomicBool>,
    reconnect_notify: Arc<Notify>,
) {
    loop {
        tokio::select! {
            maybe_event = event_rx.recv() => {
                let Some(event) = maybe_event else {
                    break;
                };

                // Always track in inventory regardless of connection state
                inventory.track_event(&event);

                // Only publish to MQTT when connected
                if !connected.load(Ordering::Relaxed) {
                    tracing::warn!("MQTT disconnected, event tracked locally but not published");
                    continue;
                }

                // Publish retained inventory for discovery/loss events
                inventory.publish_event(&event, &client).await;

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
            _ = reconnect_notify.notified() => {
                tracing::info!("MQTT reconnected, republishing inventory");
                inventory.republish_all(&client).await;
            }
        }
    }

    tracing::info!("adapter event channel closed, publish loop exiting");
}
