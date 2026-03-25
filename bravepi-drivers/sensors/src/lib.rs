//! iotkit-sensors: sensor IC ごとの変換ドライバー
//!
//! 入力ソース（I2C 生値 / UART BravePI フレーム）を問わず、
//! 同じセンサー IC なら同じ SensorReading を返す。

pub mod reading;
pub mod opt3001;
pub mod mcp9600;
pub mod mcp3427;
pub mod vl53l1x;
pub mod sdp810;
pub mod lis2duxs12;
