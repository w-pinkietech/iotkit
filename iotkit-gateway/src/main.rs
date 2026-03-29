//! iotkit-gateway: composition root。
//! adapter を起動し、core/engine に event を渡す。

mod adapter_host;
mod config;

use std::time::{Duration, Instant};

use adapter_host::{AdapterHost, AdapterHostEvent};
use iotkit_core_engine::Engine;
use iotkit_core_types::AdapterEvent;
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
        rpi_local_enabled = config.rpi_local.is_some(),
        "effective config"
    );
    if let Some(bp) = &config.bravepi {
        tracing::info!(port = %bp.port, "bravepi config");
    }
    if let Some(rpi) = &config.rpi_local {
        tracing::info!(bus_path = %rpi.bus_path, poll_interval_ms = rpi.poll_interval_ms, "rpi_local config");
    }

    let mut all_migrations = Vec::from(iotkit_core_storage::MIGRATIONS);
    all_migrations.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
    let db = match iotkit_core_storage::init_db(std::path::Path::new(&config.db_path), &all_migrations) {
        Ok(handle) => handle,
        Err(e) => {
            tracing::error!(error = %e, db_path = %config.db_path, "failed to initialize database");
            std::process::exit(1);
        }
    };
    tracing::info!(db_path = %config.db_path, "database initialized");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(run(config, db));
}

async fn run(config: config::GatewayConfig, db: iotkit_core_storage::DbHandle) {
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
            tracing::error!(error = %e, bus_path = %rpi.bus_path, "RPi local adapter config validation failed");
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

    // State for rate-limited error logging
    let mut ts_write_errors: u64 = 0;
    let mut last_ts_err_log = Instant::now();

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

                        // Extract timeseries fields BEFORE engine consumes the event
                        let ts_data = match &ev.event {
                            AdapterEvent::SensorData {
                                device_key, reading, rssi, battery_pct, ingested_at,
                            } => Some((
                                ev.adapter_id.clone(),
                                device_key.clone(),
                                *ingested_at,
                                reading.sensor_type.clone(),
                                reading.values.clone(),
                                *rssi,
                                *battery_pct,
                            )),
                            _ => None,
                        };

                        engine.apply(ev).await;

                        if let Some((adapter_id, device_key, ingested_at, sensor_type, values, rssi, battery_pct)) = ts_data {
                            if let Err(e) = iotkit_core_timeseries::insert_reading(
                                &db, &adapter_id, &device_key, ingested_at, &sensor_type, &values, rssi, battery_pct,
                            ).await {
                                ts_write_errors += 1;
                                // Log immediately on first failure, then rate-limit subsequent errors
                                if ts_write_errors == 1 || last_ts_err_log.elapsed() > Duration::from_secs(30) {
                                    tracing::error!(
                                        error = %e,
                                        suppressed = ts_write_errors.saturating_sub(1),
                                        "timeseries write failed"
                                    );
                                    ts_write_errors = 0;
                                    last_ts_err_log = Instant::now();
                                }
                            }
                        }
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
