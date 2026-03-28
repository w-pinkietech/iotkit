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

    let args: Vec<String> = std::env::args().collect();
    let config = match config::load(&args) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to load config");
            std::process::exit(1);
        }
    };

    tracing::info!(source = ?config.config_source, "config loaded");
    tracing::info!(
        db_path = %config.db_path,
        bravepi_enabled = config.bravepi.is_some(),
        bravepi_port = config.bravepi.as_ref().map(|b| b.port.as_str()).unwrap_or("N/A"),
        rpi_local_enabled = config.rpi_local.is_some(),
        rpi_local_bus_path = config.rpi_local.as_ref().map(|r| r.bus_path.as_str()).unwrap_or("N/A"),
        rpi_local_poll_interval_ms = config.rpi_local.as_ref().map(|r| r.poll_interval_ms).unwrap_or(0),
        "effective config"
    );

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(run(config));
}

async fn run(config: config::GatewayConfig) {
    let engine = Engine::new();
    let mut host = AdapterHost::new();

    // BravePI mainboard adapter
    if let Some(bp) = &config.bravepi {
        match bravepi_mainboard_adapter::task::start(bp.port.clone()) {
            Ok(h) => {
                tracing::info!(adapter_id = %h.id, port = %bp.port, "BravePI mainboard adapter started");
                let parts = h.into_parts();
                host.register(
                    parts.id,
                    parts.event_rx,
                    {
                        let sh = parts.shutdown;
                        move || Box::pin(async move { sh.shutdown().await })
                    },
                )
                .expect("duplicate adapter ID");
            }
            Err(e) => {
                tracing::error!(error = %e, port = %bp.port, "Failed to start BravePI mainboard adapter");
                std::process::exit(1);
            }
        }
    } else {
        tracing::info!("BravePI mainboard adapter disabled");
    }

    // RPi local I2C adapter
    if let Some(rpi) = &config.rpi_local {
        let targets = hardcoded_rpi_local_targets();
        tracing::info!(
            bus_path = %rpi.bus_path,
            poll_interval_ms = rpi.poll_interval_ms,
            target_count = targets.len(),
            "rpi-local using hardcoded targets: MCP9600@0x60(K-type), OPT3001@0x44 (until auto-detection #35)"
        );
        let adapter_config = rpi_local_adapter::RpiLocalConfig {
            bus_path: rpi.bus_path.clone(),
            poll_interval_ms: rpi.poll_interval_ms,
            targets,
        };
        // Preflight: catch driver-level validation before spawning background tasks
        if let Err(e) = rpi_local_adapter::validate(&adapter_config) {
            tracing::error!(error = %e, "RPi local adapter config validation failed");
            std::process::exit(1);
        }
        match rpi_local_adapter::start(adapter_config) {
            Ok(rpi_handle) => {
                tracing::info!(adapter_id = %rpi_handle.id, "RPi local adapter started");
                let parts = rpi_handle.into_parts();
                host.register(
                    parts.id,
                    parts.event_rx,
                    {
                        let sh = parts.shutdown;
                        move || Box::pin(async move { sh.shutdown().await })
                    },
                )
                .expect("duplicate adapter ID");
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    bus_path = %rpi.bus_path,
                    "Failed to start RPi local adapter"
                );
                std::process::exit(1);
            }
        }
    } else {
        tracing::info!("RPi local adapter disabled");
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

/// Hardcoded sensor targets for the v1 RPi4B hardware profile.
///
/// Deployment inventory -- lives in the gateway composition root, not the
/// adapter crate. Replaced by #35 auto-detection or #23 device-config-service.
fn hardcoded_rpi_local_targets() -> Vec<rpi_local_adapter::RpiLocalTarget> {
    use rpi_local_adapter::ThermocoupleType;
    vec![
        rpi_local_adapter::RpiLocalTarget::MCP9600 {
            address: 0x60,
            thermocouple_type: ThermocoupleType::K,
        },
        rpi_local_adapter::RpiLocalTarget::OPT3001 {
            address: 0x44,
        },
    ]
}
