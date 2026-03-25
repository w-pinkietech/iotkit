//! BravePI PoC — adapter 相当。
//! transport + codec + sensors を組み合わせて動作確認する。

use iotkit_core_types::{SensorReading, SensorType};
use bravepi_adapter::{sensor_type_from_bravepi_raw, BravepiConnection};
use bravepi_codec::codec::{BravePiCodec, BravePiFrame};
use bravepi_sensors::{lis2duxs12, mcp3427, mcp9600, opt3001, sdp810, vl53l1x};
use rpi4b_transport::SerialTransport;

use std::time::Duration;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let port_path = "/dev/ttyAMA0";

    log::info!("BravePI Driver PoC");
    log::info!("Opening serial port: {} (38400 8N1)", port_path);

    let mut transport = match SerialTransport::open(port_path) {
        Ok(t) => t,
        Err(e) => {
            log::error!("Failed to open serial port: {}", e);
            std::process::exit(1);
        }
    };

    log::info!("Listening for uplink frames... (Ctrl+C to stop)");

    let mut codec = BravePiCodec::new();
    let mut buf = [0u8; 4096];

    loop {
        match transport.read(&mut buf, Duration::from_secs(5)) {
            Ok(0) => continue,
            Ok(n) => {
                codec.feed(&buf[..n]);

                while let Some(frame) = codec.decode() {
                    match frame {
                        BravePiFrame::Sensor(s) => {
                            // codec は生データを返す → adapter が sensor crate に振り分ける
                            let sensor_type = sensor_type_from_bravepi_raw(s.sensor_type_raw);
                            let conn = BravepiConnection::Uart {
                                port: port_path.to_string(),
                                transmitter_id: s.device_number.clone(),
                            };

                            let reading = match sensor_type {
                                SensorType::Temperature => mcp9600::from_uart_payload(&s.value_data),
                                SensorType::Illuminance => opt3001::from_uart_payload(&s.value_data),
                                SensorType::Adc => mcp3427::from_uart_payload(&s.value_data),
                                SensorType::Ranging => vl53l1x::from_uart_payload(&s.value_data),
                                SensorType::DifferentialPressure => sdp810::from_uart_payload(&s.value_data),
                                SensorType::Acceleration => lis2duxs12::from_uart_payload(&s.value_data),
                                SensorType::ContactInput | SensorType::ContactOutput => {
                                    let values: Vec<f64> = s.value_data.iter()
                                        .take(s.data_count as usize)
                                        .map(|&b| if b != 0 { 1.0 } else { 0.0 })
                                        .collect();
                                    SensorReading::new(sensor_type.clone(), values)
                                }
                                SensorType::Unknown(_) => {
                                    log::warn!("Unknown sensor type: {}", s.sensor_type_raw);
                                    continue;
                                }
                            };

                            let conn_info = conn.to_connection_info();
                            let id = match &sensor_type {
                                &SensorType::Temperature => Some(mcp9600::identity(conn_info)),
                                &SensorType::Illuminance => Some(opt3001::identity(conn_info)),
                                &SensorType::Adc => Some(mcp3427::identity(conn_info)),
                                &SensorType::Ranging => Some(vl53l1x::identity(conn_info)),
                                &SensorType::DifferentialPressure => Some(sdp810::identity(conn_info)),
                                &SensorType::Acceleration => Some(lis2duxs12::identity(conn_info)),
                                _ => None,
                            };

                            if let Some(id) = id {
                                log::info!(
                                    "SENSOR | manufacturer={} ic={} connection={} type={} rssi={} battery={} values={:?}",
                                    id.manufacturer, id.ic_part_number, id.connection,
                                    id.sensor_type, s.rssi, s.battery, reading.values,
                                );
                            } else {
                                log::info!(
                                    "SENSOR | type={} rssi={} battery={} values={:?}",
                                    sensor_type, s.rssi, s.battery, reading.values,
                                );
                            }
                        }
                        BravePiFrame::Config(cfg) => {
                            log::info!(
                                "CONFIG | device={} true_type={} fw={}",
                                cfg.device_number, cfg.true_sensor_type, cfg.firmware_version,
                            );
                        }
                        BravePiFrame::DecodeError { device_number, sensor_type_raw, reason } => {
                            log::warn!("DECODE ERROR | device={} type={} reason={}", device_number, sensor_type_raw, reason);
                        }
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => {
                log::error!("Read error: {}", e);
                std::process::exit(1);
            }
        }
    }
}
