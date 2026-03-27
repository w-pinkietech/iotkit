//! rpi-local-adapter: RPi ローカル直結 hardware の adapter。
//! v1 は I2C slice のみ。

pub mod config;
mod polling_loop;
mod sensors;

pub use config::{RpiLocalConfig, SensorKind, SensorTarget, ThermocoupleType};

use iotkit_core_types::{AdapterCommand, AdapterEvent, AdapterId};
use tokio::sync::mpsc;

/// Adapter handle. Core uses this to receive events and send commands.
#[derive(Debug)]
pub struct AdapterHandle {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub command_tx: mpsc::Sender<AdapterCommand>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl AdapterHandle {
    /// Cooperative shutdown: close event_rx, send Shutdown, await task completion.
    /// Waits for any in-progress spawn_blocking (probe/poll cycle) to finish.
    pub async fn shutdown(mut self) -> Result<(), String> {
        self.event_rx.close();
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;
        if let Some(handle) = self.task_handle.take() {
            handle
                .await
                .map_err(|e| format!("polling_loop panicked: {}", e))?;
        }
        Ok(())
    }
}

/// Start the rpi-local-adapter.
///
/// Validates config first, then checks for tokio runtime, spawns the polling
/// loop task, and returns an AdapterHandle.
///
/// I2C read failures and non-target-specific task errors are reported as
/// `AdapterEvent::AdapterError`. Pending-target probe failures are logged
/// as warnings only (no event). Neither surfaces as a `start()` error.
pub fn start(config: RpiLocalConfig) -> Result<AdapterHandle, std::io::Error> {
    // Validate config before runtime check so config errors are always
    // reported as config errors, regardless of runtime presence.
    config::validate_config(&config).map_err(std::io::Error::other)?;

    let runtime_handle =
        tokio::runtime::Handle::try_current().map_err(std::io::Error::other)?;

    let id = AdapterId::new("rpi-local:default");
    let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(256);
    let (command_tx, command_rx) = mpsc::channel::<AdapterCommand>(32);

    let task_handle =
        runtime_handle.spawn(polling_loop::polling_loop(config, event_tx, command_rx));

    Ok(AdapterHandle {
        id,
        event_rx,
        command_tx,
        task_handle: Some(task_handle),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tokio runtime が無い状態で start() を呼ぶと panic せず Err を返す。
    /// #[tokio::test] ではなく plain #[test] で実行することで runtime 不在を保証する。
    #[test]
    fn start_without_runtime_returns_error() {
        let config = RpiLocalConfig {
            bus_path: "/dev/i2c-1".to_string(),
            poll_interval_ms: 1000,
            targets: vec![SensorTarget {
                address: 0x60,
                kind: SensorKind::MCP9600 {
                    thermocouple_type: ThermocoupleType::K,
                },
            }],
        };
        let result = start(config);
        assert!(result.is_err(), "start() should return Err without tokio runtime");
    }

    /// Config validation runs before runtime check, so this test verifies
    /// that invalid config produces a config-specific error message even
    /// without a tokio runtime.
    #[test]
    fn start_with_invalid_config_returns_config_error() {
        let config = RpiLocalConfig {
            bus_path: "/dev/i2c-1".to_string(),
            poll_interval_ms: 0,
            targets: vec![],
        };
        let err = start(config).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("poll_interval_ms"),
            "expected config validation error, got: {}",
            msg,
        );
    }
}
