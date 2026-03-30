mod config;

use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(name = "iotkit-rpi-local", version, about = "IoTKit RPi Local Adapter")]
struct Cli {
    /// Path to config file
    #[arg(long)]
    config: Option<PathBuf>,
}

fn resolve_config_path(explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        if !path.exists() {
            return Err(format!(
                "error: config file not found: {}",
                path.display()
            ));
        }
        return Ok(path);
    }

    let candidates = [
        PathBuf::from("./iotkit-rpi-local.toml"),
        PathBuf::from("/etc/iotkit/iotkit-rpi-local.toml"),
    ];

    for path in &candidates {
        match std::fs::metadata(path) {
            Ok(_) => return Ok(path.clone()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(format!(
                    "error: failed to read config file \"{}\": {}",
                    path.display(),
                    e
                ))
            }
        }
    }

    Err(format!(
        "error: no config file found; tried:\n  {}\nhint: use --config <path> to specify a config file explicitly",
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n  ")
    ))
}

fn redact_broker_url(raw: &str) -> String {
    let substituted = if let Some(rest) = raw.strip_prefix("mqtts://") {
        format!("https://{rest}")
    } else if let Some(rest) = raw.strip_prefix("mqtt://") {
        format!("http://{rest}")
    } else {
        return raw.to_string();
    };

    if let Ok(mut parsed) = url::Url::parse(&substituted) {
        if parsed.password().is_some() {
            let _ = parsed.set_password(Some("[REDACTED]"));
            let display = parsed.to_string();
            if raw.starts_with("mqtts://") {
                return display.replacen("https://", "mqtts://", 1);
            } else {
                return display.replacen("http://", "mqtt://", 1);
            }
        }
    }
    raw.to_string()
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // Resolve config path
    let config_path = match resolve_config_path(cli.config) {
        Ok(p) => p,
        Err(e) => {
            error!("{e}");
            return ExitCode::FAILURE;
        }
    };

    info!("loading config from {}", config_path.display());

    // Read and parse config
    let toml_str = match std::fs::read_to_string(&config_path) {
        Ok(s) => s,
        Err(e) => {
            error!("failed to read config file: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Phase 1 + 2: parse and validate
    let validated = match config::parse_and_validate(&toml_str) {
        Ok(v) => v,
        Err(e) => {
            error!("{e}");
            return ExitCode::FAILURE;
        }
    };

    // Phase 3: adapter/driver validation
    let rpi_config = match validated.to_rpi_local_config() {
        Ok(c) => c,
        Err(e) => {
            error!("config error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut phase3_errors = Vec::new();
    if let Err(e) = rpi_local_adapter::validate(&rpi_config) {
        phase3_errors.push(e);
    }
    if !phase3_errors.is_empty() {
        for e in &phase3_errors {
            error!("config error (phase 3): {e}");
        }
        return ExitCode::FAILURE;
    }

    let adapter_id = iotkit_core_types::AdapterId::new(&validated.adapter_id);
    let mqtt_config = validated.to_mqtt_config();

    // Warn if client_id exceeds 128 chars (MQTT 3.1.1 portability)
    if let Some(ref cid) = mqtt_config.client_id {
        if cid.len() > 128 {
            warn!(
                "MQTT client_id is {} chars (> 128); some brokers may reject this",
                cid.len()
            );
        }
    }

    info!(
        "connecting to broker {}",
        redact_broker_url(&validated.mqtt.broker_url)
    );

    // Create tokio runtime
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");

    rt.block_on(async move { run_async(adapter_id, mqtt_config, rpi_config).await })
}

async fn run_async(
    adapter_id: iotkit_core_types::AdapterId,
    mqtt_config: iotkit_adapter_runner::MqttConfig,
    rpi_config: rpi_local_adapter::RpiLocalConfig,
) -> ExitCode {
    // Install signal handlers
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .expect("failed to install SIGINT handler");
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("failed to install SIGTERM handler");

    // Start adapter
    let adapter_handle = match rpi_local_adapter::start_with_id(adapter_id.clone(), rpi_config) {
        Ok(h) => h,
        Err(e) => {
            error!("failed to start adapter: {e}");
            return ExitCode::FAILURE;
        }
    };

    let parts = adapter_handle.into_parts();
    let mut shutdown_handle = parts.shutdown;
    let event_rx = parts.event_rx;

    info!(adapter_id = %adapter_id, "adapter started");

    // Spawn runner
    let mut runner_join = tokio::spawn(iotkit_adapter_runner::run(
        adapter_id.clone(),
        mqtt_config,
        event_rx,
    ));

    let mut shutdown_initiated = false;

    // Event loop: wait for signal or runner exit
    tokio::select! {
        _ = sigint.recv() => {
            info!("SIGINT received, shutting down");
            shutdown_initiated = true;
        }
        _ = sigterm.recv() => {
            info!("SIGTERM received, shutting down");
            shutdown_initiated = true;
        }
        result = &mut runner_join => {
            if !shutdown_initiated {
                match result {
                    Ok(Ok(())) => {
                        error!("adapter died unexpectedly (event_rx closed without signal)");
                        return ExitCode::FAILURE;
                    }
                    Ok(Err(e)) => {
                        error!("runner error: {e}");
                        return ExitCode::FAILURE;
                    }
                    Err(e) => {
                        error!("runner panicked: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            }
        }
    }

    if !shutdown_initiated {
        return ExitCode::FAILURE;
    }

    // Shutdown sequence: stop adapter with 5s timeout
    let shutdown_fut = async {
        match tokio::time::timeout(Duration::from_secs(5), shutdown_handle.shutdown()).await {
            Ok(Ok(())) => info!("adapter stopped"),
            Ok(Err(e)) => warn!("adapter shutdown error: {e}"),
            Err(_) => {
                error!("adapter shutdown timed out after 5s");
                return ExitCode::FAILURE;
            }
        }

        // Wait for runner to finish
        match runner_join.await {
            Ok(Ok(())) => ExitCode::SUCCESS,
            Ok(Err(e)) => {
                error!("runner error during shutdown: {e}");
                ExitCode::FAILURE
            }
            Err(e) => {
                error!("runner panicked: {e}");
                ExitCode::FAILURE
            }
        }
    };

    // Race shutdown against 2nd signal
    tokio::select! {
        exit_code = shutdown_fut => exit_code,
        _ = sigint.recv() => {
            warn!("2nd signal received, forcing exit");
            std::process::exit(1);
        }
        _ = sigterm.recv() => {
            warn!("2nd signal received, forcing exit");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_broker_url_with_password() {
        let url = "mqtt://user:secret@host:1883";
        let redacted = redact_broker_url(url);
        assert!(
            redacted.contains("REDACTED"),
            "password not redacted: {redacted}"
        );
        assert!(!redacted.contains("secret"), "password leaked: {redacted}");
        assert!(
            redacted.contains("user"),
            "username should be preserved: {redacted}"
        );
    }

    #[test]
    fn redact_broker_url_without_password() {
        let url = "mqtt://host:1883";
        let redacted = redact_broker_url(url);
        assert_eq!(redacted, url, "URL without password should be unchanged");
    }

    #[test]
    fn redact_mqtts_url_with_password() {
        let url = "mqtts://user:pass@host:8883";
        let redacted = redact_broker_url(url);
        assert!(
            redacted.starts_with("mqtts://"),
            "scheme must be preserved: {redacted}"
        );
        assert!(
            redacted.contains("REDACTED"),
            "password not redacted: {redacted}"
        );
    }

    #[test]
    fn resolve_config_explicit_missing_file() {
        let result = resolve_config_path(Some(PathBuf::from("/nonexistent/path.toml")));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn resolve_config_no_candidates_found() {
        let result = resolve_config_path(None);
        if let Err(e) = result {
            assert!(e.contains("no config file found") || e.contains("hint"));
        }
    }
}
