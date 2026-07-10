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

impl SensorType {
    /// Convert to the string stored in SQLite sensor_type column.
    pub fn as_db_str(&self) -> &str {
        match self {
            Self::ContactInput => "contact_input",
            Self::ContactOutput => "contact_output",
            Self::Adc => "adc",
            Self::Ranging => "ranging",
            Self::Temperature => "temperature",
            Self::Acceleration => "acceleration",
            Self::DifferentialPressure => "differential_pressure",
            Self::Illuminance => "illuminance",
            Self::Unknown(s) => s.as_str(),
        }
    }

    /// Parse from the string stored in SQLite sensor_type column.
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "contact_input" => Self::ContactInput,
            "contact_output" => Self::ContactOutput,
            "adc" => Self::Adc,
            "ranging" => Self::Ranging,
            "temperature" => Self::Temperature,
            "acceleration" => Self::Acceleration,
            "differential_pressure" => Self::DifferentialPressure,
            "illuminance" => Self::Illuminance,
            other => Self::Unknown(other.to_string()),
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
        let params: Vec<String> = self
            .parameters
            .iter()
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
    pub labels: Vec<String>,
}

impl SensorReading {
    pub fn new(sensor_type: SensorType, values: Vec<f64>, labels: Vec<String>) -> Self {
        Self {
            sensor_type,
            values,
            labels,
        }
    }

    pub fn empty(sensor_type: SensorType) -> Self {
        Self {
            sensor_type,
            values: vec![],
            labels: vec![],
        }
    }
}

// ── Adapter-Core 境界型 ──────────────────────────────────

/// adapter の一意識別子。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AdapterId(String);

impl AdapterId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// デバイスの一意キー。adapter 内で一意であればよい。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceKey(String);

impl DeviceKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensor_type_db_str_round_trip() {
        let variants: Vec<SensorType> = vec![
            SensorType::ContactInput,
            SensorType::ContactOutput,
            SensorType::Adc,
            SensorType::Ranging,
            SensorType::Temperature,
            SensorType::Acceleration,
            SensorType::DifferentialPressure,
            SensorType::Illuminance,
        ];
        for v in variants {
            let db_str = v.as_db_str();
            let round_tripped = SensorType::from_db_str(db_str);
            assert_eq!(
                v, round_tripped,
                "round-trip failed for {v:?} -> {db_str:?}"
            );
        }
    }

    #[test]
    fn sensor_type_unknown_round_trip() {
        let original = SensorType::Unknown("custom_xyz".to_string());
        let db_str = original.as_db_str();
        assert_eq!(db_str, "custom_xyz");
        let round_tripped = SensorType::from_db_str(db_str);
        assert_eq!(round_tripped, SensorType::Unknown("custom_xyz".to_string()));
    }
}
