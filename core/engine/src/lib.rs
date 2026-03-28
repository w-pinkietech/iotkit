//! iotkit-core-engine: adapter event の集約と device state の in-memory projection。
//! core/types のみに依存し、adapter 実装を知らない。

mod state;

#[cfg(test)]
mod state_test;

use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;

use iotkit_core_types::{
    AdapterId, AdapterEvent, DeviceConfigData, DeviceKey, SensorIdentity, SensorReading,
};

use state::State;

/// adapter_id 付き envelope。app binary が adapter の event_rx から受け取った
/// AdapterEvent を包んで engine に渡す。
#[derive(Debug, Clone)]
pub struct EngineEvent {
    pub adapter_id: AdapterId,
    pub event: AdapterEvent,
}

/// engine 内でデバイスをグローバルに一意に識別する。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EngineDeviceKey {
    pub adapter_id: AdapterId,
    pub device_key: DeviceKey,
}

impl fmt::Display for EngineDeviceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}", self.adapter_id, self.device_key)
    }
}

/// query API が返すデバイスの snapshot。
#[derive(Debug, Clone)]
pub struct DeviceView {
    pub key: EngineDeviceKey,
    pub identity: SensorIdentity,
    pub last_reading: Option<SensorReading>,
    pub rssi: Option<i16>,
    pub battery_pct: Option<u8>,
    pub config: Option<DeviceConfigData>,
    pub last_error: Option<String>,
}

/// adapter event を集約し、device state の in-memory projection を提供する。
/// Clone は cheap (Arc の clone)。
#[derive(Clone)]
pub struct Engine {
    state: Arc<RwLock<State>>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// 空の engine を作る。
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(State::new())),
        }
    }

    /// EngineEvent を 1 件処理して内部状態に反映する。
    pub async fn apply(&self, event: EngineEvent) {
        let mut state = self.state.write().await;
        state.apply(event);
    }

    /// 現在生存中の全デバイスの snapshot を返す。
    pub async fn devices(&self) -> Vec<DeviceView> {
        let state = self.state.read().await;
        state.devices()
    }

    /// 特定デバイスの snapshot を返す。存在しなければ None。
    pub async fn device(&self, key: &EngineDeviceKey) -> Option<DeviceView> {
        let state = self.state.read().await;
        state.device(key)
    }
}
