//! BravePI adapter async task。
//!
//! シリアルポートからフレームを読み、AdapterEvent に変換して channel に送信する。
//! blocking serial I/O は専用スレッドで実行し、async 側と bytes channel で接続する。

use iotkit_core_types::{
    AdapterCommand, AdapterEvent, AdapterId, DeviceKey, SensorReading, SensorType,
};
use bravepi_codec::codec::{BravePiCodec, BravePiFrame};
use bravepi_sensors::{lis2duxs12, mcp3427, mcp9600, opt3001, sdp810, vl53l1x};
use rpi4b_transport::SerialTransport;
use tokio::sync::mpsc;

use std::time::Duration;

use crate::{sensor_type_from_bravepi_raw, serial_config};

/// adapter 起動結果。core はこの handle を使って adapter と通信する。
pub struct AdapterHandle {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub command_tx: mpsc::Sender<AdapterCommand>,
}

/// BravePI adapter を起動する。
///
/// 戻り値の `AdapterHandle` 経由で event を受信し、command を送信する。
/// serial read は専用スレッド、フレーム処理は tokio task で動作する。
pub fn start(port_path: String) -> Result<AdapterHandle, std::io::Error> {
    let config = serial_config();
    let transport = SerialTransport::open(&port_path, &config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(256);
    let (command_tx, command_rx) = mpsc::channel::<AdapterCommand>(32);

    // serial read 用の専用スレッド → async task へ raw bytes (またはエラー) を送る
    let (bytes_tx, bytes_rx) = mpsc::channel::<Result<Vec<u8>, String>>(64);
    let reader_port = port_path.clone();
    std::thread::Builder::new()
        .name(format!("bravepi-serial-{}", port_path))
        .spawn(move || serial_reader_thread(reader_port, transport, bytes_tx))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    let id = AdapterId(format!("bravepi:{}", port_path));

    tokio::spawn(event_loop(port_path, bytes_rx, event_tx, command_rx));

    Ok(AdapterHandle {
        id,
        event_rx,
        command_tx,
    })
}

/// 専用スレッド: serial port から読んで bytes channel に送る。
fn serial_reader_thread(
    port_path: String,
    mut transport: SerialTransport,
    bytes_tx: mpsc::Sender<Result<Vec<u8>, String>>,
) {
    tracing::info!(port = %port_path, "Serial reader thread started");
    let mut buf = [0u8; 4096];
    let timeout = Duration::from_millis(500);

    loop {
        if bytes_tx.is_closed() {
            tracing::info!("Bytes channel closed, reader thread exiting");
            return;
        }

        match transport.read(&mut buf, timeout) {
            Ok(0) => continue,
            Ok(n) => {
                if bytes_tx.blocking_send(Ok(buf[..n].to_vec())).is_err() {
                    tracing::info!("Bytes channel closed, reader thread exiting");
                    return;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => {
                let msg = format!("Serial read error on {}: {}", port_path, e);
                tracing::error!("{}", msg);
                let _ = bytes_tx.blocking_send(Err(msg));
                return;
            }
        }
    }
}

/// async task: raw bytes → codec → AdapterEvent。
async fn event_loop(
    port_path: String,
    mut bytes_rx: mpsc::Receiver<Result<Vec<u8>, String>>,
    event_tx: mpsc::Sender<AdapterEvent>,
    mut command_rx: mpsc::Receiver<AdapterCommand>,
) {
    tracing::info!(port = %port_path, "BravePI adapter event loop started");

    let mut codec = BravePiCodec::new();

    loop {
        tokio::select! {
            // Shutdown コマンドを優先処理する
            biased;

            cmd = command_rx.recv() => {
                match cmd {
                    Some(AdapterCommand::Shutdown) | None => {
                        tracing::info!("BravePI adapter shutting down");
                        return;
                    }
                }
            }
            result = bytes_rx.recv() => {
                match result {
                    Some(Ok(data)) => {
                        codec.feed(&data);
                        while let Some(frame) = codec.decode() {
                            if let Some(event) = frame_to_event(frame) {
                                if event_tx.send(event).await.is_err() {
                                    tracing::warn!("Event channel closed, shutting down");
                                    return;
                                }
                            }
                        }
                    }
                    Some(Err(error)) => {
                        tracing::error!(%error, "Serial reader reported error");
                        let _ = event_tx.send(AdapterEvent::AdapterError {
                            device_key: None,
                            error,
                        }).await;
                        return;
                    }
                    None => {
                        tracing::warn!("Serial reader thread exited without error report");
                        let _ = event_tx.send(AdapterEvent::AdapterError {
                            device_key: None,
                            error: format!("Serial reader thread for {} exited unexpectedly", port_path),
                        }).await;
                        return;
                    }
                }
            }
        }
    }
}

/// BravePiFrame を AdapterEvent に変換する。
/// None を返す場合、そのフレームは core に通知する必要がない。
pub fn frame_to_event(frame: BravePiFrame) -> Option<AdapterEvent> {
    match frame {
        BravePiFrame::Sensor(s) => {
            let sensor_type = sensor_type_from_bravepi_raw(s.sensor_type_raw);
            let device_key = DeviceKey(s.device_number.clone());

            let reading = match sensor_type {
                SensorType::Temperature => mcp9600::from_uart_payload(&s.value_data),
                SensorType::Illuminance => opt3001::from_uart_payload(&s.value_data),
                SensorType::Adc => mcp3427::from_uart_payload(&s.value_data),
                SensorType::Ranging => vl53l1x::from_uart_payload(&s.value_data),
                SensorType::DifferentialPressure => sdp810::from_uart_payload(&s.value_data),
                SensorType::Acceleration => lis2duxs12::from_uart_payload(&s.value_data),
                SensorType::ContactInput | SensorType::ContactOutput => {
                    let values: Vec<f64> = s
                        .value_data
                        .iter()
                        .take(s.data_count as usize)
                        .map(|&b| if b != 0 { 1.0 } else { 0.0 })
                        .collect();
                    SensorReading::new(sensor_type.clone(), values)
                }
                SensorType::Unknown(_) => {
                    tracing::warn!(raw = s.sensor_type_raw, "Unknown sensor type, skipping");
                    return None;
                }
            };

            Some(AdapterEvent::SensorData {
                device_key,
                reading,
                rssi: Some(s.rssi as i16),
                battery_pct: Some(s.battery),
            })
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
        } => Some(AdapterEvent::AdapterError {
            device_key: Some(DeviceKey(device_number)),
            error: format!("Decode error (type={}): {}", sensor_type_raw, reason),
        }),
    }
}
