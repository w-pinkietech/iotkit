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
    #[arg(short, long, default_value = "iotkit-rpi-local.toml")]
    config: PathBuf,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // Try default paths if specified file doesn't exist
    let config_path = if cli.config.exists() {
        cli.config
    } else if PathBuf::from("/etc/iotkit/iotkit-rpi-local.toml").exists() {
        PathBuf::from("/etc/iotkit/iotkit-rpi-local.toml")
    } else {
        cli.config // will produce a clear error in load()
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

        // Run adapter runner (blocks until signal)
        if let Err(e) =
            iotkit_adapter_runner::run(adapter_id, mqtt_config, parts.event_rx).await
        {
            tracing::error!(error = %e, "adapter runner failed");
        }

        // Shutdown adapter
        if let Err(e) = parts.shutdown.shutdown().await {
            tracing::warn!(error = %e, "adapter shutdown error");
        }

        tracing::info!("shutdown complete");
    });
}
