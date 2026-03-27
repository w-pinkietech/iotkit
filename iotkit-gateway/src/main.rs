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

/// Hardcoded rpi-local-adapter config for v1.
/// Env vars for bus path and poll interval only — no target parsing DSL.
/// Config-driven target list is deferred to sub-project C (orchestrator).
fn rpi_local_config() -> rpi_local_adapter::RpiLocalConfig {
    use rpi_local_adapter::{SensorKind, SensorTarget, ThermocoupleType};

    let bus_path = std::env::var("RPI_LOCAL_I2C_BUS")
        .unwrap_or_else(|_| "/dev/i2c-1".to_string());

    let poll_interval_ms: u64 = std::env::var("RPI_LOCAL_POLL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);

    rpi_local_adapter::RpiLocalConfig {
        bus_path,
        poll_interval_ms,
        targets: vec![
            SensorTarget {
                address: 0x60,
                kind: SensorKind::MCP9600 {
                    thermocouple_type: ThermocoupleType::K,
                },
            },
            SensorTarget {
                address: 0x44,
                kind: SensorKind::OPT3001,
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

    // RPi local adapter is optional: start failure is a warning.
    let mut rpi_local = match rpi_local_adapter::start(rpi_local_config()) {
        Ok(h) => {
            tracing::info!(adapter_id = %h.id, "RPi local adapter started");
            Some(h)
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to start RPi local adapter, continuing without it");
            None
        }
    };

    // Track whether each adapter's channel is still open.
    // Handles are kept even after channel close for shutdown cleanup.
    let mut bravepi_open = true;
    let mut rpi_local_open = rpi_local.is_some();

    loop {
        tokio::select! {
            biased;

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
    if let Some(h) = rpi_local {
        if let Err(e) = h.shutdown().await {
            tracing::error!(error = %e, "RPi local adapter shutdown error");
        }
    }

    let devices = engine.devices().await;
    tracing::info!(device_count = devices.len(), "Engine state at shutdown");
}
