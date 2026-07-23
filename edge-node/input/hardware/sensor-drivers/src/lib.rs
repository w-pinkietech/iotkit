//! iotkit-sensor-drivers: sensor IC ごとの変換ドライバー
//!
//! 入力ソース（I2C 生値 / UART BravePI フレーム）を問わず、
//! 同じセンサー IC なら同じ SensorReading を返す。

use iotkit_core_types::{ConnectionInfo, SensorIdentity, SensorReading, SensorType};

/// UART デコードの入力。payload + data_count を含む。
pub struct UartSample<'a> {
    pub payload: &'a [u8],
    pub data_count: u16,
}

/// センサー/endpoint の decode と identity 生成をまとめた descriptor。
/// 各センサーモジュールが `pub const HANDLER: SensorHandler` として公開する。
pub struct SensorHandler {
    /// メタデータ用。decode_uart が返す SensorReading にも同じ値が入る。
    /// registry のテストや将来のフィルタリング用途で使う。
    pub sensor_type: SensorType,
    pub key_suffix: &'static str,
    pub identity: fn(ConnectionInfo) -> SensorIdentity,
    pub decode_uart: fn(UartSample<'_>) -> SensorReading,
}

pub mod contact;
pub mod lis2duxs12;
pub mod mcp3427;
pub mod mcp9600;
pub mod opt3001;
pub mod sdp810;
pub mod vl53l1x;
