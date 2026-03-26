//! BravePI adapter — BravePI プロトコル固有の処理。
//! rpi4b-driver の transport / sensors を使い、BravePI 特有のマッピングを行う。
//!
//! `task` モジュールで async task として起動し、AdapterEvent channel で core と通信する。

pub mod task;
pub(crate) mod transport;

use std::collections::BTreeMap;
use iotkit_core_types::{ConnectionInfo, ConnectionKind, SensorType};
use rpi4b_transport::{DataBits, Parity, SerialConfig, StopBits};

/// BravePI adapter 内部の型安全な接続表現。
#[derive(Debug, Clone, PartialEq)]
pub enum BravepiConnection {
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

impl BravepiConnection {
    /// adapter 固有の型 → core の汎用型に変換。
    pub fn to_connection_info(&self) -> ConnectionInfo {
        match self {
            Self::Uart { port, transmitter_id } => ConnectionInfo {
                kind: ConnectionKind::Uart,
                parameters: BTreeMap::from([
                    ("port".into(), port.clone()),
                    ("transmitter_id".into(), transmitter_id.clone()),
                ]),
            },
            Self::I2c { bus, address } => ConnectionInfo {
                kind: ConnectionKind::I2c,
                parameters: BTreeMap::from([
                    ("bus".into(), bus.clone()),
                    ("address".into(), format!("0x{:02x}", address)),
                ]),
            },
            Self::Gpio { pin } => ConnectionInfo {
                kind: ConnectionKind::Gpio,
                parameters: BTreeMap::from([
                    ("pin".into(), format!("BCM{}", pin)),
                ]),
            },
        }
    }
}

/// BravePI UART 標準設定: 38400 8N1
pub fn serial_config() -> SerialConfig {
    SerialConfig {
        baud_rate: 38400,
        data_bits: DataBits::Eight,
        parity: Parity::None,
        stop_bits: StopBits::One,
    }
}

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
