//! BravePiFrame → supervision-free decoded event. Pure and stateless.

use iotkit_core_types::{DeviceKey, SensorIdentity, SensorReading};

use bravepi_codec::BravePiFrame;
use iotkit_sensor_drivers::UartSample;

use crate::BravepiConnection;
use crate::registry::lookup_handler;

#[derive(Debug)]
pub(crate) enum DecodedEvent {
    SensorData {
        device_key: DeviceKey,
        reading: SensorReading,
        rssi: Option<i16>,
        battery_pct: Option<u8>,
        observed_at: std::time::SystemTime,
    },
    AdapterError {
        device_key: Option<DeviceKey>,
        error: String,
    },
}

/// BravePiFrame を package-private decoded event に変換する。
/// SensorData フレームの場合は SensorIdentity も返す (DeviceDiscovered 用)。
/// None means the frame has no northbound observation.
pub(crate) fn frame_to_event(
    frame: BravePiFrame,
    port_path: &str,
) -> Option<(DecodedEvent, Option<SensorIdentity>)> {
    match frame {
        BravePiFrame::Sensor(s) => {
            let handler = lookup_handler(s.sensor_type_raw).or_else(|| {
                tracing::warn!(raw = s.sensor_type_raw, "Unknown sensor type, skipping");
                None
            })?;

            let transmitter_id = s.device_number.clone();
            let device_key = DeviceKey::new(format!(
                "bravepi-mainboard:{}:{}",
                transmitter_id, handler.key_suffix
            ));

            let conn_info = BravepiConnection::Uart {
                port: port_path.to_string(),
                transmitter_id,
            }
            .to_connection_info();

            let sample = UartSample {
                payload: &s.value_data,
                data_count: s.data_count,
            };
            let reading = (handler.decode_uart)(sample);
            let identity = (handler.identity)(conn_info);

            let event = DecodedEvent::SensorData {
                device_key,
                reading,
                rssi: Some(s.rssi as i16),
                battery_pct: Some(s.battery),
                observed_at: std::time::SystemTime::now(),
            };

            Some((event, Some(identity)))
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
        } => {
            let device_key = if device_number == "unknown" {
                None
            } else {
                lookup_handler(sensor_type_raw).map(|h| {
                    DeviceKey::new(format!(
                        "bravepi-mainboard:{}:{}",
                        device_number, h.key_suffix
                    ))
                })
            };
            Some((
                DecodedEvent::AdapterError {
                    device_key,
                    error: format!("Decode error (type={}): {}", sensor_type_raw, reason),
                },
                None,
            ))
        }
    }
}
