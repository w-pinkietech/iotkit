use iotkit_adapter_runner::MqttConfig;
use rpi_local_adapter::{RpiLocalConfig, RpiLocalTarget, ThermocoupleType};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct StandaloneConfig {
    pub adapter_id: String,
    pub mqtt: MqttToml,
    pub adapter: AdapterToml,
}

#[derive(Debug, Deserialize)]
pub struct MqttToml {
    pub broker_url: String,
    pub client_id: Option<String>,
    pub keepalive_secs: Option<u32>,
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
#[serde(tag = "driver")]
pub enum TargetToml {
    #[serde(rename = "mcp9600")]
    Mcp9600 {
        address: u8,
        thermocouple_type: Option<String>,
    },
    #[serde(rename = "opt3001")]
    Opt3001 { address: u8 },
}

impl StandaloneConfig {
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read config file {}: {e}", path.display()))?;
        let config: StandaloneConfig =
            toml::from_str(&contents).map_err(|e| format!("failed to parse config: {e}"))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.adapter_id.is_empty() {
            return Err("adapter_id must not be empty".into());
        }
        if !self.mqtt.broker_url.starts_with("mqtt://")
            && !self.mqtt.broker_url.starts_with("mqtts://")
        {
            return Err("mqtt.broker_url must start with mqtt:// or mqtts://".into());
        }
        if self.mqtt.broker_url.starts_with("mqtts://") && self.mqtt.ca_path.is_none() {
            return Err("mqtts:// requires mqtt.ca_path".into());
        }
        if let Some(k) = self.mqtt.keepalive_secs {
            if k == 0 {
                return Err("mqtt.keepalive_secs must be > 0".into());
            }
        }
        if self.adapter.targets.is_empty() {
            return Err("adapter.targets must not be empty".into());
        }
        // Validate thermocouple types (Codex fix #7)
        for target in &self.adapter.targets {
            if let TargetToml::Mcp9600 {
                thermocouple_type: Some(tc),
                ..
            } = target
            {
                parse_thermocouple_type(tc)?;
            }
        }
        Ok(())
    }

    pub fn to_mqtt_config(&self) -> MqttConfig {
        MqttConfig {
            broker_url: self.mqtt.broker_url.clone(),
            client_id: self.mqtt.client_id.clone(),
            keepalive_secs: self.mqtt.keepalive_secs,
            ca_path: self.mqtt.ca_path.as_ref().map(PathBuf::from),
            client_cert_path: self.mqtt.client_cert_path.as_ref().map(PathBuf::from),
            client_key_path: self.mqtt.client_key_path.as_ref().map(PathBuf::from),
        }
    }

    pub fn to_rpi_local_config(&self) -> Result<RpiLocalConfig, String> {
        let targets = self
            .adapter
            .targets
            .iter()
            .map(|t| match t {
                TargetToml::Mcp9600 {
                    address,
                    thermocouple_type,
                } => {
                    let tc = match thermocouple_type.as_deref() {
                        Some(s) => parse_thermocouple_type(s)?,
                        None => ThermocoupleType::K, // default
                    };
                    Ok(RpiLocalTarget::MCP9600 {
                        address: *address,
                        thermocouple_type: tc,
                    })
                }
                TargetToml::Opt3001 { address } => {
                    Ok(RpiLocalTarget::OPT3001 { address: *address })
                }
            })
            .collect::<Result<Vec<_>, String>>()?;

        Ok(RpiLocalConfig {
            bus_path: self.adapter.bus_path.clone(),
            poll_interval_ms: self.adapter.poll_interval_ms,
            targets,
        })
    }
}

