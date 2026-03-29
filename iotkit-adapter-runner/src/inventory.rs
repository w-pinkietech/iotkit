use iotkit_core_mqtt_contract::{encode_event, inventory_topic};
use iotkit_core_types::{AdapterId, AdapterEvent, DeviceKey};
use rumqttc::{AsyncClient, QoS};
use std::collections::HashMap;

/// Tracks active devices and manages retained inventory messages.
///
/// Entries are `Some(payload)` for active devices, `None` for tombstones
/// (devices lost while MQTT was disconnected, needing an empty-retained
/// publish to clear the broker's retained message).
pub(crate) struct InventoryTracker {
    adapter_id: AdapterId,
    /// device_key_str -> Some(payload) for active, None for tombstone
    active_devices: HashMap<String, Option<Vec<u8>>>,
}

impl InventoryTracker {
    pub fn new(adapter_id: AdapterId) -> Self {
        Self {
            adapter_id,
            active_devices: HashMap::new(),
        }
    }

    /// Track an event in the local inventory (no MQTT publish).
    /// Returns true if inventory was updated.
    pub fn track_event(&mut self, event: &AdapterEvent) -> bool {
        match event {
            AdapterEvent::DeviceDiscovered { device_key, .. } => {
                if let Ok((_, payload)) = encode_event(&self.adapter_id, event) {
                    self.active_devices
                        .insert(device_key.as_str().to_string(), Some(payload));
                }
                true
            }
            AdapterEvent::DeviceLost { device_key, .. } => {
                // Mark as tombstone (None) instead of removing.
                // republish_all() will send empty retained to clear the
                // broker, then remove the tombstone.
                self.active_devices
                    .insert(device_key.as_str().to_string(), None);
                true
            }
            _ => false,
        }
    }

    /// Publish retained inventory for a single event to MQTT.
    /// Call only when MQTT is connected.
    pub async fn publish_event(&mut self, event: &AdapterEvent, client: &AsyncClient) {
        match event {
            AdapterEvent::DeviceDiscovered { device_key, .. } => {
                if let Some(Some(payload)) = self.active_devices.get(device_key.as_str()) {
                    let topic = inventory_topic(&self.adapter_id, device_key);
                    if let Err(e) = client
                        .publish(&topic, QoS::AtLeastOnce, true, payload.clone())
                        .await
                    {
                        tracing::warn!(error = %e, device = device_key.as_str(), "failed to publish retained inventory");
                    } else {
                        tracing::debug!(device = device_key.as_str(), "published retained inventory");
                    }
                }
            }
            AdapterEvent::DeviceLost { device_key, .. } => {
                let topic = inventory_topic(&self.adapter_id, device_key);
                if let Err(e) = client
                    .publish(&topic, QoS::AtLeastOnce, true, Vec::<u8>::new())
                    .await
                {
                    tracing::warn!(error = %e, device = device_key.as_str(), "failed to clear retained inventory");
                } else {
                    tracing::debug!(device = device_key.as_str(), "cleared retained inventory");
                    // Successfully published delete — remove tombstone
                    self.active_devices.remove(device_key.as_str());
                }
            }
            _ => {}
        }
    }

    /// Re-publish all active device inventory (called on MQTT reconnect).
    /// Active devices get their payload re-published; tombstones get an
    /// empty retained message to clear the broker, then are removed.
    pub async fn republish_all(&mut self, client: &AsyncClient) {
        let mut published = 0u32;
        let mut tombstones_cleared = Vec::new();

        for (device_key_str, maybe_payload) in &self.active_devices {
            let dk = DeviceKey::new(device_key_str.clone());
            let topic = inventory_topic(&self.adapter_id, &dk);

            let payload_bytes = match maybe_payload {
                Some(payload) => payload.clone(),
                None => Vec::new(), // tombstone: send empty retained to delete
            };

            if let Err(e) = client
                .publish(&topic, QoS::AtLeastOnce, true, payload_bytes)
                .await
            {
                tracing::warn!(error = %e, device = %device_key_str, "failed to republish inventory on reconnect");
            } else {
                published += 1;
                if maybe_payload.is_none() {
                    tombstones_cleared.push(device_key_str.clone());
                }
            }
        }

        // Remove successfully-published tombstones
        for key in &tombstones_cleared {
            self.active_devices.remove(key);
        }

        if published > 0 || !tombstones_cleared.is_empty() {
            tracing::info!(
                active = published.saturating_sub(tombstones_cleared.len() as u32),
                tombstones = tombstones_cleared.len(),
                "republished inventory on reconnect"
            );
        }
    }
}
