//! Internal state: DeviceState + State apply logic.

use std::collections::HashMap;
use tokio::time::Instant;

use crate::{DeviceView, EngineDeviceKey, EngineEvent};

pub(crate) struct DeviceState {
    pub view: DeviceView,
    #[allow(dead_code)]
    pub discovered_at: Instant,
    #[allow(dead_code)]
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

    pub fn apply(&mut self, _event: EngineEvent) {
        // TODO: implement in Task 2
    }

    pub fn devices(&self) -> Vec<DeviceView> {
        self.devices.values().map(|ds| ds.view.clone()).collect()
    }

    pub fn device(&self, key: &EngineDeviceKey) -> Option<DeviceView> {
        self.devices.get(key).map(|ds| ds.view.clone())
    }
}
