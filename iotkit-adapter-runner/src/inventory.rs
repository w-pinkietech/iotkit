use iotkit_core_mqtt_contract::{encode_event, inventory_topic};
use iotkit_core_types::{AdapterId, AdapterEvent, DeviceKey};
use rumqttc::{AsyncClient, QoS};
use std::collections::HashMap;

/// Tracks active devices and manages retained inventory messages.
pub(crate) struct InventoryTracker {
    adapter_id: AdapterId,
    /// device_key_str -> last discovery payload (JSON bytes)
    active_devices: HashMap<String, Vec<u8>>,
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
                        .insert(device_key.as_str().to_string(), payload);
                }
                true
            }
            AdapterEvent::DeviceLost { device_key, .. } => {
                self.active_devices.remove(device_key.as_str());
                true
            }
            _ => false,
        }
    }

    /// Publish retained inventory for a single event to MQTT.
    /// Call only when MQTT is connected.
    pub async fn publish_event(&self, event: &AdapterEvent, client: &AsyncClient) {
        match event {
            AdapterEvent::DeviceDiscovered { device_key, .. } => {
                if let Some(payload) = self.active_devices.get(device_key.as_str()) {
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
                }
            }
            _ => {}
        }
    }

    /// Re-publish all active device inventory (called on MQTT reconnect).
    pub async fn republish_all(&self, client: &AsyncClient) {
        for (device_key_str, payload) in &self.active_devices {
            let dk = DeviceKey::new(device_key_str.clone());
            let topic = inventory_topic(&self.adapter_id, &dk);
            if let Err(e) = client
                .publish(&topic, QoS::AtLeastOnce, true, payload.clone())
                .await
            {
                tracing::warn!(error = %e, device = %device_key_str, "failed to republish inventory on reconnect");
            }
        }
        if !self.active_devices.is_empty() {
            tracing::info!(
                count = self.active_devices.len(),
                "republished inventory on reconnect"
            );
        }
    }
}
