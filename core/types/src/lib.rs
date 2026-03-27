//! iotkit-core-types: ドメインのエンティティ型。
//! core 層に属し、adapter や driver はこれに依存する（逆はない）。
//! プロトコル固有の番号や変換ロジックは持たない。

use std::collections::BTreeMap;
use std::fmt;

/// センサータイプ。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SensorType {
    ContactInput,
    ContactOutput,
    Adc,
    Ranging,
    Temperature,
    Acceleration,
    DifferentialPressure,
    Illuminance,
    Unknown(String),
}

impl fmt::Display for SensorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContactInput => write!(f, "ContactInput"),
            Self::ContactOutput => write!(f, "ContactOutput"),
            Self::Adc => write!(f, "ADC"),
            Self::Ranging => write!(f, "Ranging"),
            Self::Temperature => write!(f, "Temperature"),
            Self::Acceleration => write!(f, "Acceleration"),
            Self::DifferentialPressure => write!(f, "DifferentialPressure"),
            Self::Illuminance => write!(f, "Illuminance"),
            Self::Unknown(v) => write!(f, "Unknown({})", v),
        }
    }
}

/// 接続方式の大分類。コアは「種類」を知るが「詳細」は知らない。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConnectionKind {
    Uart,
    I2c,
    Gpio,
    Modbus,
    Other(String),
}

impl fmt::Display for ConnectionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uart => write!(f, "UART"),
            Self::I2c => write!(f, "I2C"),
            Self::Gpio => write!(f, "GPIO"),
            Self::Modbus => write!(f, "Modbus"),
            Self::Other(v) => write!(f, "{}", v),
        }
    }
}

/// 接続の詳細情報。大分類 + キーバリューのパラメータ。
/// adapter が型安全な内部表現からこの形式に変換して core に渡す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub kind: ConnectionKind,
    pub parameters: BTreeMap<String, String>,
}

impl fmt::Display for ConnectionInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", self.kind)?;
        let params: Vec<String> = self.parameters.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        write!(f, "{})", params.join(", "))
    }
}

/// センサーの素性（基本変わらない情報）。
/// UI表示、アセット管理、調達、メンテナンスに使う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SensorIdentity {
    pub manufacturer: String,
    pub ic_part_number: String,
    pub sensor_type: SensorType,
    pub connection: ConnectionInfo,
}

/// センサーの値（毎回変わる）。
#[derive(Debug, Clone, PartialEq)]
pub struct SensorReading {
    pub sensor_type: SensorType,
    pub values: Vec<f64>,
    pub labels: Vec<&'static str>,
}

impl SensorReading {
    pub fn new(sensor_type: SensorType, values: Vec<f64>, labels: Vec<&'static str>) -> Self {
        Self { sensor_type, values, labels }
    }

    pub fn empty(sensor_type: SensorType) -> Self {
        Self { sensor_type, values: vec![], labels: vec![] }
    }
}

// ── Adapter-Core 境界型 ──────────────────────────────────

/// adapter の一意識別子。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AdapterId(String);

impl AdapterId {
    pub fn new(id: impl Into<String>) -> Self { Self(id.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
}

/// デバイスの一意キー。adapter 内で一意であればよい。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceKey(String);

impl DeviceKey {
    pub fn new(key: impl Into<String>) -> Self { Self(key.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
}

impl fmt::Display for AdapterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for DeviceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// adapter → core へ送信するイベント。
#[derive(Debug, Clone, PartialEq)]
pub enum AdapterEvent {
    /// センサーデータ受信。
    SensorData {
        device_key: DeviceKey,
        reading: SensorReading,
        rssi: Option<i16>,
        battery_pct: Option<u8>,
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
        assert_eq!(ConfigValue::String("hello".into()), ConfigValue::String("hello".into()));
        assert_eq!(ConfigValue::Integer(42), ConfigValue::Integer(42));
        assert_eq!(ConfigValue::Float(3.14), ConfigValue::Float(3.14));
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
