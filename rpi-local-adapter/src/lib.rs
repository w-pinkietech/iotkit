//! rpi-local-adapter: RPi ローカル直結 hardware の adapter。
//! v1 は I2C slice のみ。

pub mod config;

pub use config::{RpiLocalConfig, SensorKind, SensorTarget, ThermocoupleType};
