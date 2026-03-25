//! adapter 層の型定義。
//! core 型 (SensorType, SensorReading) は iotkit-core-types から re-export する。

use std::fmt;

// core 型を re-export（既存コードの import を壊さないため）
pub use iotkit_core_types::{SensorReading, SensorType};

/// BravePI プロトコルの sensor_type 番号から core の SensorType に変換。
pub fn sensor_type_from_bravepi_raw(raw: u16) -> SensorType {
    match raw {
        257 => SensorType::ContactInput,
        258 => SensorType::ContactOutput,
        259 => SensorType::Adc,
        260 => SensorType::Ranging,
        261 => SensorType::Temperature,
        262 => SensorType::Acceleration,
        263 => SensorType::DifferentialPressure,
        264 => SensorType::Illuminance,
        other => SensorType::Unknown(format!("bravepi:{}", other)),
    }
}

/// 接続タイプ（adapter 層の関心事）。
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

/// センサーの素性（adapter 層の関心事）。
#[derive(Debug, Clone, PartialEq)]
pub struct SensorIdentity {
    pub manufacturer: &'static str,
    pub ic_part_number: &'static str,
    pub sensor_type: SensorType,
    pub connection_type: ConnectionType,
}
