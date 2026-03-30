use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub adapter_id: String,
    pub mqtt: MqttToml,
    pub adapter: AdapterToml,
}

#[derive(Debug, Deserialize)]
pub struct MqttToml {
    pub broker_url: String,
    pub client_id: Option<String>,
    pub keepalive_secs: Option<u16>,
    pub ca_path: Option<String>,
    pub client_cert_path: Option<String>,
    pub client_key_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AdapterToml {
    pub bus_path: String,
    pub poll_interval_ms: u64,
    pub targets: Vec<TargetToml>,
}

#[derive(Debug, Deserialize)]
pub struct TargetToml {
    pub driver: String,
    pub address: u8,
    pub thermocouple_type: Option<String>,
}

#[derive(Debug)]
pub struct ValidatedConfig {
    pub adapter_id: String,
    pub mqtt: MqttToml,
    pub adapter: AdapterToml,
    pub host: String,
    pub port: u16,
    pub tls: bool,
}

/// Parse broker URL for config validation.
///
/// Note: similar logic exists in `iotkit_adapter_runner::mqtt_client::parse_broker_url`.
/// This version performs stricter validation (rejects path/query/fragment) because it runs
/// at config-load time. The runner version is only used for MQTT connection setup.
pub fn parse_broker_url(raw: &str) -> Result<(String, u16, bool), String> {
    let (substituted, default_port, tls) = if let Some(rest) = raw.strip_prefix("mqtts://") {
        (format!("https://{rest}"), 8883u16, true)
    } else if let Some(rest) = raw.strip_prefix("mqtt://") {
        (format!("http://{rest}"), 1883u16, false)
    } else {
        return Err(format!(
            "config error: mqtt.broker_url: scheme must be \"mqtt\" or \"mqtts\", got \"{raw}\""
        ));
    };

    let parsed = url::Url::parse(&substituted)
        .map_err(|e| format!("config error: mqtt.broker_url: invalid URL: {e}"))?;

    let host = parsed
        .host_str()
        .ok_or("config error: mqtt.broker_url: host must not be empty")?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();

    if host.is_empty() {
        return Err("config error: mqtt.broker_url: host must not be empty".into());
    }

    let path = parsed.path();
    if path != "" && path != "/" {
        return Err(
            "config error: mqtt.broker_url: must not contain path, query, or fragment components"
                .into(),
        );
    }
    if parsed.query().is_some() {
        return Err(
            "config error: mqtt.broker_url: must not contain path, query, or fragment components"
                .into(),
        );
    }
    if parsed.fragment().is_some() {
        return Err(
            "config error: mqtt.broker_url: must not contain path, query, or fragment components"
                .into(),
        );
    }

    let port = parsed.port().unwrap_or(default_port);
    Ok((host, port, tls))
}

/// Phase 1 (serde) + Phase 2 (cross-field validation).
pub fn parse_and_validate(toml_str: &str) -> Result<ValidatedConfig, String> {
    // Phase 1: serde parse (fail-fast)
    let config: Config =
        toml::from_str(toml_str).map_err(|e| format!("config error: {e}"))?;

    // Phase 2: cross-field validation (collect all errors)
    let mut errors = Vec::new();

    // adapter_id
    if config.adapter_id.trim().is_empty() {
        errors.push("config error: adapter_id: must not be empty".to_string());
    }

    // broker_url
    let url_result = if config.mqtt.broker_url.is_empty() {
        errors.push("config error: mqtt.broker_url: must not be empty".to_string());
        None
    } else {
        match parse_broker_url(&config.mqtt.broker_url) {
            Ok((host, port, tls)) => Some((host, port, tls)),
            Err(e) => {
                errors.push(e);
                None
            }
        }
    };

    let tls = url_result.as_ref().map(|(_, _, t)| *t).unwrap_or(false);

    // TLS field rules
    if !tls {
        if config.mqtt.ca_path.is_some() {
            errors.push(
                "config error: mqtt.ca_path: must not be set when broker_url uses mqtt:// (non-TLS)"
                    .into(),
            );
        }
        if config.mqtt.client_cert_path.is_some() {
            errors.push(
                "config error: mqtt.client_cert_path: must not be set when broker_url uses mqtt:// (non-TLS)"
                    .into(),
            );
        }
        if config.mqtt.client_key_path.is_some() {
            errors.push(
                "config error: mqtt.client_key_path: must not be set when broker_url uses mqtt:// (non-TLS)"
                    .into(),
            );
        }
    } else if config.mqtt.ca_path.is_none() {
        errors.push(
            "config error: mqtt.ca_path: required when broker_url uses mqtts://".into(),
        );
    }

    // TLS file existence checks (pre-flight validation per spec 4.2)
    if let Some(ref path) = config.mqtt.ca_path {
        if !std::path::Path::new(path).exists() {
            errors.push(format!("config error: mqtt.ca_path: file not found: {path}"));
        }
    }
    if let Some(ref path) = config.mqtt.client_cert_path {
        if !std::path::Path::new(path).exists() {
            errors.push(format!(
                "config error: mqtt.client_cert_path: file not found: {path}"
            ));
        }
    }
    if let Some(ref path) = config.mqtt.client_key_path {
        if !std::path::Path::new(path).exists() {
            errors.push(format!(
                "config error: mqtt.client_key_path: file not found: {path}"
            ));
        }
    }

    // Cert/key pairing
    match (&config.mqtt.client_cert_path, &config.mqtt.client_key_path) {
        (Some(_), None) => errors.push(
            "config error: mqtt.client_key_path: must be set when mqtt.client_cert_path is set"
                .into(),
        ),
        (None, Some(_)) => errors.push(
            "config error: mqtt.client_cert_path: must be set when mqtt.client_key_path is set"
                .into(),
        ),
        _ => {}
    }

    // keepalive_secs
    if config.mqtt.keepalive_secs == Some(0) {
        errors.push("config error: mqtt.keepalive_secs: must be >= 1, got 0".into());
    }

    // client_id
    if config.mqtt.client_id.as_deref() == Some("") {
        errors.push("config error: mqtt.client_id: must not be empty if specified".into());
    }

    // adapter.bus_path
    if config.adapter.bus_path.trim().is_empty() {
        errors.push("config error: adapter.bus_path: must not be empty".into());
    }

    // adapter.poll_interval_ms
    if config.adapter.poll_interval_ms == 0 {
        errors.push("config error: adapter.poll_interval_ms: must be >= 1, got 0".into());
    }

    // adapter.targets
    if config.adapter.targets.is_empty() {
        errors.push("config error: adapter.targets: must contain at least one target".into());
    }

    // Per-target validation
    let known_drivers = ["mcp9600", "opt3001"];
    let mut seen_addresses: std::collections::HashMap<u8, usize> =
        std::collections::HashMap::new();

    for (i, target) in config.adapter.targets.iter().enumerate() {
        if target.driver.is_empty() {
            errors.push(format!(
                "config error: adapter.targets[{i}].driver: must not be empty"
            ));
        } else if !known_drivers.contains(&target.driver.as_str()) {
            errors.push(format!(
                "config error: adapter.targets[{i}].driver: unknown driver \"{}\"; known drivers: {}",
                target.driver,
                known_drivers.join(", ")
            ));
        }

        if target.address < 0x08 || target.address > 0x77 {
            errors.push(format!(
                "config error: adapter.targets[{i}].address: I2C address 0x{:02x} out of valid range 0x08-0x77",
                target.address
            ));
        }

        // Duplicate address check
        if let Some(prev_idx) = seen_addresses.get(&target.address) {
            errors.push(format!(
                "config error: adapter.targets: duplicate I2C address 0x{:02x} at indices {} and {}",
                target.address, prev_idx, i
            ));
        } else {
            seen_addresses.insert(target.address, i);
        }

        // Driver-specific validation
        if target.driver == "mcp9600" {
            match &target.thermocouple_type {
                None => errors.push(format!(
                    "config error: adapter.targets[{i}].thermocouple_type: required for driver \"mcp9600\""
                )),
                Some(tc) => {
                    let valid = ["K", "J", "T", "N", "S", "E", "B", "R"];
                    if !valid.contains(&tc.as_str()) {
                        errors.push(format!(
                            "config error: adapter.targets[{i}].thermocouple_type: unknown type \"{tc}\"; valid values: {}",
                            valid.join(", ")
                        ));
                    }
                }
            }
        } else if target.thermocouple_type.is_some() {
            errors.push(format!(
                "config error: adapter.targets[{i}].thermocouple_type: not applicable to driver \"{}\"",
                target.driver
            ));
        }
    }

    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }

