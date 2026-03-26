//! BravePiFrame → AdapterEvent 変換。純粋関数、状態なし。

use iotkit_core_types::{
    AdapterEvent, DeviceKey, SensorIdentity, SensorReading, SensorType,
};

use bravepi_codec::codec::BravePiFrame;
use bravepi_sensors::{lis2duxs12, mcp3427, mcp9600, opt3001, sdp810, vl53l1x};

use crate::{sensor_type_from_bravepi_raw, BravepiConnection};

/// BravePiFrame を AdapterEvent に変換する。
/// SensorData フレームの場合は SensorIdentity も返す (DeviceDiscovered 用)。
/// None を返す場合、そのフレームは core に通知する必要がない。
pub fn frame_to_event(
    frame: BravePiFrame,
    port_path: &str,
) -> Option<(AdapterEvent, Option<SensorIdentity>)> {
    match frame {
        BravePiFrame::Sensor(s) => {
            let sensor_type = sensor_type_from_bravepi_raw(s.sensor_type_raw);
            let device_key = DeviceKey(s.device_number.clone());

            let conn_info = BravepiConnection::Uart {
                port: port_path.to_string(),
                transmitter_id: s.device_number.clone(),
            }
            .to_connection_info();

            // reading と identity を同じ match で生成。センサー追加時に1箇所だけ更新すればよい。
            let (reading, identity) = match sensor_type {
                SensorType::Temperature => (
                    mcp9600::from_uart_payload(&s.value_data),
                    Some(mcp9600::identity(conn_info)),
                ),
                SensorType::Illuminance => (
                    opt3001::from_uart_payload(&s.value_data),
                    Some(opt3001::identity(conn_info)),
                ),
                SensorType::Adc => (
                    mcp3427::from_uart_payload(&s.value_data),
                    Some(mcp3427::identity(conn_info)),
                ),
                SensorType::Ranging => (
                    vl53l1x::from_uart_payload(&s.value_data),
                    Some(vl53l1x::identity(conn_info)),
                ),
                SensorType::DifferentialPressure => (
                    sdp810::from_uart_payload(&s.value_data),
                    Some(sdp810::identity(conn_info)),
                ),
                SensorType::Acceleration => (
                    lis2duxs12::from_uart_payload(&s.value_data),
                    Some(lis2duxs12::identity(conn_info)),
                ),
                SensorType::ContactInput | SensorType::ContactOutput => {
                    let values: Vec<f64> = s
                        .value_data
                        .iter()
                        .take(s.data_count as usize)
                        .map(|&b| if b != 0 { 1.0 } else { 0.0 })
                        .collect();
                    (SensorReading::new(sensor_type.clone(), values, vec![]), None)
                }
                SensorType::Unknown(_) => {
                    tracing::warn!(raw = s.sensor_type_raw, "Unknown sensor type, skipping");
                    return None;
                }
            };

            let event = AdapterEvent::SensorData {
                device_key,
                reading,
                rssi: Some(s.rssi as i16),
                battery_pct: Some(s.battery),
            };

            Some((event, identity))
        }
        BravePiFrame::Config(cfg) => {
            tracing::info!(
                device = %cfg.device_number,
                sensor_type = cfg.true_sensor_type,
                firmware = %cfg.firmware_version,
                "Config frame received"
            );
            None
        }
        BravePiFrame::DecodeError {
            device_number,
            sensor_type_raw,
            reason,
        } => Some((
            AdapterEvent::AdapterError {
                device_key: Some(DeviceKey(device_number)),
                error: format!("Decode error (type={}): {}", sensor_type_raw, reason),
            },
            None,
        )),
    }
}
