use crate::inventory::InventoryTracker;
use iotkit_core_mqtt_contract::{encode_event, topic, EventType};
use iotkit_core_types::{AdapterId, AdapterEvent};
use rumqttc::{AsyncClient, QoS};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Notify};

/// Maximum number of events buffered while MQTT is disconnected.
const PENDING_BUFFER_CAP: usize = 1000;

/// Consume events from adapter and publish to MQTT.
/// Runs until event_rx is closed.
/// Buffers events when MQTT is disconnected and flushes on reconnect.
/// Inventory tracking always runs regardless of connection state.
/// Listens on `reconnect_notify` to republish all inventory on MQTT reconnect.
pub(crate) async fn run(
    adapter_id: AdapterId,
    client: AsyncClient,
    mut event_rx: mpsc::Receiver<AdapterEvent>,
    mut inventory: InventoryTracker,
    connected: Arc<AtomicBool>,
    reconnect_notify: Arc<Notify>,
) {
    let mut pending_events: VecDeque<(EventType, Vec<u8>)> = VecDeque::new();

    loop {
        tokio::select! {
            maybe_event = event_rx.recv() => {
                let Some(event) = maybe_event else {
                    break;
                };

                // Always track in inventory regardless of connection state
                inventory.track_event(&event);

                if connected.load(Ordering::Relaxed) {
                    // Publish retained inventory for discovery/loss events
                    inventory.publish_event(&event, &client).await;

                    // Encode and publish to event topic
                    publish_event(&adapter_id, &event, &client).await;
                } else {
                    // Buffer encoded event for later flush
                    buffer_event(&adapter_id, &event, &mut pending_events);
                }
            }
            _ = reconnect_notify.notified() => {
                tracing::info!("MQTT reconnected, republishing inventory");
                inventory.republish_all(&client).await;

                // Flush buffered events
                let count = pending_events.len();
                if count > 0 {
                    tracing::info!(count, "flushing buffered events after reconnect");
                }
                while let Some((event_type, payload)) = pending_events.pop_front() {
                    let t = topic(&adapter_id, event_type);
                    if let Err(e) = client.publish(&t, QoS::AtLeastOnce, false, payload).await {
                        tracing::warn!(
                            error = %e,
                            event_type = ?event_type,
                            "MQTT publish failed during flush, dropping event"
                        );
                    }
                }
            }
        }
    }

    tracing::info!("adapter event channel closed, publish loop exiting");
}

/// Encode and publish a single event to MQTT immediately.
async fn publish_event(adapter_id: &AdapterId, event: &AdapterEvent, client: &AsyncClient) {
    match encode_event(adapter_id, event) {
        Ok((event_type, payload)) => {
            let t = topic(adapter_id, event_type);
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

/// Encode an event and push it to the pending buffer. Drops oldest if full.
fn buffer_event(
    adapter_id: &AdapterId,
    event: &AdapterEvent,
    pending: &mut VecDeque<(EventType, Vec<u8>)>,
) {
    match encode_event(adapter_id, event) {
        Ok((event_type, payload)) => {
            if pending.len() >= PENDING_BUFFER_CAP {
                pending.pop_front();
                tracing::warn!("pending event buffer full, dropping oldest event");
            }
            pending.push_back((event_type, payload));
        }
        Err(e) => {
            tracing::debug!(error = %e, "skipping unencodable event (disconnected)");
        }
    }
}
