//! Internal state: DeviceState + State apply logic.

use std::collections::HashMap;
use tokio::time::Instant;

use iotkit_core_types::{AdapterEvent, DeviceKey, SensorIdentity};

use crate::{DeviceView, EngineDeviceKey, EngineEvent};

pub(crate) struct DeviceState {
    pub view: DeviceView,
    #[allow(dead_code)]
    pub discovered_at: Instant,
    pub last_seen: Instant,
}

pub(crate) struct State {
    devices: HashMap<EngineDeviceKey, DeviceState>,
}

impl State {
    pub fn new() -> Self {
        Self {
            devices: HashMap::new(),
        }
    }

    pub fn apply(&mut self, event: EngineEvent) {
        let adapter_id = event.adapter_id;
        match event.event {
            AdapterEvent::DeviceDiscovered { device_key, identity } => {
                self.apply_discovered(adapter_id, device_key, identity);
            }
            AdapterEvent::SensorData { device_key, reading, rssi, battery_pct } => {
                let key = EngineDeviceKey {
                    adapter_id,
                    device_key: device_key.clone(),
                };
                match self.devices.get_mut(&key) {
                    Some(ds) => {
                        ds.view.last_reading = Some(reading);
                        ds.view.rssi = rssi;
                        ds.view.battery_pct = battery_pct;
                        ds.last_seen = Instant::now();
                    }
                    None => {
                        tracing::warn!(
                            device_key = %device_key,
                            "SensorData for unknown device, ignoring"
                        );
                    }
                }
            }
            _ => {
                // Remaining variants handled in Task 3
            }
        }
    }

    fn apply_discovered(
        &mut self,
        adapter_id: iotkit_core_types::AdapterId,
        device_key: DeviceKey,
        identity: SensorIdentity,
    ) {
        let key = EngineDeviceKey {
            adapter_id,
            device_key,
        };
        let now = Instant::now();
        match self.devices.get_mut(&key) {
            Some(ds) => {
                ds.view.identity = identity;
                ds.last_seen = now;
            }
            None => {
                self.devices.insert(key.clone(), DeviceState {
                    view: DeviceView {
                        key,
                        identity,
                        last_reading: None,
                        rssi: None,
                        battery_pct: None,
                        config: None,
                        last_error: None,
                    },
                    discovered_at: now,
                    last_seen: now,
                });
            }
        }
    }

    pub fn devices(&self) -> Vec<DeviceView> {
        self.devices.values().map(|ds| ds.view.clone()).collect()
    }

    pub fn device(&self, key: &EngineDeviceKey) -> Option<DeviceView> {
        self.devices.get(key).map(|ds| ds.view.clone())
    }
}
