//! Phase 1 PoC — channel ベースの adapter-core 境界検証。
//!
//! BravePI adapter を async task として起動し、
//! core 側は AdapterEvent を受信して表示するだけの最小ループ。

use bravepi_adapter::task;
use iotkit_core_types::AdapterEvent;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let port_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/dev/ttyAMA0".to_string());

    tracing::info!(port = %port_path, "Phase 1 PoC: channel-based adapter-core boundary");

    let mut handle = match task::start(port_path) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "Failed to start BravePI adapter");
            std::process::exit(1);
        }
    };

    tracing::info!(adapter_id = %handle.id, "Adapter started, listening for events...");

    // core 側の最小受信ループ
    while let Some(event) = handle.event_rx.recv().await {
        match &event {
            AdapterEvent::SensorData {
                device_key,
                reading,
                rssi,
                battery_pct,
            } => {
                tracing::info!(
                    device = %device_key,
                    sensor_type = %reading.sensor_type,
                    values = ?reading.values,
                    rssi = ?rssi,
                    battery = ?battery_pct,
                    "SensorData"
                );
            }
            AdapterEvent::DeviceDiscovered {
                device_key,
                identity,
            } => {
                tracing::info!(
                    device = %device_key,
                    manufacturer = %identity.manufacturer,
                    ic = %identity.ic_part_number,
                    "DeviceDiscovered"
                );
            }
            AdapterEvent::DeviceLost { device_key, reason } => {
                tracing::warn!(device = %device_key, reason = %reason, "DeviceLost");
            }
            AdapterEvent::AdapterError { device_key, error } => {
                tracing::error!(device = ?device_key, error = %error, "AdapterError");
            }
        }
    }

    tracing::info!("Event channel closed, exiting");
}
