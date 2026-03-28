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

/// Fully hardcoded rpi-local-adapter config for v1.
/// All config (bus, interval, targets) is fixed in code.
/// Config-driven setup is deferred to sub-project C (orchestrator).
fn rpi_local_config() -> rpi_local_adapter::RpiLocalConfig {
    use rpi_local_adapter::{RpiLocalTarget, ThermocoupleType};

    rpi_local_adapter::RpiLocalConfig {
        bus_path: "/dev/i2c-1".to_string(),
        poll_interval_ms: 1000,
        targets: vec![
            RpiLocalTarget::MCP9600 {
                address: 0x60,
                thermocouple_type: ThermocoupleType::K,
            },
            RpiLocalTarget::OPT3001 {
                address: 0x44,
            },
        ],
    }
}

async fn run(port_path: String) {
    let engine = Engine::new();

    // BravePI mainboard adapter is required: start failure is fatal.
    let mut bravepi = match bravepi_mainboard_adapter::task::start(port_path) {
        Ok(h) => {
            tracing::info!(adapter_id = %h.id, "BravePI mainboard adapter started");
            h
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to start BravePI mainboard adapter");
            std::process::exit(1);
        }
    };
    let bravepi_id = bravepi.id.clone();

    // RPi local adapter is optional: disabled by default, enable with RPI_LOCAL_ENABLED=1.
    // This avoids perpetual probe-failure warnings on hosts without I2C sensors.
    let rpi_local_enabled = std::env::var("RPI_LOCAL_ENABLED")
        .map(|v| v == "1")
        .unwrap_or(false);

    let mut rpi_local = if rpi_local_enabled {
        match rpi_local_adapter::start(rpi_local_config()) {
            Ok(h) => {
                tracing::info!(adapter_id = %h.id, "RPi local adapter started");
                Some(h)
            }
            Err(e) => {
                // When explicitly enabled, start failure is fatal — it indicates a
                // config/code bug, not transient hardware absence.
                tracing::error!(error = %e, "Failed to start RPi local adapter (enabled but failed)");
                std::process::exit(1);
            }
        }
    } else {
        tracing::info!("RPi local adapter disabled (set RPI_LOCAL_ENABLED=1 to enable)");
        None
    };

    // Track whether each adapter's channel is still open.
    // Handles are kept even after channel close for shutdown cleanup.
    let mut bravepi_open = true;
    let mut rpi_local_open = rpi_local.is_some();

    loop {
        tokio::select! {
            // No biased; — fair scheduling between adapter branches to prevent
            // one adapter's traffic from starving the other.

            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutdown signal received");
                break;
            }

            event = bravepi.event_rx.recv(), if bravepi_open => {
                match event {
                    Some(ev) => {
                        tracing::debug!(adapter = %bravepi_id, event = ?ev, "BravePI event");
                        engine.apply(EngineEvent {
                            adapter_id: bravepi_id.clone(),
                            event: ev,
                        }).await;
                    }
                    None => {
                        tracing::info!("BravePI adapter channel closed");
                        bravepi_open = false;
                        if !rpi_local_open {
                            tracing::info!("All adapter channels closed, exiting");
                            break;
                        }
                    }
                }
            }

            event = async {
                match rpi_local.as_mut() {
                    Some(h) => h.event_rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if rpi_local_open => {
                match event {
                    Some(ev) => {
                        let adapter_id = rpi_local.as_ref().unwrap().id.clone();
                        tracing::debug!(adapter = %adapter_id, event = ?ev, "RPi local event");
                        engine.apply(EngineEvent {
                            adapter_id,
                            event: ev,
                        }).await;
                    }
                    None => {
                        tracing::info!("RPi local adapter channel closed");
                        rpi_local_open = false;
                        if !bravepi_open {
                            tracing::info!("All adapter channels closed, exiting");
                            break;
                        }
                    }
                }
            }
        }
    }

    // Shutdown all adapters (handles kept even after channel close).
    if let Err(e) = bravepi.shutdown().await {
        tracing::error!(error = %e, "BravePI adapter shutdown error");
    }
    if let Some(mut h) = rpi_local {
        h.shutdown().await;
    }

    let devices = engine.devices().await;
    tracing::info!(device_count = devices.len(), "Engine state at shutdown");
}