    let (host, port, tls) = url_result.unwrap();

    Ok(ValidatedConfig {
        adapter_id: config.adapter_id,
        mqtt: config.mqtt,
        adapter: config.adapter,
        host,
        port,
        tls,
    })
}

fn parse_thermocouple_type(
    s: &str,
) -> Option<rpi_local_adapter::ThermocoupleType> {
    use rpi_local_adapter::ThermocoupleType;
    match s {
        "K" => Some(ThermocoupleType::K),
        "J" => Some(ThermocoupleType::J),
        "T" => Some(ThermocoupleType::T),
        "N" => Some(ThermocoupleType::N),
        "S" => Some(ThermocoupleType::S),
        "E" => Some(ThermocoupleType::E),
        "B" => Some(ThermocoupleType::B),
        "R" => Some(ThermocoupleType::R),
        _ => None,
    }
}

impl ValidatedConfig {
    /// Load and validate from a TOML file.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read config file {}: {e}", path.display()))?;
        parse_and_validate(&contents)
    }

    /// Convert to runner's MqttConfig.
    pub fn to_mqtt_config(&self) -> iotkit_adapter_runner::MqttConfig {
        let client_id = self.mqtt.client_id.clone().unwrap_or_else(|| {
            format!(
                "iotkit-{}",
                percent_encoding::utf8_percent_encode(
                    &self.adapter_id,
                    percent_encoding::NON_ALPHANUMERIC,
                )
            )
        });

        iotkit_adapter_runner::MqttConfig {
            broker_url: self.mqtt.broker_url.clone(),
            client_id: Some(client_id),
            keepalive_secs: self.mqtt.keepalive_secs,
            ca_path: self.mqtt.ca_path.as_ref().map(PathBuf::from),
            client_cert_path: self.mqtt.client_cert_path.as_ref().map(PathBuf::from),
            client_key_path: self.mqtt.client_key_path.as_ref().map(PathBuf::from),
        }
    }

    /// Convert to adapter's RpiLocalConfig.
    pub fn to_rpi_local_config(&self) -> Result<rpi_local_adapter::RpiLocalConfig, String> {
        let mut targets = Vec::new();
        for target in &self.adapter.targets {
            let t = match target.driver.as_str() {
                "mcp9600" => {
                    let tc_str = target.thermocouple_type.as_ref()
                        .ok_or_else(|| format!("adapter.targets: mcp9600 missing thermocouple_type"))?;
                    let tc = parse_thermocouple_type(tc_str)
                        .ok_or_else(|| format!("invalid thermocouple type: {tc_str}"))?;
                    rpi_local_adapter::RpiLocalTarget::MCP9600 {
                        address: target.address,
                        thermocouple_type: tc,
                    }
                }
                "opt3001" => rpi_local_adapter::RpiLocalTarget::OPT3001 {
                    address: target.address,
                },
                other => return Err(format!("unknown driver: {other}")),
            };
            targets.push(t);
        }
        Ok(rpi_local_adapter::RpiLocalConfig {
            bus_path: self.adapter.bus_path.clone(),
            poll_interval_ms: self.adapter.poll_interval_ms,
            targets,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TOML: &str = r#"
adapter_id = "rpi-local:default"

[mqtt]
broker_url = "mqtt://localhost:1883"

[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000

[[adapter.targets]]
driver = "mcp9600"
address = 96
thermocouple_type = "K"
"#;

    #[test]
    fn valid_config_parses() {
        let config = parse_and_validate(VALID_TOML).unwrap();
        assert_eq!(config.adapter_id, "rpi-local:default");
    }

    #[test]
    fn empty_adapter_id_rejected() {
        let toml = VALID_TOML.replace("rpi-local:default", "  ");
        let result = parse_and_validate(&toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("adapter_id"),
            "error should mention adapter_id: {err}"
        );
    }

    #[test]
    fn invalid_scheme_rejected() {
        let toml = VALID_TOML.replace("mqtt://localhost:1883", "tcp://localhost:1883");
        let result = parse_and_validate(&toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("scheme"),
            "error should mention scheme: {err}"
        );
    }

    #[test]
    fn mqtt_with_ca_path_rejected() {
        let toml = r#"
adapter_id = "test"
[mqtt]
broker_url = "mqtt://localhost"
ca_path = "/ca.pem"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
[[adapter.targets]]
driver = "mcp9600"
address = 96
thermocouple_type = "K"
"#;
        let result = parse_and_validate(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ca_path"));
    }

    #[test]
    fn cert_without_key_rejected() {
        let toml = r#"
adapter_id = "test"
[mqtt]
broker_url = "mqtts://localhost"
ca_path = "/ca.pem"
client_cert_path = "/cert.pem"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
[[adapter.targets]]
driver = "mcp9600"
address = 96
thermocouple_type = "K"
"#;
        let result = parse_and_validate(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("client_key_path"));
    }

    #[test]
    fn keepalive_zero_rejected() {
        let toml = VALID_TOML.replace("[mqtt]", "[mqtt]\nkeepalive_secs = 0");
        let result = parse_and_validate(&toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("keepalive_secs"));
    }

    #[test]
    fn empty_targets_rejected() {
        let toml = r#"
adapter_id = "test"
[mqtt]
broker_url = "mqtt://localhost"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
targets = []
"#;
        let result = parse_and_validate(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("targets"));
    }

    #[test]
    fn unknown_driver_rejected() {
        let toml = VALID_TOML.replace("mcp9600", "unknown_driver");
        let result = parse_and_validate(&toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown driver"));
    }

    #[test]
    fn address_out_of_range_rejected() {
        let toml = VALID_TOML.replace("address = 96", "address = 7");
        let result = parse_and_validate(&toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("address"));
    }

    #[test]
    fn missing_thermocouple_type_for_mcp9600_rejected() {
        let toml = r#"
adapter_id = "test"
[mqtt]
broker_url = "mqtt://localhost"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
[[adapter.targets]]
driver = "mcp9600"
address = 96
"#;
        let result = parse_and_validate(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("thermocouple_type"));
    }

    #[test]
    fn thermocouple_on_opt3001_rejected() {
        let toml = r#"
adapter_id = "test"
[mqtt]
broker_url = "mqtt://localhost"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
[[adapter.targets]]
driver = "opt3001"
address = 68
thermocouple_type = "K"
"#;
        let result = parse_and_validate(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not applicable"));
    }

    #[test]
    fn duplicate_address_rejected() {
        let toml = r#"
adapter_id = "test"
[mqtt]
broker_url = "mqtt://localhost"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
[[adapter.targets]]
driver = "mcp9600"
address = 96
thermocouple_type = "K"
[[adapter.targets]]
driver = "opt3001"
address = 96
"#;
        let result = parse_and_validate(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("duplicate"));
    }

    #[test]
    fn multiple_errors_collected() {
        let toml = r#"
adapter_id = ""
[mqtt]
broker_url = "mqtt://localhost"
keepalive_secs = 0
[adapter]
bus_path = ""
poll_interval_ms = 0
[[adapter.targets]]
driver = "mcp9600"
address = 96
thermocouple_type = "K"
"#;
        let result = parse_and_validate(toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("adapter_id"),
            "missing adapter_id error: {err}"
        );
        assert!(
            err.contains("keepalive_secs"),
            "missing keepalive error: {err}"
        );
    }

    #[test]
    fn deterministic_client_id() {
        let config = parse_and_validate(VALID_TOML).unwrap();
        let mqtt = config.to_mqtt_config();
        let expected_client_id = format!(
            "iotkit-{}",
            percent_encoding::utf8_percent_encode(
                "rpi-local:default",
                percent_encoding::NON_ALPHANUMERIC,
            )
        );
        assert_eq!(mqtt.client_id, Some(expected_client_id));
    }

    #[test]
    fn default_port_mqtt() {
        let (_, port, _) = parse_broker_url("mqtt://localhost").unwrap();
        assert_eq!(port, 1883);
    }

    #[test]
    fn default_port_mqtts() {
        let (_, port, _) = parse_broker_url("mqtts://localhost").unwrap();
        assert_eq!(port, 8883);
    }

    #[test]
    fn broker_url_with_path_rejected() {
        let toml = VALID_TOML.replace("mqtt://localhost:1883", "mqtt://localhost/some/path");
        let result = parse_and_validate(&toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("path"));
    }

    #[test]
    fn empty_broker_url_rejected() {
        let toml = VALID_TOML.replace("mqtt://localhost:1883", "");
        let result = parse_and_validate(&toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("broker_url"));
    }

    #[test]
    fn broker_url_with_query_rejected() {
        let toml = VALID_TOML.replace("mqtt://localhost:1883", "mqtt://localhost?key=val");
        let result = parse_and_validate(&toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("query"));
    }

    #[test]
    fn broker_url_with_fragment_rejected() {
        let toml = VALID_TOML.replace("mqtt://localhost:1883", "mqtt://localhost#frag");
        let result = parse_and_validate(&toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("fragment"));
    }

    #[test]
    fn mqtts_without_ca_path_rejected() {
        let toml = VALID_TOML.replace("mqtt://localhost:1883", "mqtts://localhost");
        let result = parse_and_validate(&toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ca_path"));
    }

    /// Issue 14: Missing host in broker_url → error mentioning "host"
    #[test]
    fn missing_host_rejected() {
        let toml = VALID_TOML.replace("mqtt://localhost:1883", "mqtt://");
        let result = parse_and_validate(&toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_lowercase().contains("host"),
            "error should mention host: {err}"
        );
    }

    /// Issue 15: Empty client_id → error mentioning "client_id"
    #[test]
    fn empty_client_id_rejected() {
        let toml = VALID_TOML.replace("[mqtt]", "[mqtt]\nclient_id = \"\"");
        let result = parse_and_validate(&toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("client_id"),
            "error should mention client_id: {err}"
        );
    }

    /// Issue 16: Invalid thermocouple_type → error mentioning valid values
    #[test]
    fn invalid_thermocouple_type_rejected() {
        let toml = VALID_TOML.replace("thermocouple_type = \"K\"", "thermocouple_type = \"X\"");
        let result = parse_and_validate(&toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("thermocouple_type"),
            "error should mention thermocouple_type: {err}"
        );
        // Should list valid values
        assert!(
            err.contains("K") && err.contains("J") && err.contains("T"),
            "error should mention valid values: {err}"
        );
    }

    /// Issue 17: Long client_id (>128 chars) — parse_and_validate succeeds
    /// (warning is runtime, not validation-time)
    #[test]
    fn long_client_id_accepted() {
        let long_adapter_id = "a".repeat(200);
        let toml = VALID_TOML.replace("rpi-local:default", &long_adapter_id);
        let config = parse_and_validate(&toml).unwrap();
        let mqtt = config.to_mqtt_config();
        // client_id is auto-generated from adapter_id and will be >128 chars
        assert!(
            mqtt.client_id.as_ref().unwrap().len() > 128,
            "client_id should be >128 chars: {}",
            mqtt.client_id.as_ref().unwrap().len()
        );
    }

    /// Issue 18: Phase 1 fail-fast — malformed TOML (missing required field) gives
    /// a single serde error, not collect-all-errors
    #[test]
    fn malformed_toml_fail_fast() {
        // Missing [adapter] section entirely — serde should fail first
        let toml = r#"
adapter_id = "test"
[mqtt]
broker_url = "mqtt://localhost"
"#;
        let result = parse_and_validate(toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        // Should be a single serde error (Phase 1 fail-fast), not multiple Phase 2 errors.
        // The serde error message itself may contain newlines (TOML position info),
        // but there should be exactly one "config error:" prefix (not multiple collected errors).
        let error_count = err.matches("config error:").count();
        assert_eq!(
            error_count, 1,
            "should be a single error (Phase 1 fail-fast), got {error_count} errors: {err}"
        );
        // Should mention the missing field, not a Phase 2 validation error
        assert!(
            err.contains("missing field"),
            "should be a serde missing-field error: {err}"
        );
    }
}
