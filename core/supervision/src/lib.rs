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
mod tests {
    use super::*;

    #[test]
    fn device_command_construction() {
        let cmd = DeviceCommand {
            device_key: DeviceKey::new("bravepi:abc:temperature"),
            payload: DeviceCommandPayload::RequestReading,
        };
        assert_eq!(cmd.device_key.as_str(), "bravepi:abc:temperature");
    }

    #[test]
    fn device_command_set_output() {
        let cmd = DeviceCommand {
            device_key: DeviceKey::new("bravepi:abc:contact_output"),
            payload: DeviceCommandPayload::SetOutput {
                value: true,
                duration_ms: Some(5000),
            },
        };
        match cmd.payload {
            DeviceCommandPayload::SetOutput { value, duration_ms } => {
                assert!(value);
                assert_eq!(duration_ms, Some(5000));
            }
            _ => panic!("expected SetOutput"),
        }
    }

    #[test]
    fn adapter_command_device_command_variant() {
        let cmd = AdapterCommand::DeviceCommand(DeviceCommand {
            device_key: DeviceKey::new("test"),
            payload: DeviceCommandPayload::QueryConfig,
        });
        match cmd {
            AdapterCommand::DeviceCommand(dc) => {
                assert_eq!(dc.device_key.as_str(), "test");
            }
            _ => panic!("expected DeviceCommand"),
        }
    }

    #[test]
    fn device_config_data_construction() {
        let config = DeviceConfigData {
            firmware_version: Some("1.2.3".to_string()),
            uplink_interval_secs: Some(60),
            properties: BTreeMap::from([
                ("timezone".into(), ConfigValue::Integer(9)),
                ("ble_mode".into(), ConfigValue::Integer(1)),
            ]),
        };
        assert_eq!(config.firmware_version.as_deref(), Some("1.2.3"));
        assert_eq!(config.uplink_interval_secs, Some(60));
        assert_eq!(config.properties.len(), 2);
    }

    #[test]
    fn config_value_variants() {
        assert_eq!(
            ConfigValue::String("hello".into()),
            ConfigValue::String("hello".into())
        );
        assert_eq!(ConfigValue::Integer(42), ConfigValue::Integer(42));
        assert_eq!(ConfigValue::Float(1.5_f64), ConfigValue::Float(1.5_f64));
        assert_eq!(ConfigValue::Bool(true), ConfigValue::Bool(true));
    }

    #[test]
    fn adapter_event_device_config_variant() {
        let event = AdapterEvent::DeviceConfig {
            device_key: DeviceKey::new("bravepi:abc:temperature"),
            config: DeviceConfigData {
                firmware_version: Some("1.0.0".to_string()),
                uplink_interval_secs: None,
                properties: BTreeMap::new(),
            },
        };
        match event {
            AdapterEvent::DeviceConfig { device_key, config } => {
                assert_eq!(device_key.as_str(), "bravepi:abc:temperature");
                assert_eq!(config.firmware_version.as_deref(), Some("1.0.0"));
            }
            _ => panic!("expected DeviceConfig"),
        }
    }
}
