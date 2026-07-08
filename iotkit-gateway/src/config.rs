//! Bootstrap config: TOML parse → ENV merge → validated GatewayConfig.

use std::net::{IpAddr, SocketAddr};
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
    #[serde(default)]
    pub api: RawApiConfig,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawGatewayConfig {
    pub db_path: Option<String>,
    pub retention_days: Option<u64>,
    pub quarantine_ttl_days: Option<u64>,
    pub health_json_path: Option<String>,
    pub disk_high_watermark_pct: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawAdaptersConfig {
    pub bravepi: Option<RawBravepiConfig>,
    pub rpi_local: Option<RawRpiLocalConfig>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawApiConfig {
    pub enabled: Option<bool>,
    pub bind: Option<String>,
    pub gateway_name: Option<String>,
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
    pub retention_days: u64,
    pub quarantine_ttl_days: u64,
    pub health_json_path: PathBuf,
    pub disk_high_watermark_pct: u64,
    pub bravepi: Option<BravepiConfig>,
    pub rpi_local: Option<RpiLocalResolvedConfig>,
    pub api: ApiConfig,
}

#[derive(Debug)]
pub enum ConfigSource {
    // Path payloads are retained for provenance in Debug/config introspection.
    #[allow(dead_code)]
    CliArg(PathBuf),
    #[allow(dead_code)]
    EnvVar(PathBuf),
    #[allow(dead_code)]
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

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub enabled: bool,
    pub bind: SocketAddr,
    pub gateway_name: String,
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
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => Ok(RawConfig::default()),
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
    if let Ok(val) = std::env::var("IOTKIT_RETENTION_DAYS") {
        raw.gateway.retention_days = Some(parse_u64_env("IOTKIT_RETENTION_DAYS", &val)?);
    }
    if let Ok(val) = std::env::var("IOTKIT_QUARANTINE_TTL_DAYS") {
        raw.gateway.quarantine_ttl_days = Some(parse_u64_env("IOTKIT_QUARANTINE_TTL_DAYS", &val)?);
    }
    if let Ok(val) = std::env::var("IOTKIT_HEALTH_JSON_PATH") {
        raw.gateway.health_json_path = Some(val);
    }
    if let Ok(val) = std::env::var("IOTKIT_DISK_HIGH_WATERMARK_PCT") {
        raw.gateway.disk_high_watermark_pct =
            Some(parse_u64_env("IOTKIT_DISK_HIGH_WATERMARK_PCT", &val)?);
    }
    if let Ok(val) = std::env::var("IOTKIT_API_ENABLED") {
        raw.api.enabled = Some(parse_bool_env("IOTKIT_API_ENABLED", &val)?);
    }
    if let Ok(val) = std::env::var("IOTKIT_API_BIND") {
        raw.api.bind = Some(val);
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

// ── Pipeline: resolve ──────────────────────────────────

/// Resolve a `RawConfig` into a validated `GatewayConfig`.
///
/// Applies defaults to `None` fields, validates constraints,
/// and returns `Err(ConfigError::Validation)` on invalid values.
pub fn resolve(raw: RawConfig, source: ConfigSource) -> Result<GatewayConfig, ConfigError> {
    let db_path = raw
        .gateway
        .db_path
        .unwrap_or_else(|| "iotkit.db".to_string());
    if db_path.is_empty() {
        return Err(ConfigError::Validation(
            "db_path must not be empty".to_string(),
        ));
    }
    let retention_days = raw.gateway.retention_days.unwrap_or(90).max(7);
    let quarantine_ttl_days = raw.gateway.quarantine_ttl_days.unwrap_or(7);
    let disk_high_watermark_pct = raw.gateway.disk_high_watermark_pct.unwrap_or(90);
    let health_json_path = raw
        .gateway
        .health_json_path
        .map(PathBuf::from)
        .unwrap_or_else(|| default_health_json_path(&db_path));

    // BravePI: enabled by default
    let bravepi = {
        let (enabled, port) = match raw.adapters.bravepi {
            Some(bp) => (
                bp.enabled.unwrap_or(true),
                bp.port.unwrap_or_else(|| "/dev/ttyAMA0".to_string()),
            ),
            None => (true, "/dev/ttyAMA0".to_string()),
        };
        if enabled {
            if port.is_empty() {
                return Err(ConfigError::Validation(
                    "adapters.bravepi.port must not be empty".to_string(),
                ));
            }
            Some(BravepiConfig { port })
        } else {
            None
        }
    };

    // RPi local: disabled by default
    let rpi_local = {
        let (enabled, bus_path, poll_interval_ms) = match raw.adapters.rpi_local {
            Some(rpi) => (
                rpi.enabled.unwrap_or(false),
                rpi.bus_path.unwrap_or_else(|| "/dev/i2c-1".to_string()),
                rpi.poll_interval_ms.unwrap_or(1000),
            ),
            None => (false, "/dev/i2c-1".to_string(), 1000),
        };
        if enabled {
            if bus_path.is_empty() {
                return Err(ConfigError::Validation(
                    "adapters.rpi_local.bus_path must not be empty".to_string(),
                ));
            }
            if poll_interval_ms == 0 {
                return Err(ConfigError::Validation(
                    "adapters.rpi_local.poll_interval_ms must be > 0".to_string(),
                ));
            }
            Some(RpiLocalResolvedConfig {
                bus_path,
                poll_interval_ms,
            })
        } else {
            None
        }
    };

    let api = resolve_api(raw.api)?;

    if bravepi.is_none() && rpi_local.is_none() && !api.enabled {
        return Err(ConfigError::Validation(
            "at least one adapter or api must be enabled".to_string(),
        ));
    }

    Ok(GatewayConfig {
        config_source: source,
        db_path,
        retention_days,
        quarantine_ttl_days,
        health_json_path,
        disk_high_watermark_pct,
        bravepi,
        rpi_local,
        api,
    })
}

fn resolve_api(raw: RawApiConfig) -> Result<ApiConfig, ConfigError> {
    let enabled = raw.enabled.unwrap_or(true);
    let bind_raw = raw.bind.unwrap_or_else(|| "0.0.0.0:8443".to_string());
    let bind: SocketAddr = bind_raw.parse().map_err(|_| {
        ConfigError::Validation(format!(
            "api.bind must be an IPv4 socket address: '{bind_raw}'"
        ))
    })?;
    if !matches!(bind.ip(), IpAddr::V4(_)) {
        return Err(ConfigError::Validation(
            "api.bind must be an IPv4 socket address".to_string(),
        ));
    }
    let gateway_name = raw
        .gateway_name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(default_gateway_name);
    Ok(ApiConfig {
        enabled,
        bind,
        gateway_name,
    })
}

fn default_gateway_name() -> String {
    crate::api::tls::hostname().unwrap_or_else(|| "iotkit-gateway".to_string())
}

pub fn default_health_json_path(db_path: &str) -> PathBuf {
    let db_path = Path::new(db_path);
    match db_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join("health.json"),
        _ => PathBuf::from("health.json"),
    }
}

// ── Pipeline: load (public entry point) ────────────────

/// Load gateway config from TOML file + ENV overrides.
///
/// Config source resolution order:
/// 1. `--config <path>` CLI arg -> must exist
/// 2. `IOTKIT_CONFIG_PATH` ENV -> must exist
/// 3. `./iotkit.toml` -> optional (silently skipped if absent)
/// 4. No file -> all defaults
pub fn load(args: &[String]) -> Result<GatewayConfig, ConfigError> {
    enum Found {
        CliArg(PathBuf),
        EnvVar(PathBuf),
        ImplicitFile(PathBuf),
        DefaultsOnly,
    }

    let found = if let Some(cli_path) = parse_config_arg(args)? {
        Found::CliArg(PathBuf::from(cli_path))
    } else if let Ok(env_path) = std::env::var("IOTKIT_CONFIG_PATH") {
        Found::EnvVar(PathBuf::from(env_path))
    } else {
        let implicit = PathBuf::from("iotkit.toml");
        match implicit.try_exists() {
            Ok(true) => Found::ImplicitFile(implicit),
            Ok(false) => Found::DefaultsOnly,
            Err(e) => return Err(ConfigError::Io(e)),
        }
    };

    let (path_buf, explicit, source) = match &found {
        Found::CliArg(p) => (Some(p.clone()), true, ConfigSource::CliArg(p.clone())),
        Found::EnvVar(p) => (Some(p.clone()), true, ConfigSource::EnvVar(p.clone())),
        Found::ImplicitFile(p) => (
            Some(p.clone()),
            false,
            ConfigSource::ImplicitFile(p.clone()),
        ),
        Found::DefaultsOnly => (None, false, ConfigSource::DefaultsOnly),
    };

    let mut raw = load_raw(path_buf.as_deref(), explicit)?;
    apply_env(&mut raw)?;
    resolve(raw, source)
}

/// Parse `--config <path>` from CLI args.
fn parse_config_arg(args: &[String]) -> Result<Option<&str>, ConfigError> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            return match iter.next() {
                Some(path) => Ok(Some(path.as_str())),
                None => Err(ConfigError::Validation(
                    "--config requires a file path argument".to_string(),
                )),
            };
        }
    }
    Ok(None)
}

