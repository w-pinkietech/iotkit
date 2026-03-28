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

// ── Pipeline: apply_env ────────────────────────────────

fn parse_bool_env(var: &str, val: &str) -> Result<bool, ConfigError> {
    match val {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(ConfigError::Validation(format!(
            "invalid value for {var}: '{val}' (expected true/false/1/0)"
        ))),
    }
}

fn parse_u64_env(var: &str, val: &str) -> Result<u64, ConfigError> {
    val.parse::<u64>().map_err(|_| {
        ConfigError::Validation(format!(
            "invalid value for {var}: '{val}' (expected integer)"
        ))
    })
}

/// Apply ENV overrides to a `RawConfig`. Returns error on parse failure.
pub fn apply_env(raw: &mut RawConfig) -> Result<(), ConfigError> {
    if let Ok(val) = std::env::var("IOTKIT_DB_PATH") {
        raw.gateway.db_path = Some(val);
    }

    if let Ok(val) = std::env::var("BRAVEPI_ENABLED") {
        let bp = raw.adapters.bravepi.get_or_insert(RawBravepiConfig {
            enabled: None,
            port: None,
        });
        bp.enabled = Some(parse_bool_env("BRAVEPI_ENABLED", &val)?);
    }
    if let Ok(val) = std::env::var("BRAVEPI_PORT") {
        let bp = raw.adapters.bravepi.get_or_insert(RawBravepiConfig {
            enabled: None,
            port: None,
        });
        bp.port = Some(val);
    }

    if let Ok(val) = std::env::var("RPI_LOCAL_ENABLED") {
        let rpi = raw.adapters.rpi_local.get_or_insert(RawRpiLocalConfig {
            enabled: None,
            bus_path: None,
            poll_interval_ms: None,
        });
        rpi.enabled = Some(parse_bool_env("RPI_LOCAL_ENABLED", &val)?);
    }
    if let Ok(val) = std::env::var("RPI_LOCAL_BUS_PATH") {
        let rpi = raw.adapters.rpi_local.get_or_insert(RawRpiLocalConfig {
            enabled: None,
            bus_path: None,
            poll_interval_ms: None,
        });
        rpi.bus_path = Some(val);
    }
    if let Ok(val) = std::env::var("RPI_LOCAL_POLL_INTERVAL_MS") {
        let rpi = raw.adapters.rpi_local.get_or_insert(RawRpiLocalConfig {
            enabled: None,
            bus_path: None,
            poll_interval_ms: None,
        });
        rpi.poll_interval_ms = Some(parse_u64_env("RPI_LOCAL_POLL_INTERVAL_MS", &val)?);
    }

    Ok(())
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

    // ── apply_env tests ────────────────────────────────

    /// All ENV vars that `apply_env()` and `load()` read.
    const CONFIG_ENV_KEYS: &[&str] = &[
        "IOTKIT_DB_PATH", "BRAVEPI_ENABLED", "BRAVEPI_PORT",
        "RPI_LOCAL_ENABLED", "RPI_LOCAL_BUS_PATH", "RPI_LOCAL_POLL_INTERVAL_MS",
        "IOTKIT_CONFIG_PATH",
    ];

    /// RAII guard that restores env vars on drop (including on panic/unwind).
    struct EnvGuard {
        prior: Vec<(&'static str, Option<String>)>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, old) in &self.prior {
                // SAFETY: tests run single-threaded (--test-threads=1).
                match old {
                    Some(v) => unsafe { std::env::set_var(k, v) },
                    None => unsafe { std::env::remove_var(k) },
                }
            }
        }
    }

    /// Helper: clear ALL config-related env vars, set the given vars,
    /// run the closure, then restore all prior values (even on panic).
    fn with_env_vars<F: FnOnce()>(vars: &[(&str, &str)], f: F) {
        let prior: Vec<(&'static str, Option<String>)> = CONFIG_ENV_KEYS
            .iter()
            .map(|k| (*k, std::env::var(k).ok()))
            .collect();
        let _guard = EnvGuard { prior };
        for k in CONFIG_ENV_KEYS {
            // SAFETY: tests run single-threaded (--test-threads=1).
            unsafe { std::env::remove_var(k); }
        }
        for (k, v) in vars {
            // SAFETY: tests run single-threaded (--test-threads=1).
            unsafe { std::env::set_var(k, v); }
        }
        f();
    }

    #[test]
    fn apply_env_overrides_db_path() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("IOTKIT_DB_PATH", "env.db")], || {
            apply_env(&mut raw).unwrap();
        });
        assert_eq!(raw.gateway.db_path.as_deref(), Some("env.db"));
    }

    #[test]
    fn apply_env_overrides_bravepi_port() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("BRAVEPI_PORT", "/dev/ttyUSB1")], || {
            apply_env(&mut raw).unwrap();
        });
        let bp = raw.adapters.bravepi.as_ref().unwrap();
        assert_eq!(bp.port.as_deref(), Some("/dev/ttyUSB1"));
    }

    #[test]
    fn apply_env_overrides_rpi_local_enabled() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("RPI_LOCAL_ENABLED", "1")], || {
            apply_env(&mut raw).unwrap();
        });
        let rpi = raw.adapters.rpi_local.as_ref().unwrap();
        assert_eq!(rpi.enabled, Some(true));
    }

    #[test]
    fn apply_env_overrides_rpi_local_enabled_false() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("RPI_LOCAL_ENABLED", "false")], || {
            apply_env(&mut raw).unwrap();
        });
        let rpi = raw.adapters.rpi_local.as_ref().unwrap();
        assert_eq!(rpi.enabled, Some(false));
    }

    #[test]
    fn apply_env_overrides_bravepi_enabled() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("BRAVEPI_ENABLED", "0")], || {
            apply_env(&mut raw).unwrap();
        });
        let bp = raw.adapters.bravepi.as_ref().unwrap();
        assert_eq!(bp.enabled, Some(false));
    }

    #[test]
    fn apply_env_overrides_rpi_local_bus_path() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("RPI_LOCAL_BUS_PATH", "/dev/i2c-3")], || {
            apply_env(&mut raw).unwrap();
        });
        let rpi = raw.adapters.rpi_local.as_ref().unwrap();
        assert_eq!(rpi.bus_path.as_deref(), Some("/dev/i2c-3"));
    }

    #[test]
    fn apply_env_overrides_poll_interval() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("RPI_LOCAL_POLL_INTERVAL_MS", "2000")], || {
            apply_env(&mut raw).unwrap();
        });
        let rpi = raw.adapters.rpi_local.as_ref().unwrap();
        assert_eq!(rpi.poll_interval_ms, Some(2000));
    }

    #[test]
    fn apply_env_invalid_poll_interval_returns_error() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("RPI_LOCAL_POLL_INTERVAL_MS", "abc")], || {
            let result = apply_env(&mut raw);
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(msg.contains("RPI_LOCAL_POLL_INTERVAL_MS"), "error should name the var: {msg}");
            assert!(msg.contains("abc"), "error should include raw value: {msg}");
        });
    }

    #[test]
    fn apply_env_overrides_toml_value() {
        let mut raw: RawConfig = toml::from_str("[gateway]\ndb_path = \"from-toml.db\"").unwrap();
        assert_eq!(raw.gateway.db_path.as_deref(), Some("from-toml.db"));
        with_env_vars(&[("IOTKIT_DB_PATH", "from-env.db")], || {
            apply_env(&mut raw).unwrap();
        });
        assert_eq!(raw.gateway.db_path.as_deref(), Some("from-env.db"));
    }

    #[test]
    fn apply_env_invalid_bool_returns_error() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("BRAVEPI_ENABLED", "yes")], || {
            let result = apply_env(&mut raw);
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(msg.contains("BRAVEPI_ENABLED"), "error should name the var: {msg}");
            assert!(msg.contains("yes"), "error should include raw value: {msg}");
        });
    }
}
