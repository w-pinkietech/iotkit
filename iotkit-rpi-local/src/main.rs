mod config;

use clap::Parser;
use iotkit_core_types::AdapterId;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "iotkit-rpi-local",
    version,
    about = "Standalone I2C sensor adapter with MQTT output"
)]
struct Cli {
    /// Path to TOML config file
    #[arg(short, long)]
    config: Option<PathBuf>,
}

async fn wait_for_shutdown_signal() {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("failed to register SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("SIGINT received, shutting down");
        }
        _ = sigterm.recv() => {
            tracing::info!("SIGTERM received, shutting down");
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // Resolve config path: if explicitly provided via --config, use it as-is.
    // Otherwise try ./iotkit-rpi-local.toml, then /etc/iotkit/iotkit-rpi-local.toml.
    let config_path = if let Some(explicit) = cli.config {
        if !explicit.exists() {
            tracing::error!(path = %explicit.display(), "explicit config file not found");
            std::process::exit(1);
        }
        explicit
    } else {
        let local = PathBuf::from("iotkit-rpi-local.toml");
        let system = PathBuf::from("/etc/iotkit/iotkit-rpi-local.toml");
        if local.exists() {
            local
        } else if system.exists() {
            system
        } else {
            local // will produce a clear error in load()
        }
    };

    let config = match config::StandaloneConfig::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, path = %config_path.display(), "failed to load config");
            std::process::exit(1);
        }
    };

    tracing::info!(
        adapter_id = %config.adapter_id,
        broker = %config.mqtt.broker_url,
        bus_path = %config.adapter.bus_path,
        targets = config.adapter.targets.len(),
        "config loaded"
    );

    let adapter_id = AdapterId::new(&config.adapter_id);
    let mqtt_config = config.to_mqtt_config();
    let rpi_config = match config.to_rpi_local_config() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "adapter config conversion failed");
            std::process::exit(1);
        }
    };

    // Validate adapter config
    if let Err(e) = rpi_local_adapter::validate(&rpi_config) {
        tracing::error!(error = %e, "adapter config validation failed");
        std::process::exit(1);
    }

    // Create runtime - must exist before adapter start (requires tokio context)
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async {
        // Start adapter
        let handle = match rpi_local_adapter::start_with_id(adapter_id.clone(), rpi_config) {
            Ok(h) => {
                tracing::info!(adapter_id = %h.id, "adapter started");
                h
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to start adapter");
                std::process::exit(1);
            }
        };

        let parts = handle.into_parts();

        // Start runner in a background task (blocks until event_rx closes)
        let runner_handle = tokio::spawn(
            iotkit_adapter_runner::run(adapter_id, mqtt_config, parts.event_rx),
        );

        // Wait for shutdown signal
        wait_for_shutdown_signal().await;

        // 1. Shutdown adapter first (stops producing events, closes event_rx)
        if let Err(e) = parts.shutdown.shutdown().await {
            tracing::warn!(error = %e, "adapter shutdown error");
        }

        // 2. Small delay for in-flight events to flush through the publish loop
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // 3. Runner detects closed event_rx, publishes offline status, then exits
        match runner_handle.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::error!(error = %e, "adapter runner failed");
                std::process::exit(1);
            }
            Err(e) => {
                tracing::error!(error = %e, "runner task panicked");
                std::process::exit(1);
            }
        }

        tracing::info!("shutdown complete");
    });
}
