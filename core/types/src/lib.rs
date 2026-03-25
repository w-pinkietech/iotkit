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
}

impl SensorReading {
    pub fn new(sensor_type: SensorType, values: Vec<f64>) -> Self {
        Self { sensor_type, values }
    }

    pub fn empty(sensor_type: SensorType) -> Self {
        Self { sensor_type, values: vec![] }
    }
}
