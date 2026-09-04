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

// ── 公開識別子（MQTT Output Adapter v1 契約） ───────────────

/// edge-node-id と pipeline-id に共通する制約違反。
///
/// 契約: `^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$` に一致し、UTF-8 で 1〜64 バイト。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentifierError {
    Empty,
    TooLong { bytes: usize },
    InvalidChar { position: usize, ch: char },
    LeadingOrTrailingHyphen,
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "must not be empty"),
            Self::TooLong { bytes } => write!(f, "must be at most 64 bytes, got {bytes}"),
            Self::InvalidChar { position, ch } => write!(
                f,
                "must contain only lowercase ASCII letters, digits, and '-', found {ch:?} at byte {position}"
            ),
            Self::LeadingOrTrailingHyphen => write!(f, "must not start or end with '-'"),
        }
    }
}

impl std::error::Error for IdentifierError {}

pub const IDENTIFIER_MAX_BYTES: usize = 64;

/// Checks the shared grammar for edge-node-id and pipeline-id.
pub fn validate_identifier(value: &str) -> Result<(), IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError::Empty);
    }
    if value.len() > IDENTIFIER_MAX_BYTES {
        return Err(IdentifierError::TooLong { bytes: value.len() });
    }
    if let Some((position, ch)) = value
        .char_indices()
        .find(|(_, ch)| !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || *ch == '-'))
    {
        return Err(IdentifierError::InvalidChar { position, ch });
    }
    if value.starts_with('-') || value.ends_with('-') {
        return Err(IdentifierError::LeadingOrTrailingHyphen);
    }
    Ok(())
}

macro_rules! contract_identifier {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                validate_identifier(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdentifierError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl std::str::FromStr for $name {
            type Err = IdentifierError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

contract_identifier!(
    /// IoTKit 端末を識別する安定した ID。起動設定で与え、Broker namespace 内で一意。
    EdgeNodeId
);

contract_identifier!(
    /// 端末内の処理 pipeline を識別する、利用者が設定する安定した ID。
    PipelineId
);

#[cfg(test)]
#[path = "../tests/unit/lib_tests.rs"]
mod tests;
