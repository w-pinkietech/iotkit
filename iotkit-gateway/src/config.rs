//! Bootstrap config: TOML parse → ENV merge → validated GatewayConfig.

use std::path::{Path, PathBuf};

use serde::Deserialize;

// ── Error ───────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid config: {0}")]
    Validation(String),
}

// ── Raw (serde target) ─────────────────────────────────

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawConfig {
    #[serde(default)]
    pub gateway: RawGatewayConfig,
    #[serde(default)]
    pub adapters: RawAdaptersConfig,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawGatewayConfig {
    pub db_path: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawAdaptersConfig {
    pub bravepi: Option<RawBravepiConfig>,
    pub rpi_local: Option<RawRpiLocalConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawBravepiConfig {
    pub enabled: Option<bool>,
    pub port: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawRpiLocalConfig {
    pub enabled: Option<bool>,
    pub bus_path: Option<String>,
    pub poll_interval_ms: Option<u64>,
}

// ── Resolved (validated) ────────────────────────────────

#[derive(Debug)]
pub struct GatewayConfig {
    pub config_source: ConfigSource,
    pub db_path: String,
    pub bravepi: Option<BravepiConfig>,
    pub rpi_local: Option<RpiLocalResolvedConfig>,
}

#[derive(Debug)]
pub enum ConfigSource {
    CliArg(PathBuf),
    EnvVar(PathBuf),
    ImplicitFile(PathBuf),
    DefaultsOnly,
}

#[derive(Debug)]
pub struct BravepiConfig {
    pub port: String,
}

#[derive(Debug)]
pub struct RpiLocalResolvedConfig {
    pub bus_path: String,
    pub poll_interval_ms: u64,
}

// ── Pipeline: load_raw ─────────────────────────────────

/// Load and parse a TOML config file.
///
/// If `path` is `Some` and `explicit` is true, the file MUST exist (error on missing).
/// If `path` is `Some` and `explicit` is false, a missing file silently returns defaults.
/// If `path` is `None`, returns defaults.
pub fn load_raw(path: Option<&Path>, explicit: bool) -> Result<RawConfig, ConfigError> {
    let Some(path) = path else {
        return Ok(RawConfig::default());
    };

    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let raw: RawConfig = toml::from_str(&contents)?;
            Ok(raw)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => {
            Ok(RawConfig::default())
        }
        Err(e) => Err(ConfigError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_toml() {
        let toml_str = r#"
[gateway]
db_path = "test.db"

[adapters.bravepi]
enabled = true
port = "/dev/ttyUSB0"

[adapters.rpi_local]
enabled = true
bus_path = "/dev/i2c-3"
poll_interval_ms = 500
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(raw.gateway.db_path.as_deref(), Some("test.db"));
        let bp = raw.adapters.bravepi.unwrap();
        assert_eq!(bp.enabled, Some(true));
        assert_eq!(bp.port.as_deref(), Some("/dev/ttyUSB0"));
        let rpi = raw.adapters.rpi_local.unwrap();
        assert_eq!(rpi.enabled, Some(true));
        assert_eq!(rpi.bus_path.as_deref(), Some("/dev/i2c-3"));
        assert_eq!(rpi.poll_interval_ms, Some(500));
    }

    #[test]
    fn parse_empty_toml_gives_defaults() {
        let raw: RawConfig = toml::from_str("").unwrap();
        assert!(raw.gateway.db_path.is_none());
        assert!(raw.adapters.bravepi.is_none());
        assert!(raw.adapters.rpi_local.is_none());
    }

    #[test]
    fn unknown_field_rejected() {
        let result: Result<RawConfig, _> = toml::from_str("[gateway]\nunknown = true");
        assert!(result.is_err());
    }

    #[test]
    fn unknown_adapter_rejected() {
        let result: Result<RawConfig, _> =
            toml::from_str("[adapters.nonexistent]\nfoo = \"bar\"");
        assert!(result.is_err());
    }

    // ── load_raw tests ─────────────────────────────────

    use std::io::Write as _;

    #[test]
    fn load_raw_missing_implicit_returns_defaults() {
        let raw = load_raw(Some(Path::new("/tmp/does-not-exist.toml")), false).unwrap();
        assert!(raw.gateway.db_path.is_none());
    }

    #[test]
    fn load_raw_missing_explicit_returns_error() {
        let result = load_raw(Some(Path::new("/tmp/does-not-exist.toml")), true);
        assert!(matches!(result, Err(ConfigError::Io(_))));
    }

    #[test]
    fn load_raw_valid_file() {
        let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
        write!(tmpfile, "[gateway]\ndb_path = \"from-file.db\"").unwrap();
        let raw = load_raw(Some(tmpfile.path()), true).unwrap();
        assert_eq!(raw.gateway.db_path.as_deref(), Some("from-file.db"));
    }

    #[test]
    fn load_raw_invalid_toml_returns_error() {
        let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
        write!(tmpfile, "not valid {{{{ toml").unwrap();
        let result = load_raw(Some(tmpfile.path()), true);
        assert!(matches!(result, Err(ConfigError::Toml(_))));
    }

    #[test]
    fn load_raw_none_path_returns_defaults() {
        let raw = load_raw(None, false).unwrap();
        assert!(raw.gateway.db_path.is_none());
    }
}
