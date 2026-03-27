//! iotkit-gateway: composition root。
//! adapter を起動し、core/engine に event を渡す。

use iotkit_core_engine::{Engine, EngineEvent};
use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let port_path =
        std::env::var("BRAVEPI_PORT").unwrap_or_else(|_| "/dev/ttyAMA0".to_string());

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(run(port_path));
}

async fn run(port_path: String) {
    let engine = Engine::new();

    let mut handle = match bravepi_mainboard_adapter::task::start(port_path) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "Failed to start BravePI mainboard adapter");
            std::process::exit(1);
        }
    };
    let adapter_id = handle.id.clone();
    tracing::info!(adapter_id = %adapter_id, "BravePI mainboard adapter started");

    loop {
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutdown signal received");
                if let Err(e) = handle.shutdown().await {
                    tracing::error!(error = %e, "Adapter shutdown error");
                }
                break;
            }
            event = handle.event_rx.recv() => {
                match event {
                    Some(ev) => {
                        tracing::debug!(event = ?ev, "Received adapter event");
                        engine.apply(EngineEvent {
                            adapter_id: adapter_id.clone(),
                            event: ev,
                        }).await;
                    }
                    None => {
                        tracing::info!("Adapter event channel closed");
                        break;
                    }
                }
            }
        }
    }

    let devices = engine.devices().await;
    tracing::info!(device_count = devices.len(), "Engine state at shutdown");
}
