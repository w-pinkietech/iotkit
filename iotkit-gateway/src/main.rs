//! iotkit-gateway: composition root。
//! adapter を起動し、core/engine に event を渡す。

mod adapter_host;
mod config;

use adapter_host::{AdapterHost, AdapterHostEvent};
use iotkit_core_engine::Engine;
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
    let mut host = AdapterHost::new();

    // BravePI mainboard adapter — required: start failure is fatal.
    let bravepi = match bravepi_mainboard_adapter::task::start(port_path) {
        Ok(h) => {
            tracing::info!(adapter_id = %h.id, "BravePI mainboard adapter started");
            h
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to start BravePI mainboard adapter");
            std::process::exit(1);
        }
    };
    let bravepi_parts = bravepi.into_parts();
    host.register(
        bravepi_parts.id,
        bravepi_parts.event_rx,
        {
            let sh = bravepi_parts.shutdown;
            move || Box::pin(async move { sh.shutdown().await })
        },
    )
    .expect("duplicate adapter ID");

    // RPi local adapter — optional: disabled by default, enable with RPI_LOCAL_ENABLED=1.
    let rpi_local_enabled = std::env::var("RPI_LOCAL_ENABLED")
        .map(|v| v == "1")
        .unwrap_or(false);

    if rpi_local_enabled {
        match rpi_local_adapter::start(rpi_local_config()) {
            Ok(rpi) => {
                tracing::info!(adapter_id = %rpi.id, "RPi local adapter started");
                let rpi_parts = rpi.into_parts();
                host.register(
                    rpi_parts.id,
                    rpi_parts.event_rx,
                    {
                        let sh = rpi_parts.shutdown;
                        move || Box::pin(async move { sh.shutdown().await })
                    },
                )
                .expect("duplicate adapter ID");
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "Failed to start RPi local adapter (enabled but failed)"
                );
                std::process::exit(1);
            }
        }
    } else {
        tracing::info!("RPi local adapter disabled (set RPI_LOCAL_ENABLED=1 to enable)");
    }

    // Unified fan-in loop
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutdown signal received");
                break;
            }
            event = host.next_event() => {
                match event {
                    Some(AdapterHostEvent::Event(ev)) => {
                        tracing::debug!(
                            adapter = %ev.adapter_id,
                            event = ?ev.event,
                            "Adapter event"
                        );
                        engine.apply(ev).await;
                    }
                    Some(AdapterHostEvent::AdapterClosed(id)) => {
                        // During normal operation, an adapter closing is unexpected.
                        // During shutdown (after loop break), closures are expected
                        // and handled by shutdown_all().
                        tracing::warn!(
                            adapter = %id,
                            "Adapter channel closed unexpectedly"
                        );
                    }
                    None => {
                        tracing::info!("All adapter channels closed");
                        break;
                    }
                }
            }
        }
    }

    host.shutdown_all().await;

    let devices = engine.devices().await;
    tracing::info!(device_count = devices.len(), "Engine state at shutdown");
}
