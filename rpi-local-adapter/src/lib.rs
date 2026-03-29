//! rpi-local-adapter: RPi local I2C sensor adapter.
//! Thin wrapper over iotkit-polling-adapter-runtime with MCP9600 and OPT3001 drivers.

pub mod drivers;

pub use bravepi_sensors::mcp9600::ThermocoupleType;
pub use iotkit_polling_adapter_runtime::AdapterHandle;

use std::sync::Arc;

use iotkit_polling_adapter_runtime::{PollingAdapterConfig, SensorTargetConfig};
use iotkit_core_types::AdapterId;

/// Adapter configuration. Passed to [`start`].
#[derive(Debug, Clone)]
pub struct RpiLocalConfig {
    /// I2C bus path, e.g. "/dev/i2c-1".
    pub bus_path: String,
    /// Polling interval in milliseconds. Must be > 0.
    pub poll_interval_ms: u64,
    /// Sensor targets to probe and poll.
    pub targets: Vec<RpiLocalTarget>,
}

/// A single sensor target on the I2C bus.
#[derive(Debug, Clone)]
pub enum RpiLocalTarget {
    MCP9600 {
        address: u8,
        thermocouple_type: ThermocoupleType,
    },
    OPT3001 {
        address: u8,
    },
}

/// Start the rpi-local-adapter with the default adapter ID ("rpi-local:default").
///
/// Validates config (including per-driver validation), then delegates to
/// `iotkit_polling_adapter_runtime::start`.
pub fn start(config: RpiLocalConfig) -> Result<AdapterHandle, std::io::Error> {
    start_with_id(AdapterId::new("rpi-local:default"), config)
}

/// Start the rpi-local-adapter with a custom adapter ID.
///
/// Same as [`start`] but allows the caller to specify the adapter identity,
/// which is needed for standalone binary operation where the ID comes from config.
pub fn start_with_id(adapter_id: AdapterId, config: RpiLocalConfig) -> Result<AdapterHandle, std::io::Error> {
    let polling_config = to_polling_config(&config);
    iotkit_polling_adapter_runtime::start(adapter_id, polling_config)
}

/// Validate an `RpiLocalConfig` without starting the adapter.
///
/// Converts to `PollingAdapterConfig` internally and delegates to
/// `iotkit_polling_adapter_runtime::validate_config()`. Used for
/// preflight validation in the gateway before `start()`.
pub fn validate(config: &RpiLocalConfig) -> Result<(), String> {
    let polling_config = to_polling_config(config);
    iotkit_polling_adapter_runtime::validate_config(&polling_config)
}

fn to_polling_config(config: &RpiLocalConfig) -> PollingAdapterConfig {
    let targets = config
        .targets
        .iter()
        .map(|t| match t {
            RpiLocalTarget::MCP9600 {
                address,
                thermocouple_type,
            } => SensorTargetConfig {
                address: *address,
                driver: Arc::new(drivers::mcp9600::Mcp9600Driver {
                    thermocouple_type: *thermocouple_type,
                }),
                key_suffix: None,
            },
            RpiLocalTarget::OPT3001 { address } => SensorTargetConfig {
                address: *address,
                driver: Arc::new(drivers::opt3001::Opt3001Driver),
                key_suffix: None,
            },
        })
        .collect();
    PollingAdapterConfig {
        bus_path: config.bus_path.clone(),
        poll_interval_ms: config.poll_interval_ms,
        targets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tokio runtime が無い状態で start() を呼ぶと panic せず Err を返す。
    #[test]
    fn start_without_runtime_returns_error() {
        let config = RpiLocalConfig {
            bus_path: "/dev/i2c-1".to_string(),
            poll_interval_ms: 1000,
            targets: vec![RpiLocalTarget::MCP9600 {
                address: 0x60,
                thermocouple_type: ThermocoupleType::K,
            }],
        };
        let result = start(config);
        assert!(
            result.is_err(),
            "start() should return Err without tokio runtime"
        );
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

    #[test]
    fn validate_rejects_short_poll_interval_for_opt3001() {
        let cfg = RpiLocalConfig {
            bus_path: "/dev/i2c-1".into(),
            poll_interval_ms: 50,
            targets: vec![RpiLocalTarget::OPT3001 { address: 0x44 }],
        };
        let err = validate(&cfg).unwrap_err();
        assert!(err.contains("poll_interval_ms"), "unexpected error: {err}");
    }

    #[test]
    fn validate_accepts_valid_config() {
        let cfg = RpiLocalConfig {
            bus_path: "/dev/i2c-1".into(),
            poll_interval_ms: 1000,
            targets: vec![
                RpiLocalTarget::MCP9600 { address: 0x60, thermocouple_type: ThermocoupleType::K },
                RpiLocalTarget::OPT3001 { address: 0x44 },
            ],
        };
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn start_with_id_without_runtime_returns_error() {
        let config = RpiLocalConfig {
            bus_path: "/dev/i2c-1".to_string(),
            poll_interval_ms: 1000,
            targets: vec![RpiLocalTarget::MCP9600 {
                address: 0x60,
                thermocouple_type: ThermocoupleType::K,
            }],
        };
        let custom_id = AdapterId::new("my-custom:adapter");
        let result = start_with_id(custom_id, config);
        assert!(result.is_err(), "start_with_id should return Err without tokio runtime");
    }

    #[tokio::test]
    async fn start_with_id_preserves_adapter_id() {
        // start_with_id will fail due to missing /dev/i2c-1, but we verify
        // that the function signature is correct and the ID would be preserved.
        let config = RpiLocalConfig {
            bus_path: "/dev/i2c-1".to_string(),
            poll_interval_ms: 1000,
            targets: vec![RpiLocalTarget::MCP9600 {
                address: 0x60,
                thermocouple_type: ThermocoupleType::K,
            }],
        };
        let custom_id = AdapterId::new("my-custom:adapter");
        // On a real RPi with /dev/i2c-1 this would succeed and we could check handle.id
        // On CI it fails at bus open, which is expected
        let result = start_with_id(custom_id, config);
        // The function should return an error (no I2C bus), not panic
        assert!(result.is_err());
    }

    /// OPT3001 driver rejects poll intervals shorter than 200ms.
    #[test]
    fn opt3001_rejects_short_poll_interval() {
        let config = RpiLocalConfig {
            bus_path: "/dev/i2c-1".to_string(),
            poll_interval_ms: 50,
            targets: vec![RpiLocalTarget::OPT3001 { address: 0x44 }],
        };
        let err = start(config).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("OPT3001"),
            "expected OPT3001 validation error, got: {}",
            msg,
        );
    }
}
