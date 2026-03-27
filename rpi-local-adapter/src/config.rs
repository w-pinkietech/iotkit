//! Adapter configuration and validation.

/// Re-export ThermocoupleType so users don't depend on bravepi-sensors directly.
pub use bravepi_sensors::mcp9600::ThermocoupleType;

/// Adapter configuration. Passed to `start()`.
#[derive(Debug, Clone)]
pub struct RpiLocalConfig {
    /// I2C bus path, e.g. "/dev/i2c-1".
    pub bus_path: String,
    /// Polling interval in milliseconds. Must be > 0.
    pub poll_interval_ms: u64,
    /// Sensor targets to probe and poll.
    pub targets: Vec<SensorTarget>,
}

/// A single sensor target on the I2C bus.
#[derive(Debug, Clone)]
pub struct SensorTarget {
    /// 7-bit I2C address.
    pub address: u8,
    /// Sensor IC kind and its configuration.
    pub kind: SensorKind,
}

/// Sensor IC type with IC-specific configuration.
#[derive(Debug, Clone)]
pub enum SensorKind {
    MCP9600 {
        thermocouple_type: ThermocoupleType,
    },
    OPT3001,
}

/// Returns the IC name string for DeviceKey generation and duplicate detection.
pub fn sensor_ic_name(kind: &SensorKind) -> &'static str {
    match kind {
        SensorKind::MCP9600 { .. } => "mcp9600",
        SensorKind::OPT3001 => "opt3001",
    }
}

/// Validates config before starting the adapter.
pub fn validate_config(config: &RpiLocalConfig) -> Result<(), String> {
    if config.bus_path.is_empty() {
        return Err("bus_path must not be empty".to_string());
    }
    if config.poll_interval_ms == 0 {
        return Err("poll_interval_ms must be > 0".to_string());
    }

    let mut seen_addresses = std::collections::HashSet::new();
    for target in &config.targets {
        if !(0x08..=0x77).contains(&target.address) {
            return Err(format!(
                "address 0x{:02x} outside valid 7-bit I2C range (0x08..=0x77)",
                target.address,
            ));
        }
        if !seen_addresses.insert(target.address) {
            return Err(format!(
                "duplicate address 0x{:02x}: same bus cannot have two devices at one address",
                target.address,
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> RpiLocalConfig {
        RpiLocalConfig {
            bus_path: "/dev/i2c-1".to_string(),
            poll_interval_ms: 1000,
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

    #[test]
    fn valid_config_passes() {
        assert!(validate_config(&valid_config()).is_ok());
    }

    #[test]
    fn zero_poll_interval_is_rejected() {
        let mut config = valid_config();
        config.poll_interval_ms = 0;
        let err = validate_config(&config).unwrap_err();
        assert!(err.contains("poll_interval_ms"), "error: {}", err);
    }

    #[test]
    fn duplicate_address_is_rejected() {
        let mut config = valid_config();
        config.targets.push(SensorTarget {
            address: 0x60,
            kind: SensorKind::OPT3001,
        });
        let err = validate_config(&config).unwrap_err();
        assert!(err.contains("duplicate"), "error: {}", err);
    }

    #[test]
    fn address_out_of_range_is_rejected() {
        let mut config = valid_config();
        config.targets[0].address = 0x80;
        let err = validate_config(&config).unwrap_err();
        assert!(err.contains("outside valid"), "error: {}", err);
    }

    #[test]
    fn empty_bus_path_is_rejected() {
        let mut config = valid_config();
        config.bus_path = String::new();
        let err = validate_config(&config).unwrap_err();
        assert!(err.contains("bus_path"), "error: {}", err);
    }

    #[test]
    fn empty_targets_is_valid() {
        let mut config = valid_config();
        config.targets.clear();
        assert!(validate_config(&config).is_ok());
    }
}
