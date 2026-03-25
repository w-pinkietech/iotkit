//! iotkit-core-types: ドメインのエンティティ型。
//! core 層に属し、adapter や driver はこれに依存する（逆はない）。

use std::fmt;

/// センサータイプ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SensorType {
    ContactInput,
    ContactOutput,
    Adc,
    Ranging,
    Temperature,
    Acceleration,
    DifferentialPressure,
    Illuminance,
    Unknown(u16),
}

impl SensorType {
    pub fn from_raw(raw: u16) -> Self {
        match raw {
            257 => Self::ContactInput,
            258 => Self::ContactOutput,
            259 => Self::Adc,
            260 => Self::Ranging,
            261 => Self::Temperature,
            262 => Self::Acceleration,
            263 => Self::DifferentialPressure,
            264 => Self::Illuminance,
            other => Self::Unknown(other),
        }
    }

    pub fn to_raw(self) -> u16 {
        match self {
            Self::ContactInput => 257,
            Self::ContactOutput => 258,
            Self::Adc => 259,
            Self::Ranging => 260,
            Self::Temperature => 261,
            Self::Acceleration => 262,
            Self::DifferentialPressure => 263,
            Self::Illuminance => 264,
            Self::Unknown(v) => v,
        }
    }
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