#[cfg(test)]
// SAFETY: Tests in this module mutate process-global state (env vars, cwd).
// Tests that use `with_env_vars` or `CwdGuard` are annotated `#[serial]`
// (via the `serial_test` crate) so they run one-at-a-time even under the
// default parallel test runner.
mod tests {
    use super::*;
    use serial_test::serial;

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
        let result: Result<RawConfig, _> = toml::from_str("[adapters.nonexistent]\nfoo = \"bar\"");
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
        "IOTKIT_DB_PATH",
        "BRAVEPI_ENABLED",
        "BRAVEPI_PORT",
        "RPI_LOCAL_ENABLED",
        "RPI_LOCAL_BUS_PATH",
        "RPI_LOCAL_POLL_INTERVAL_MS",
        "IOTKIT_RETENTION_DAYS",
        "IOTKIT_QUARANTINE_TTL_DAYS",
        "IOTKIT_HEALTH_JSON_PATH",
        "IOTKIT_DISK_HIGH_WATERMARK_PCT",
        "IOTKIT_API_ENABLED",
        "IOTKIT_API_BIND",
        "IOTKIT_CONFIG_PATH",
    ];

    /// RAII guard that restores env vars on drop (including on panic/unwind).
    struct EnvGuard {
        prior: Vec<(&'static str, Option<String>)>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, old) in &self.prior {
                // SAFETY: env-var mutation is exclusive because these tests are
                // serialized via #[serial] (see the module-level comment above).
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
            // SAFETY: env-var mutation is exclusive because these tests are
            // serialized via #[serial] (see the module-level comment above).
            unsafe {
                std::env::remove_var(k);
            }
        }
        for (k, v) in vars {
            // SAFETY: env-var mutation is exclusive because these tests are
            // serialized via #[serial] (see the module-level comment above).
            unsafe {
                std::env::set_var(k, v);
            }
        }
        f();
    }

    #[test]
    #[serial]
    fn apply_env_overrides_db_path() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("IOTKIT_DB_PATH", "env.db")], || {
            apply_env(&mut raw).unwrap();
        });
        assert_eq!(raw.gateway.db_path.as_deref(), Some("env.db"));
    }

    #[test]
    #[serial]
    fn apply_env_overrides_bravepi_port() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("BRAVEPI_PORT", "/dev/ttyUSB1")], || {
            apply_env(&mut raw).unwrap();
        });
        let bp = raw.adapters.bravepi.as_ref().unwrap();
        assert_eq!(bp.port.as_deref(), Some("/dev/ttyUSB1"));
    }

    #[test]
    #[serial]
    fn apply_env_overrides_rpi_local_enabled() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("RPI_LOCAL_ENABLED", "1")], || {
            apply_env(&mut raw).unwrap();
        });
        let rpi = raw.adapters.rpi_local.as_ref().unwrap();
        assert_eq!(rpi.enabled, Some(true));
    }

    #[test]
    #[serial]
    fn apply_env_overrides_rpi_local_enabled_false() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("RPI_LOCAL_ENABLED", "false")], || {
            apply_env(&mut raw).unwrap();
        });
        let rpi = raw.adapters.rpi_local.as_ref().unwrap();
        assert_eq!(rpi.enabled, Some(false));
    }

    #[test]
    #[serial]
    fn apply_env_overrides_bravepi_enabled() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("BRAVEPI_ENABLED", "0")], || {
            apply_env(&mut raw).unwrap();
        });
        let bp = raw.adapters.bravepi.as_ref().unwrap();
        assert_eq!(bp.enabled, Some(false));
    }

    #[test]
    #[serial]
    fn apply_env_overrides_rpi_local_bus_path() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("RPI_LOCAL_BUS_PATH", "/dev/i2c-3")], || {
            apply_env(&mut raw).unwrap();
        });
        let rpi = raw.adapters.rpi_local.as_ref().unwrap();
        assert_eq!(rpi.bus_path.as_deref(), Some("/dev/i2c-3"));
    }

    #[test]
    #[serial]
    fn apply_env_overrides_poll_interval() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("RPI_LOCAL_POLL_INTERVAL_MS", "2000")], || {
            apply_env(&mut raw).unwrap();
        });
        let rpi = raw.adapters.rpi_local.as_ref().unwrap();
        assert_eq!(rpi.poll_interval_ms, Some(2000));
    }

    #[test]
    #[serial]
    fn apply_env_invalid_poll_interval_returns_error() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("RPI_LOCAL_POLL_INTERVAL_MS", "abc")], || {
            let result = apply_env(&mut raw);
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("RPI_LOCAL_POLL_INTERVAL_MS"),
                "error should name the var: {msg}"
            );
            assert!(msg.contains("abc"), "error should include raw value: {msg}");
        });
    }

    #[test]
    #[serial]
    fn apply_env_overrides_toml_value() {
        let mut raw: RawConfig = toml::from_str("[gateway]\ndb_path = \"from-toml.db\"").unwrap();
        assert_eq!(raw.gateway.db_path.as_deref(), Some("from-toml.db"));
        with_env_vars(&[("IOTKIT_DB_PATH", "from-env.db")], || {
            apply_env(&mut raw).unwrap();
        });
        assert_eq!(raw.gateway.db_path.as_deref(), Some("from-env.db"));
    }

    #[test]
    #[serial]
    fn apply_env_invalid_bool_returns_error() {
        let mut raw = RawConfig::default();
        with_env_vars(&[("BRAVEPI_ENABLED", "yes")], || {
            let result = apply_env(&mut raw);
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("BRAVEPI_ENABLED"),
                "error should name the var: {msg}"
            );
            assert!(msg.contains("yes"), "error should include raw value: {msg}");
        });
    }

    // ── resolve tests ──────────────────────────────────

    fn raw_with_defaults() -> RawConfig {
        RawConfig::default()
    }

    #[test]
    fn resolve_all_defaults() {
        let raw = raw_with_defaults();
        let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
        assert_eq!(config.db_path, "iotkit.db");
        assert_eq!(config.retention_days, 90);
        assert_eq!(config.quarantine_ttl_days, 7);
        assert_eq!(config.health_json_path, PathBuf::from("health.json"));
        assert_eq!(config.disk_high_watermark_pct, 90);
        // bravepi enabled by default
        let bp = config.bravepi.as_ref().unwrap();
        assert_eq!(bp.port, "/dev/ttyAMA0");
        // rpi_local disabled by default
        assert!(config.rpi_local.is_none());
    }

    #[test]
    fn resolve_health_json_path_defaults_to_db_parent() {
        let mut raw = raw_with_defaults();
        raw.gateway.db_path = Some("var/lib/iotkit/iotkit.db".to_string());
        let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
        assert_eq!(
            config.health_json_path,
            PathBuf::from("var/lib/iotkit/health.json")
        );
    }

    #[test]
    fn resolve_retention_days_clamps_to_minimum_seven() {
        let mut raw = raw_with_defaults();
        raw.gateway.retention_days = Some(3);
        let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
        assert_eq!(config.retention_days, 7);
    }

    #[test]
    #[serial]
    fn apply_env_overrides_retention_health_and_watermark_fields() {
        let mut raw = RawConfig::default();
        with_env_vars(
            &[
                ("IOTKIT_RETENTION_DAYS", "120"),
                ("IOTKIT_QUARANTINE_TTL_DAYS", "14"),
                ("IOTKIT_HEALTH_JSON_PATH", "/tmp/iotkit-health.json"),
                ("IOTKIT_DISK_HIGH_WATERMARK_PCT", "85"),
            ],
            || apply_env(&mut raw).unwrap(),
        );
        assert_eq!(raw.gateway.retention_days, Some(120));
        assert_eq!(raw.gateway.quarantine_ttl_days, Some(14));
        assert_eq!(
            raw.gateway.health_json_path.as_deref(),
            Some("/tmp/iotkit-health.json")
        );
        assert_eq!(raw.gateway.disk_high_watermark_pct, Some(85));
    }

    #[test]
    fn resolve_bravepi_disabled() {
        let mut raw = raw_with_defaults();
        raw.adapters.bravepi = Some(RawBravepiConfig {
            enabled: Some(false),
            port: None,
        });
        // rpi_local must be enabled since both can't be disabled
        raw.adapters.rpi_local = Some(RawRpiLocalConfig {
            enabled: Some(true),
            bus_path: None,
            poll_interval_ms: None,
        });
        let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
        assert!(config.bravepi.is_none());
    }

    #[test]
    fn resolve_rpi_local_enabled() {
        let mut raw = raw_with_defaults();
        raw.adapters.rpi_local = Some(RawRpiLocalConfig {
            enabled: Some(true),
            bus_path: Some("/dev/i2c-1".to_string()),
            poll_interval_ms: Some(500),
        });
        let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
        let rpi = config.rpi_local.as_ref().unwrap();
        assert_eq!(rpi.bus_path, "/dev/i2c-1");
        assert_eq!(rpi.poll_interval_ms, 500);
    }

    #[test]
    fn resolve_rpi_local_enabled_uses_defaults_for_missing_fields() {
        let mut raw = raw_with_defaults();
        raw.adapters.rpi_local = Some(RawRpiLocalConfig {
            enabled: Some(true),
            bus_path: None,
            poll_interval_ms: None,
        });
        let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
        let rpi = config.rpi_local.as_ref().unwrap();
        assert_eq!(rpi.bus_path, "/dev/i2c-1");
        assert_eq!(rpi.poll_interval_ms, 1000);
    }

    #[test]
    fn resolve_rejects_empty_db_path() {
        let mut raw = raw_with_defaults();
        raw.gateway.db_path = Some(String::new());
        let result = resolve(raw, ConfigSource::DefaultsOnly);
        assert!(matches!(result, Err(ConfigError::Validation(msg)) if msg.contains("db_path")));
    }

    #[test]
    fn resolve_rejects_empty_bus_path() {
        let mut raw = raw_with_defaults();
        raw.adapters.rpi_local = Some(RawRpiLocalConfig {
            enabled: Some(true),
            bus_path: Some(String::new()),
            poll_interval_ms: Some(1000),
        });
        let result = resolve(raw, ConfigSource::DefaultsOnly);
        assert!(matches!(result, Err(ConfigError::Validation(msg)) if msg.contains("bus_path")));
    }

    #[test]
    fn resolve_rejects_zero_poll_interval() {
        let mut raw = raw_with_defaults();
        raw.adapters.rpi_local = Some(RawRpiLocalConfig {
            enabled: Some(true),
            bus_path: Some("/dev/i2c-1".to_string()),
            poll_interval_ms: Some(0),
        });
        let result = resolve(raw, ConfigSource::DefaultsOnly);
        assert!(
            matches!(result, Err(ConfigError::Validation(msg)) if msg.contains("poll_interval_ms"))
        );
    }

    #[test]
    fn resolve_rejects_empty_bravepi_port() {
        let mut raw = raw_with_defaults();
        raw.adapters.bravepi = Some(RawBravepiConfig {
            enabled: Some(true),
            port: Some(String::new()),
        });
        let result = resolve(raw, ConfigSource::DefaultsOnly);
        assert!(matches!(result, Err(ConfigError::Validation(msg)) if msg.contains("port")));
    }

    #[test]
    fn resolve_rpi_local_disabled_explicit() {
        let mut raw = raw_with_defaults();
        raw.adapters.rpi_local = Some(RawRpiLocalConfig {
            enabled: Some(false),
            bus_path: Some("/dev/i2c-1".to_string()),
            poll_interval_ms: Some(500),
        });
        let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
        assert!(config.rpi_local.is_none());
    }

    #[test]
    fn resolve_allows_all_adapters_disabled_when_api_is_enabled() {
        let mut raw = raw_with_defaults();
        raw.adapters.bravepi = Some(RawBravepiConfig {
            enabled: Some(false),
            port: None,
        });
        // rpi_local defaults to disabled
        let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
        assert!(config.bravepi.is_none());
        assert!(config.rpi_local.is_none());
        assert!(config.api.enabled);
    }

    #[test]
    fn resolve_rejects_all_adapters_disabled_when_api_is_disabled() {
        let mut raw = raw_with_defaults();
        raw.adapters.bravepi = Some(RawBravepiConfig {
            enabled: Some(false),
            port: None,
        });
        raw.api.enabled = Some(false);
        let result = resolve(raw, ConfigSource::DefaultsOnly);
        assert!(
            matches!(result, Err(ConfigError::Validation(msg)) if msg.contains("at least one adapter or api"))
        );
    }

    // ── load tests ─────────────────────────────────────

    #[test]
    #[serial]
    fn load_with_explicit_missing_file_errors() {
        with_env_vars(&[], || {
            let args = vec![
                "gateway".to_string(),
                "--config".to_string(),
                "/tmp/no-such-file.toml".to_string(),
            ];
            let result = load(&args);
            assert!(result.is_err());
        });
    }

    #[test]
    #[serial]
    fn load_with_config_flag_but_no_path_errors() {
        with_env_vars(&[], || {
            let args = vec!["gateway".to_string(), "--config".to_string()];
            let result = load(&args);
            assert!(
                matches!(result, Err(ConfigError::Validation(msg)) if msg.contains("--config"))
            );
        });
    }

    /// RAII guard that restores the working directory on drop (including on panic).
    struct CwdGuard {
        prev: PathBuf,
    }
    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.prev);
        }
    }

    #[test]
    #[serial]
    fn load_with_no_args_and_no_env_uses_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let _cwd_guard = CwdGuard {
            prev: std::env::current_dir().unwrap(),
        };
        std::env::set_current_dir(tmp.path()).unwrap();
        with_env_vars(&[], || {
            let args = vec!["gateway".to_string()];
            let config = load(&args).unwrap();
            assert_eq!(config.db_path, "iotkit.db");
        });
    }

    #[test]
    #[serial]
    fn load_with_env_config_path_missing_file_errors() {
        with_env_vars(
            &[("IOTKIT_CONFIG_PATH", "/tmp/nonexistent-config.toml")],
            || {
                let args = vec!["gateway".to_string()];
                let result = load(&args);
                assert!(result.is_err());
            },
        );
    }

    #[test]
    #[serial]
    fn load_with_valid_file() {
        let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
        write!(
            tmpfile,
            r#"
[gateway]
db_path = "loaded.db"

[adapters.bravepi]
port = "/dev/ttyUSB0"
"#
        )
        .unwrap();
        with_env_vars(&[], || {
            let args = vec![
                "gateway".to_string(),
                "--config".to_string(),
                tmpfile.path().to_str().unwrap().to_string(),
            ];
            let config = load(&args).unwrap();
            assert_eq!(config.db_path, "loaded.db");
            let bp = config.bravepi.as_ref().unwrap();
            assert_eq!(bp.port, "/dev/ttyUSB0");
            assert!(matches!(config.config_source, ConfigSource::CliArg(_)));
        });
    }

    #[test]
    #[serial]
    fn load_integration_full_toml() {
        let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
        write!(
            tmpfile,
            r#"
[gateway]
db_path = "integration.db"

[adapters.bravepi]
enabled = true
port = "/dev/ttyUSB1"

[adapters.rpi_local]
enabled = true
bus_path = "/dev/i2c-3"
poll_interval_ms = 750
"#
        )
        .unwrap();
        with_env_vars(&[], || {
            let args = vec![
                "gateway".to_string(),
                "--config".to_string(),
                tmpfile.path().to_str().unwrap().to_string(),
            ];
            let config = load(&args).unwrap();
            assert_eq!(config.db_path, "integration.db");
            let bp = config.bravepi.as_ref().unwrap();
            assert_eq!(bp.port, "/dev/ttyUSB1");
            let rpi = config.rpi_local.as_ref().unwrap();
            assert_eq!(rpi.bus_path, "/dev/i2c-3");
            assert_eq!(rpi.poll_interval_ms, 750);
            assert!(matches!(config.config_source, ConfigSource::CliArg(_)));
        });
    }

    #[test]
    #[serial]
    fn load_with_env_config_path_valid_file() {
        let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
        write!(tmpfile, "[gateway]\ndb_path = \"env-path.db\"").unwrap();
        with_env_vars(
            &[("IOTKIT_CONFIG_PATH", tmpfile.path().to_str().unwrap())],
            || {
                let args = vec!["gateway".to_string()];
                let config = load(&args).unwrap();
                assert_eq!(config.db_path, "env-path.db");
                assert!(matches!(config.config_source, ConfigSource::EnvVar(_)));
            },
        );
    }

    #[test]
    #[serial]
    fn load_with_implicit_iotkit_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let toml_path = tmp.path().join("iotkit.toml");
        std::fs::write(&toml_path, "[gateway]\ndb_path = \"implicit.db\"").unwrap();
        let _cwd_guard = CwdGuard {
            prev: std::env::current_dir().unwrap(),
        };
        std::env::set_current_dir(tmp.path()).unwrap();
        with_env_vars(&[], || {
            let args = vec!["gateway".to_string()];
            let config = load(&args).unwrap();
            assert_eq!(config.db_path, "implicit.db");
            assert!(matches!(
                config.config_source,
                ConfigSource::ImplicitFile(_)
            ));
        });
    }

    #[test]
    #[serial]
    fn load_cli_arg_takes_precedence_over_env() {
        let mut cli_file = tempfile::NamedTempFile::new().unwrap();
        write!(cli_file, "[gateway]\ndb_path = \"from-cli.db\"").unwrap();
        let mut env_file = tempfile::NamedTempFile::new().unwrap();
        write!(env_file, "[gateway]\ndb_path = \"from-env.db\"").unwrap();
        with_env_vars(
            &[("IOTKIT_CONFIG_PATH", env_file.path().to_str().unwrap())],
            || {
                let args = vec![
                    "gateway".to_string(),
                    "--config".to_string(),
                    cli_file.path().to_str().unwrap().to_string(),
                ];
                let config = load(&args).unwrap();
                assert_eq!(config.db_path, "from-cli.db");
                assert!(matches!(config.config_source, ConfigSource::CliArg(_)));
            },
        );
    }
}
