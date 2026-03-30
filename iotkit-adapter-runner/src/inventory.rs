use iotkit_core_mqtt_contract::{now_ms, InventoryData};
use iotkit_core_types::AdapterEvent;
use std::collections::HashMap;

/// Inventory tracker. Exclusively owned by publish_task.
pub(crate) struct Inventory {
    pub desired: HashMap<String, Option<InventoryData>>,
}

impl Inventory {
    pub fn new() -> Self {
        Self {
            desired: HashMap::new(),
        }
    }

    /// Track a DeviceDiscovered or DeviceLost event.
    /// Returns true if the event affected inventory, false otherwise.
    pub fn track_event(&mut self, event: &AdapterEvent) -> bool {
        match event {
            AdapterEvent::DeviceDiscovered {
                device_key,
                identity,
            } => {
                let key = device_key.as_str().to_string();
                let existing = self.desired.get(&key);

                // If already active (Some), preserve first_seen_at.
                // If tombstone (None) or absent, set new first_seen_at.
                let first_seen_at = match existing {
                    Some(Some(data)) => data.first_seen_at,
                    _ => now_ms(),
                };

                self.desired.insert(
                    key,
                    Some(InventoryData {
                        device_key: device_key.clone(),
                        identity: identity.clone(),
                        first_seen_at,
                    }),
                );
                true
            }
            AdapterEvent::DeviceLost { device_key, .. } => {
                let key = device_key.as_str().to_string();
                self.desired.insert(key, None);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_types::*;
    use std::collections::BTreeMap;

    fn make_discovery(key: &str) -> AdapterEvent {
        AdapterEvent::DeviceDiscovered {
            device_key: DeviceKey::new(key),
            identity: SensorIdentity {
                manufacturer: "Test".into(),
                ic_part_number: "T1".into(),
                sensor_type: SensorType::Temperature,
                connection: ConnectionInfo {
                    kind: ConnectionKind::I2c,
                    parameters: BTreeMap::new(),
                },
            },
        }
    }

    fn make_loss(key: &str) -> AdapterEvent {
        AdapterEvent::DeviceLost {
            device_key: DeviceKey::new(key),
            reason: "timeout".into(),
        }
    }

    #[test]
    fn track_discovery_creates_active_entry() {
        let mut inv = Inventory::new();
        let tracked = inv.track_event(&make_discovery("sensor-a"));
        assert!(tracked);
        assert!(inv.desired.get("sensor-a").unwrap().is_some());
    }

    #[test]
    fn track_loss_creates_tombstone() {
        let mut inv = Inventory::new();
        inv.track_event(&make_discovery("sensor-a"));
        let tracked = inv.track_event(&make_loss("sensor-a"));
        assert!(tracked);
        assert!(inv.desired.get("sensor-a").unwrap().is_none());
    }

    #[test]
    fn rediscovery_after_loss_resets_first_seen_at() {
        let mut inv = Inventory::new();
        inv.track_event(&make_discovery("sensor-a"));
        let first = inv.desired["sensor-a"].as_ref().unwrap().first_seen_at;
        std::thread::sleep(std::time::Duration::from_millis(2));
        inv.track_event(&make_loss("sensor-a"));
        inv.track_event(&make_discovery("sensor-a"));
        let second = inv.desired["sensor-a"].as_ref().unwrap().first_seen_at;
        assert!(
            second > first,
            "first_seen_at must reset on rediscovery after loss"
        );
    }

    #[test]
    fn rediscovery_without_loss_preserves_first_seen_at() {
        let mut inv = Inventory::new();
        inv.track_event(&make_discovery("sensor-a"));
        let first = inv.desired["sensor-a"].as_ref().unwrap().first_seen_at;
        std::thread::sleep(std::time::Duration::from_millis(2));
        // Second discovery without intervening loss
        inv.track_event(&make_discovery("sensor-a"));
        let second = inv.desired["sensor-a"].as_ref().unwrap().first_seen_at;
        assert_eq!(
            first, second,
            "first_seen_at must be preserved when already active"
        );
    }

    #[test]
    fn track_telemetry_returns_false() {
        let mut inv = Inventory::new();
        let event = AdapterEvent::SensorData {
            device_key: DeviceKey::new("test"),
            reading: SensorReading::empty(SensorType::Temperature),
            rssi: None,
            battery_pct: None,
            ingested_at: std::time::SystemTime::now(),
        };
        assert!(!inv.track_event(&event));
    }

    #[test]
    fn loss_for_unknown_device_creates_tombstone() {
        let mut inv = Inventory::new();
        inv.track_event(&make_loss("unknown"));
        assert!(inv.desired.get("unknown").unwrap().is_none());
    }

    #[test]
    fn lost_then_rediscovered_offline_shows_latest() {
        let mut inv = Inventory::new();
        inv.track_event(&make_discovery("sensor-a"));
        inv.track_event(&make_loss("sensor-a"));
        inv.track_event(&make_discovery("sensor-a"));
        // Final state: active
        assert!(inv.desired["sensor-a"].is_some());
    }
}
