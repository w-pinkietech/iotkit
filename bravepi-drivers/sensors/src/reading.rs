//! 全センサー共通の出力型。
//! センサーがどこにいても、ここに集約される。

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

/// 接続タイプ。
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionType {
    Uart {
        port: String,
        transmitter_id: String,
    },
    I2c {
        bus: String,
        address: u8,
    },
    Gpio {
        pin: u8,
    },
}

impl fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uart { port, transmitter_id } => {
                write!(f, "UART({}:{})", port, transmitter_id)
            }
            Self::I2c { bus, address } => {
                write!(f, "I2C({}:0x{:02x})", bus, address)
            }
            Self::Gpio { pin } => {
                write!(f, "GPIO(BCM{})", pin)
            }
        }
    }
}

/// センサーの素性（基本変わらない情報）。
#[derive(Debug, Clone, PartialEq)]
pub struct SensorIdentity {
    pub manufacturer: &'static str,
    pub ic_part_number: &'static str,
    pub sensor_type: SensorType,
    pub connection_type: ConnectionType,
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