/// Parse a thermocouple type string. Returns error for unknown types (Codex fix #7).
fn parse_thermocouple_type(s: &str) -> Result<ThermocoupleType, String> {
    match s {
        "K" => Ok(ThermocoupleType::K),
        "J" => Ok(ThermocoupleType::J),
        "T" => Ok(ThermocoupleType::T),
        "N" => Ok(ThermocoupleType::N),
        "S" => Ok(ThermocoupleType::S),
        "E" => Ok(ThermocoupleType::E),
        "B" => Ok(ThermocoupleType::B),
        "R" => Ok(ThermocoupleType::R),
        other => Err(format!(
            "unknown thermocouple_type '{}': must be one of K, J, T, N, S, E, B, R",
            other
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_config() {
        let toml_str = r#"
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

[[adapter.targets]]
driver = "opt3001"
address = 68
"#;
        let config: StandaloneConfig = toml::from_str(toml_str).unwrap();
        assert!(config.validate().is_ok());
        assert_eq!(config.adapter_id, "rpi-local:default");
        assert_eq!(config.adapter.targets.len(), 2);
    }

    #[test]
    fn validate_empty_adapter_id() {
        let toml_str = r#"
adapter_id = ""
[mqtt]
broker_url = "mqtt://localhost:1883"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
[[adapter.targets]]
driver = "opt3001"
address = 68
"#;
        let config: StandaloneConfig = toml::from_str(toml_str).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_invalid_broker_url() {
        let toml_str = r#"
adapter_id = "test"
[mqtt]
broker_url = "http://localhost"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
[[adapter.targets]]
driver = "opt3001"
address = 68
"#;
        let config: StandaloneConfig = toml::from_str(toml_str).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_mqtts_requires_ca() {
        let toml_str = r#"
adapter_id = "test"
[mqtt]
broker_url = "mqtts://broker.example.com"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
[[adapter.targets]]
driver = "opt3001"
address = 68
"#;
        let config: StandaloneConfig = toml::from_str(toml_str).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_empty_targets() {
        let toml_str = r#"
adapter_id = "test"
[mqtt]
broker_url = "mqtt://localhost:1883"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
"#;
        // Missing targets should fail parse or validation
        let result: Result<StandaloneConfig, _> = toml::from_str(toml_str);
        assert!(result.is_err() || result.unwrap().validate().is_err());
    }

    #[test]
    fn validate_unknown_thermocouple_type_returns_error() {
        let toml_str = r#"
adapter_id = "test"
[mqtt]
broker_url = "mqtt://localhost:1883"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
[[adapter.targets]]
driver = "mcp9600"
address = 96
thermocouple_type = "X"
"#;
        let config: StandaloneConfig = toml::from_str(toml_str).unwrap();
        let err = config.validate().unwrap_err();
        assert!(
            err.contains("unknown thermocouple_type"),
            "expected thermocouple validation error, got: {}",
            err
        );
    }

    #[test]
    fn to_rpi_local_config_default_thermocouple() {
        let toml_str = r#"
adapter_id = "test"
[mqtt]
broker_url = "mqtt://localhost:1883"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
[[adapter.targets]]
driver = "mcp9600"
address = 96
"#;
        let config: StandaloneConfig = toml::from_str(toml_str).unwrap();
        let rpi_config = config.to_rpi_local_config().unwrap();
        match &rpi_config.targets[0] {
            RpiLocalTarget::MCP9600 {
                thermocouple_type, ..
            } => {
                assert_eq!(*thermocouple_type, ThermocoupleType::K);
            }
            _ => panic!("expected MCP9600"),
        }
    }

    #[test]
    fn to_mqtt_config_maps_fields() {
        let toml_str = r#"
adapter_id = "test"
[mqtt]
broker_url = "mqtt://localhost:1883"
keepalive_secs = 60
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
[[adapter.targets]]
driver = "opt3001"
address = 68
"#;
        let config: StandaloneConfig = toml::from_str(toml_str).unwrap();
        let mqtt = config.to_mqtt_config();
        assert_eq!(mqtt.broker_url, "mqtt://localhost:1883");
        assert_eq!(mqtt.keepalive_secs, Some(60));
    }
}
