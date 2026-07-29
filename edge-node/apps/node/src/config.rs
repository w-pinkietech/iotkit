//! Bootstrap config: TOML parse → ENV merge → validated EdgeNodeConfig.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::input_adapters::PreparedInputAdapter;

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
    pub edge_node: RawEdgeNodeConfig,
    #[serde(default)]
    pub adapters: RawAdaptersConfig,
    #[serde(default)]
    pub api: RawApiConfig,
    #[serde(default)]
    pub exit: RawExitConfig,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawEdgeNodeConfig {
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
    #[serde(default)]
    pub instances: BTreeMap<String, RawInputAdapterInstance>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawInputAdapterInstance {
    #[serde(rename = "type")]
    pub adapter_type: String,
    pub enabled: Option<bool>,
    pub config_schema_version: u16,
    pub source: String,
    pub port: Option<String>,
    pub bus_path: Option<String>,
    pub poll_interval_ms: Option<u64>,
    pub devices: Option<Vec<RawInputAdapterDevice>>,
}

#[derive(Debug, Deserialize)]
pub struct RawInputAdapterDevice {
    pub model: String,
    pub address: u8,
    #[serde(flatten)]
    pub settings: BTreeMap<String, RawAdapterConfigScalar>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum RawAdapterConfigScalar {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
}

impl RawAdapterConfigScalar {
    pub fn to_host_value(&self) -> iotkit_input_adapter_host_api::AdapterConfigScalar {
        use iotkit_input_adapter_host_api::AdapterConfigScalar;
        match self {
            Self::String(value) => AdapterConfigScalar::String(value.clone()),
            Self::Integer(value) => AdapterConfigScalar::Integer(*value),
            Self::Float(value) => AdapterConfigScalar::Float(*value),
            Self::Boolean(value) => AdapterConfigScalar::Boolean(*value),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawApiConfig {
    pub enabled: Option<bool>,
    pub bind: Option<String>,
    pub edge_node_name: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawExitConfig {
    pub mqtt: Option<RawMqttExitConfig>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawMqttExitConfig {
    pub enabled: Option<bool>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub password_file: Option<String>,
    pub trust_mode: Option<String>,
    pub ca_file: Option<String>,
    pub allow_insecure: Option<bool>,
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
pub struct EdgeNodeConfig {
    pub config_source: ConfigSource,
    pub db_path: String,
    pub retention_days: u64,
    pub quarantine_ttl_days: u64,
    pub health_json_path: PathBuf,
    pub disk_high_watermark_pct: u64,
    pub bravepi: Option<BravepiConfig>,
    pub rpi_local: Option<RpiLocalResolvedConfig>,
    pub adapter_instances: Vec<PreparedInputAdapter>,
    pub api: ApiConfig,
    pub mqtt_exit: Option<MqttExitConfig>,
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
    pub edge_node_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MqttTrustMode {
    SystemRoots,
    BundleOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttExitConfig {
    pub host: String,
    pub port: u16,
    pub password_file: PathBuf,
    pub trust_mode: MqttTrustMode,
    pub ca_file: Option<PathBuf>,
    pub allow_insecure: bool,
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
        raw.edge_node.db_path = Some(val);
    }
    if let Ok(val) = std::env::var("IOTKIT_RETENTION_DAYS") {
        raw.edge_node.retention_days = Some(parse_u64_env("IOTKIT_RETENTION_DAYS", &val)?);
    }
    if let Ok(val) = std::env::var("IOTKIT_QUARANTINE_TTL_DAYS") {
        raw.edge_node.quarantine_ttl_days =
            Some(parse_u64_env("IOTKIT_QUARANTINE_TTL_DAYS", &val)?);
    }
    if let Ok(val) = std::env::var("IOTKIT_HEALTH_JSON_PATH") {
        raw.edge_node.health_json_path = Some(val);
    }
    if let Ok(val) = std::env::var("IOTKIT_DISK_HIGH_WATERMARK_PCT") {
        raw.edge_node.disk_high_watermark_pct =
            Some(parse_u64_env("IOTKIT_DISK_HIGH_WATERMARK_PCT", &val)?);
    }
    if let Ok(val) = std::env::var("IOTKIT_API_ENABLED") {
        raw.api.enabled = Some(parse_bool_env("IOTKIT_API_ENABLED", &val)?);
    }
    if let Ok(val) = std::env::var("IOTKIT_API_BIND") {
        raw.api.bind = Some(val);
    }

    let legacy_adapter_form = raw.adapters.instances.is_empty();
    if legacy_adapter_form && let Ok(val) = std::env::var("BRAVEPI_ENABLED") {
        let bp = raw.adapters.bravepi.get_or_insert(RawBravepiConfig {
            enabled: None,
            port: None,
        });
        bp.enabled = Some(parse_bool_env("BRAVEPI_ENABLED", &val)?);
    }
    if legacy_adapter_form && let Ok(val) = std::env::var("BRAVEPI_PORT") {
        let bp = raw.adapters.bravepi.get_or_insert(RawBravepiConfig {
            enabled: None,
            port: None,
        });
        bp.port = Some(val);
    }

    if legacy_adapter_form && let Ok(val) = std::env::var("RPI_LOCAL_ENABLED") {
        let rpi = raw.adapters.rpi_local.get_or_insert(RawRpiLocalConfig {
            enabled: None,
            bus_path: None,
            poll_interval_ms: None,
        });
        rpi.enabled = Some(parse_bool_env("RPI_LOCAL_ENABLED", &val)?);
    }
    if legacy_adapter_form && let Ok(val) = std::env::var("RPI_LOCAL_BUS_PATH") {
        let rpi = raw.adapters.rpi_local.get_or_insert(RawRpiLocalConfig {
            enabled: None,
            bus_path: None,
            poll_interval_ms: None,
        });
        rpi.bus_path = Some(val);
    }
    if legacy_adapter_form && let Ok(val) = std::env::var("RPI_LOCAL_POLL_INTERVAL_MS") {
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

/// Resolve a `RawConfig` into a validated `EdgeNodeConfig`.
///
/// Applies defaults to `None` fields, validates constraints,
/// and returns `Err(ConfigError::Validation)` on invalid values.
pub fn resolve(raw: RawConfig, source: ConfigSource) -> Result<EdgeNodeConfig, ConfigError> {
    let db_path = raw
        .edge_node
        .db_path
        .unwrap_or_else(|| "iotkit.db".to_string());
    if db_path.is_empty() {
        return Err(ConfigError::Validation(
            "db_path must not be empty".to_string(),
        ));
    }
    let retention_days = raw.edge_node.retention_days.unwrap_or(90).max(7);
    let quarantine_ttl_days = raw.edge_node.quarantine_ttl_days.unwrap_or(7);
    let disk_high_watermark_pct = raw.edge_node.disk_high_watermark_pct.unwrap_or(90);
    let health_json_path = raw
        .edge_node
        .health_json_path
        .map(PathBuf::from)
        .unwrap_or_else(|| default_health_json_path(&db_path));

    let using_instances = !raw.adapters.instances.is_empty();
    if using_instances && (raw.adapters.bravepi.is_some() || raw.adapters.rpi_local.is_some()) {
        return Err(ConfigError::Validation(
            "adapters.instances cannot be combined with adapters.bravepi or adapters.rpi_local"
                .to_string(),
        ));
    }

    // BravePI: enabled by default in the legacy form.
    let bravepi = if using_instances {
        None
    } else {
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
    let rpi_local = if using_instances {
        None
    } else {
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

    let adapter_instances = if using_instances {
        resolve_adapter_instances(raw.adapters.instances)?
    } else {
        let mut instances = Vec::new();
        if let Some(bravepi) = &bravepi {
            instances.push(
                crate::input_adapters::resolve_instance(
                    "bravepi_main".into(),
                    RawInputAdapterInstance {
                        adapter_type: "bravepi-mainboard".into(),
                        enabled: Some(true),
                        config_schema_version: 1,
                        source: format!("bravepi-mainboard:{}", bravepi.port),
                        port: Some(bravepi.port.clone()),
                        bus_path: None,
                        poll_interval_ms: None,
                        devices: None,
                    },
                )
                .map_err(ConfigError::Validation)?
                .expect("enabled legacy BravePI instance"),
            );
        }
        if let Some(rpi) = &rpi_local {
            instances.push(
                crate::input_adapters::resolve_instance(
                    "rpi_local_default".into(),
                    RawInputAdapterInstance {
                        adapter_type: "rpi-local".into(),
                        enabled: Some(true),
                        config_schema_version: 1,
                        source: "rpi-local:default".into(),
                        port: None,
                        bus_path: Some(rpi.bus_path.clone()),
                        poll_interval_ms: Some(rpi.poll_interval_ms),
                        devices: None,
                    },
                )
                .map_err(ConfigError::Validation)?
                .expect("enabled legacy RPi instance"),
            );
        }
        instances
    };

    let api = resolve_api(raw.api)?;
    let mqtt_exit = resolve_mqtt_exit(raw.exit)?;

    if adapter_instances.is_empty() && !api.enabled && mqtt_exit.is_none() {
        return Err(ConfigError::Validation(
            "at least one adapter, api, or MQTT exit must be enabled".to_string(),
        ));
    }

    Ok(EdgeNodeConfig {
        config_source: source,
        db_path,
        retention_days,
        quarantine_ttl_days,
        health_json_path,
        disk_high_watermark_pct,
        bravepi,
        rpi_local,
        adapter_instances,
        api,
        mqtt_exit,
    })
}

fn resolve_adapter_instances(
    raw_instances: BTreeMap<String, RawInputAdapterInstance>,
) -> Result<Vec<PreparedInputAdapter>, ConfigError> {
    let mut resolved = Vec::new();
    let mut sources = std::collections::BTreeSet::new();
    for (raw_id, raw) in raw_instances {
        let Some(config) = crate::input_adapters::resolve_instance(raw_id, raw)
            .map_err(ConfigError::Validation)?
        else {
            continue;
        };
        if !sources.insert(config.source().as_str().to_owned()) {
            return Err(ConfigError::Validation(format!(
                "duplicate input adapter source {:?}",
                config.source().as_str()
            )));
        }
        resolved.push(config);
    }
    Ok(resolved)
}

fn resolve_mqtt_exit(raw: RawExitConfig) -> Result<Option<MqttExitConfig>, ConfigError> {
    let Some(raw) = raw.mqtt else {
        return Ok(None);
    };
    if !raw.enabled.unwrap_or(false) {
        return Ok(None);
    }

    let host = raw.host.unwrap_or_else(|| "127.0.0.1".to_string());
    if host.trim().is_empty() {
        return Err(ConfigError::Validation(
            "exit.mqtt.host must not be empty".to_string(),
        ));
    }
    let password_file = raw
        .password_file
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| {
            ConfigError::Validation(
                "exit.mqtt.password_file is required when MQTT exit is enabled".to_string(),
            )
        })?;

    let port = raw.port.unwrap_or(8883);
    if port == 0 {
        return Err(ConfigError::Validation(
            "exit.mqtt.port must be greater than zero".to_string(),
        ));
    }

    let allow_insecure = raw.allow_insecure.unwrap_or(false);
    let (trust_mode, ca_file) = if allow_insecure {
        if raw.trust_mode.is_some() || raw.ca_file.is_some() {
            return Err(ConfigError::Validation(
                "exit.mqtt.allow_insecure cannot be combined with trust_mode or ca_file"
                    .to_string(),
            ));
        }
        (MqttTrustMode::SystemRoots, None)
    } else {
        match (raw.trust_mode.as_deref(), raw.ca_file) {
            (Some("system_roots"), None) => (MqttTrustMode::SystemRoots, None),
            (Some("system_roots"), Some(_)) => {
                return Err(ConfigError::Validation(
                    "exit.mqtt.ca_file is forbidden with system_roots".to_string(),
                ));
            }
            (Some("bundle_only"), Some(path)) if !path.trim().is_empty() => {
                (MqttTrustMode::BundleOnly, Some(PathBuf::from(path)))
            }
            (Some("bundle_only"), _) => {
                return Err(ConfigError::Validation(
                    "exit.mqtt.ca_file is required with bundle_only".to_string(),
                ));
            }
            (Some(other), _) => {
                return Err(ConfigError::Validation(format!(
                    "exit.mqtt.trust_mode must be system_roots or bundle_only, got {other:?}"
                )));
            }
            (None, _) => {
                return Err(ConfigError::Validation(
                    "exit.mqtt.trust_mode is required when TLS is enabled".to_string(),
                ));
            }
        }
    };

    Ok(Some(MqttExitConfig {
        host,
        port,
        password_file: PathBuf::from(password_file),
        trust_mode,
        ca_file,
        allow_insecure,
    }))
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
    let edge_node_name = raw
        .edge_node_name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(default_edge_node_name);
    Ok(ApiConfig {
        enabled,
        bind,
        edge_node_name,
    })
}

fn default_edge_node_name() -> String {
    crate::api::tls::hostname().unwrap_or_else(|| "iotkit-edge-node".to_string())
}

pub fn default_health_json_path(db_path: &str) -> PathBuf {
    let db_path = Path::new(db_path);
    match db_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join("health.json"),
        _ => PathBuf::from("health.json"),
    }
}

// ── Pipeline: load (public entry point) ────────────────

/// Parsed TOML and environment overrides before adapter/catalog resolution.
///
/// The composition root uses this boundary to obtain `db_path`, probe the
/// process-wide recovery fence, and only then resolve adapter instances and
/// effective configuration. Keeping the raw value and source together avoids
/// reading the config file twice or changing source precedence between phases.
pub struct UnresolvedConfig {
    raw: RawConfig,
    source: ConfigSource,
}

impl UnresolvedConfig {
    pub fn db_path(&self) -> Result<&str, ConfigError> {
        let path = self.raw.edge_node.db_path.as_deref().unwrap_or("iotkit.db");
        if path.is_empty() {
            return Err(ConfigError::Validation(
                "db_path must not be empty".to_string(),
            ));
        }
        Ok(path)
    }

    pub fn resolve(self) -> Result<EdgeNodeConfig, ConfigError> {
        resolve(self.raw, self.source)
    }
}

/// Load and parse TOML plus ENV overrides without resolving adapters.
///
/// Config source resolution order:
/// 1. `--config <path>` CLI arg -> must exist
/// 2. `IOTKIT_CONFIG_PATH` ENV -> must exist
/// 3. `./iotkit.toml` -> optional (silently skipped if absent)
/// 4. No file -> all defaults
pub fn load_unresolved(args: &[String]) -> Result<UnresolvedConfig, ConfigError> {
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
    Ok(UnresolvedConfig { raw, source })
}

/// Load Edge Node config from TOML + ENV and resolve all adapters/effective values.
pub fn load(args: &[String]) -> Result<EdgeNodeConfig, ConfigError> {
    load_unresolved(args)?.resolve()
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
#[path = "../tests/unit/config_tests.rs"]
mod tests;
