//! Frozen supervision / legacy-southbound vocabulary from D4/D12.
//! New variants and new dependents are forbidden without a corpus decision;
//! the dependent set is pinned by `scripts/check-layers` rule 7.

use std::collections::BTreeMap;

use iotkit_core_types::{DeviceKey, SensorIdentity, SensorReading};

/// adapter → core へ送信するイベント。
#[derive(Debug, Clone, PartialEq)]
pub enum AdapterEvent {
    /// センサーデータ受信。
    SensorData {
        device_key: DeviceKey,
        reading: SensorReading,
        rssi: Option<i16>,
        battery_pct: Option<u8>,
        ingested_at: std::time::SystemTime,
    },

    /// 新しいデバイスを発見。
    DeviceDiscovered {
        device_key: DeviceKey,
        identity: SensorIdentity,
    },

    /// デバイスがロスト。
    DeviceLost {
        device_key: DeviceKey,
        reason: String,
    },

    /// adapter 内部エラー。
    AdapterError {
        device_key: Option<DeviceKey>,
        error: String,
    },

    /// デバイス設定の応答。QueryConfig の結果として非同期に返る。
    DeviceConfig {
        device_key: DeviceKey,
        config: DeviceConfigData,
    },
}

/// core → adapter へ送信するコマンド。
#[derive(Debug, Clone, PartialEq)]
pub enum AdapterCommand {
    /// シャットダウン要求。
    Shutdown,
    /// デバイス宛コマンド。
    DeviceCommand(DeviceCommand),
}

/// device-targeted command の共通 envelope。
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceCommand {
    pub device_key: DeviceKey,
    pub payload: DeviceCommandPayload,
}

/// device command の payload。adapter 横断で意味が通る名前。
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceCommandPayload {
    /// センサーに即時読み取りを要求する。
    RequestReading,
    /// デバイスの設定情報を問い合わせる。
    QueryConfig,
    /// 接点出力を設定する。
    SetOutput {
        value: bool,
        duration_ms: Option<u32>,
    },
}

/// デバイス設定の応答 DTO。adapter 横断で共通の named field + adapter 固有の typed properties。
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceConfigData {
    pub firmware_version: Option<String>,
    pub uplink_interval_secs: Option<u32>,
    pub properties: BTreeMap<String, ConfigValue>,
}

/// 型付き設定値。downstream が parse 不要で使える lossless 表現。
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigValue {
    String(String),
    Integer(i64),
    Float(f64),
    Bool(bool),
}

#[cfg(test)]
#[path = "../tests/unit/lib_tests.rs"]
mod tests;
