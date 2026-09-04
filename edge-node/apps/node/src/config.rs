//! Bootstrap config: TOML parse → ENV merge → validated EdgeNodeConfig.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use iotkit_core_types::EdgeNodeId;
use serde::Deserialize;

use crate::input_adapters::PreparedInputAdapter;

pub const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
pub const MIN_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
pub const MAX_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60 * 60);

// ── Error ───────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid edge_node.db_path")]
    BootstrapDbPath,
    #[error("bootstrap database path changed before full configuration")]
    BootstrapDbPathMismatch,
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
    pub output: RawOutputConfig,
    #[serde(default)]
    pub status: RawStatusConfig,
    #[serde(default)]
    pub pipelines: RawPipelinesConfig,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawEdgeNodeConfig {
    pub id: Option<String>,
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
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawOutputConfig {
    pub mqtt: Option<RawMqttOutputConfig>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawMqttOutputConfig {
    pub enabled: Option<bool>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub password_file: Option<String>,
    pub trust_mode: Option<String>,
    pub ca_file: Option<String>,
    pub allow_insecure: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawStatusConfig {
    pub heartbeat_interval: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawPipelinesConfig {
    pub export_path: Option<String>,
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
    pub edge_node_id: EdgeNodeId,
    pub db_path: String,
    pub retention_days: u64,
    pub quarantine_ttl_days: u64,
    pub health_json_path: PathBuf,
    pub disk_high_watermark_pct: u64,
    pub bravepi: Option<BravepiConfig>,
    pub rpi_local: Option<RpiLocalResolvedConfig>,
    pub adapter_instances: Vec<PreparedInputAdapter>,
    pub api: ApiConfig,
    pub mqtt_output: Option<MqttOutputConfig>,
    pub status: StatusConfig,
    pub pipelines: PipelinesConfig,
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
    pub edge_node_id: EdgeNodeId,
    /// Where the API writes `pipelines.toml` after a committed pipeline
    /// operation (`[pipelines] export_path`).
    pub pipelines_export_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MqttTrustMode {
    SystemRoots,
    BundleOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttOutputConfig {
    pub host: String,
    pub port: u16,
    pub password_file: PathBuf,
    pub trust_mode: MqttTrustMode,
    pub ca_file: Option<PathBuf>,
    pub allow_insecure: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusConfig {
    pub heartbeat_interval: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelinesConfig {
    /// Derived backup of the pipeline definitions; written after each committed
    /// change and never read at startup.
    pub export_path: PathBuf,
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
    let edge_node_id = resolve_edge_node_id(raw.edge_node.id)?;
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

    let mqtt_output = resolve_mqtt_output(raw.output)?;
    let status = resolve_status(raw.status)?;
    let pipelines = resolve_pipelines(raw.pipelines, &db_path)?;
    let api = resolve_api(raw.api, edge_node_id.clone(), pipelines.export_path.clone())?;

    if adapter_instances.is_empty() && !api.enabled && mqtt_output.is_none() {
        return Err(ConfigError::Validation(
            "at least one adapter, api, or MQTT output must be enabled".to_string(),
        ));
    }

    Ok(EdgeNodeConfig {
        config_source: source,
        edge_node_id,
        db_path,
        retention_days,
        quarantine_ttl_days,
        health_json_path,
        disk_high_watermark_pct,
        bravepi,
        rpi_local,
        adapter_instances,
        api,
        mqtt_output,
        status,
        pipelines,
    })
}

fn resolve_edge_node_id(raw: Option<String>) -> Result<EdgeNodeId, ConfigError> {
    let raw = raw.ok_or_else(|| {
        ConfigError::Validation(
            "edge_node.id is required (stable edge-node-id, e.g. \"rpi1\")".to_string(),
        )
    })?;
    EdgeNodeId::parse(raw.as_str())
        .map_err(|error| ConfigError::Validation(format!("edge_node.id {error}: {raw:?}")))
}

fn resolve_status(raw: RawStatusConfig) -> Result<StatusConfig, ConfigError> {
    let heartbeat_interval = match raw.heartbeat_interval {
        None => DEFAULT_HEARTBEAT_INTERVAL,
        Some(text) => parse_duration(&text).ok_or_else(|| {
            ConfigError::Validation(format!(
                "status.heartbeat_interval must be an integer with unit ms, s, m, or h (e.g. \"60s\"), got {text:?}"
            ))
        })?,
    };
    if !(MIN_HEARTBEAT_INTERVAL..=MAX_HEARTBEAT_INTERVAL).contains(&heartbeat_interval) {
        return Err(ConfigError::Validation(format!(
            "status.heartbeat_interval must be between 5s and 1h, got {}ms",
            heartbeat_interval.as_millis()
        )));
    }
    Ok(StatusConfig { heartbeat_interval })
}

/// Parses `<integer><unit>` where unit is `ms`, `s`, `m`, or `h`.
fn parse_duration(text: &str) -> Option<Duration> {
    let text = text.trim();
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let (digits, unit) = text.split_at(split);
    let amount: u64 = digits.parse().ok()?;
    match unit {
        "ms" => Some(Duration::from_millis(amount)),
        "s" => Some(Duration::from_secs(amount)),
        "m" => amount.checked_mul(60).map(Duration::from_secs),
        "h" => amount.checked_mul(60 * 60).map(Duration::from_secs),
        _ => None,
    }
}

fn resolve_pipelines(
    raw: RawPipelinesConfig,
    db_path: &str,
) -> Result<PipelinesConfig, ConfigError> {
    let export_path = match raw.export_path {
        None => default_pipelines_export_path(db_path),
        Some(path) if path.trim().is_empty() => {
            return Err(ConfigError::Validation(
                "pipelines.export_path must not be empty".to_string(),
            ));
        }
        Some(path) => PathBuf::from(path),
    };
    if export_path == Path::new(db_path) {
        return Err(ConfigError::Validation(
            "pipelines.export_path must differ from edge_node.db_path".to_string(),
        ));
    }
    Ok(PipelinesConfig { export_path })
}

pub fn default_pipelines_export_path(db_path: &str) -> PathBuf {
    iotkit_core_pipeline::default_export_path(Path::new(db_path))
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

fn resolve_mqtt_output(raw: RawOutputConfig) -> Result<Option<MqttOutputConfig>, ConfigError> {
    let Some(raw) = raw.mqtt else {
        return Ok(None);
    };
    if !raw.enabled.unwrap_or(false) {
        return Ok(None);
    }

    let host = raw.host.unwrap_or_else(|| "127.0.0.1".to_string());
    if host.trim().is_empty() {
        return Err(ConfigError::Validation(
            "output.mqtt.host must not be empty".to_string(),
        ));
    }
    let password_file = raw
        .password_file
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| {
            ConfigError::Validation(
                "output.mqtt.password_file is required when MQTT output is enabled".to_string(),
            )
        })?;

    let port = raw.port.unwrap_or(8883);
    if port == 0 {
        return Err(ConfigError::Validation(
            "output.mqtt.port must be greater than zero".to_string(),
        ));
    }

    let allow_insecure = raw.allow_insecure.unwrap_or(false);
    let (trust_mode, ca_file) = if allow_insecure {
        if raw.trust_mode.is_some() || raw.ca_file.is_some() {
            return Err(ConfigError::Validation(
                "output.mqtt.allow_insecure cannot be combined with trust_mode or ca_file"
                    .to_string(),
            ));
        }
        (MqttTrustMode::SystemRoots, None)
    } else {
        match (raw.trust_mode.as_deref(), raw.ca_file) {
            (Some("system_roots"), None) => (MqttTrustMode::SystemRoots, None),
            (Some("system_roots"), Some(_)) => {
                return Err(ConfigError::Validation(
                    "output.mqtt.ca_file is forbidden with system_roots".to_string(),
                ));
            }
            (Some("bundle_only"), Some(path)) if !path.trim().is_empty() => {
                (MqttTrustMode::BundleOnly, Some(PathBuf::from(path)))
            }
            (Some("bundle_only"), _) => {
                return Err(ConfigError::Validation(
                    "output.mqtt.ca_file is required with bundle_only".to_string(),
                ));
            }
            (Some(other), _) => {
                return Err(ConfigError::Validation(format!(
                    "output.mqtt.trust_mode must be system_roots or bundle_only, got {other:?}"
                )));
            }
            (None, _) => {
                return Err(ConfigError::Validation(
                    "output.mqtt.trust_mode is required when TLS is enabled".to_string(),
                ));
            }
        }
    };

    Ok(Some(MqttOutputConfig {
        host,
        port,
        password_file: PathBuf::from(password_file),
        trust_mode,
        ca_file,
        allow_insecure,
    }))
}

fn resolve_api(
    raw: RawApiConfig,
    edge_node_id: EdgeNodeId,
    pipelines_export_path: PathBuf,
) -> Result<ApiConfig, ConfigError> {
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
    Ok(ApiConfig {
        enabled,
        bind,
        edge_node_id,
        pipelines_export_path,
    })
}

pub fn default_health_json_path(db_path: &str) -> PathBuf {
    sibling_of_db(db_path, "health.json")
}

fn sibling_of_db(db_path: &str, file_name: &str) -> PathBuf {
    match Path::new(db_path).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(file_name),
        _ => PathBuf::from(file_name),
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

/// The config source and bytes selected before the process-wide recovery fence.
///
/// Only the selected file and `IOTKIT_DB_PATH` are touched in this phase.  The
/// full, strict `RawConfig` deserialization and all other environment parsing
/// happen after the fence has been observed.  Keeping the bytes here also
/// prevents a path replacement between the read-only probe and post-fence
/// effective config construction from changing the selected source.
pub struct BootstrapConfig {
    source: ConfigSource,
    contents: Option<String>,
    db_path: String,
}

impl BootstrapConfig {
    pub fn db_path(&self) -> Result<&str, ConfigError> {
        if self.db_path.is_empty() {
            return Err(ConfigError::Validation(
                "db_path must not be empty".to_string(),
            ));
        }
        Ok(&self.db_path)
    }

    /// Complete strict parsing and environment application after the recovery
    /// fence.  This consumes the bootstrap so the selected source bytes cannot
    /// be silently reread from a mutable path.
    pub fn load_full(self) -> Result<UnresolvedConfig, ConfigError> {
        let bootstrap_db_path = self.db_path;
        let mut raw = match self.contents.as_deref() {
            Some(contents) => toml::from_str(contents)?,
            None => RawConfig::default(),
        };
        apply_env(&mut raw)?;
        let effective_db_path = raw.edge_node.db_path.as_deref().unwrap_or("iotkit.db");
        if effective_db_path.is_empty() {
            return Err(ConfigError::Validation(
                "db_path must not be empty".to_string(),
            ));
        }
        if effective_db_path != bootstrap_db_path {
            return Err(ConfigError::BootstrapDbPathMismatch);
        }
        Ok(UnresolvedConfig {
            raw,
            source: self.source,
        })
    }
}

/// Load and parse TOML plus ENV overrides without resolving adapters.
///
/// Config source resolution order:
/// 1. `--config <path>` CLI arg -> must exist
/// 2. `IOTKIT_CONFIG_PATH` ENV -> must exist
/// 3. `./iotkit.toml` -> optional (silently skipped if absent)
/// 4. No file -> all defaults
fn select_config_source(
    args: &[String],
) -> Result<(Option<PathBuf>, bool, ConfigSource), ConfigError> {
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

    Ok(match &found {
        Found::CliArg(p) => (Some(p.clone()), true, ConfigSource::CliArg(p.clone())),
        Found::EnvVar(p) => (Some(p.clone()), true, ConfigSource::EnvVar(p.clone())),
        Found::ImplicitFile(p) => (
            Some(p.clone()),
            false,
            ConfigSource::ImplicitFile(p.clone()),
        ),
        Found::DefaultsOnly => (None, false, ConfigSource::DefaultsOnly),
    })
}

/// Selects the config source and extracts only the database path needed by the
/// process-wide recovery fence.  Unrelated TOML fields are deliberately kept
/// out of this phase; strict unknown-field and adapter validation waits until
/// `BootstrapConfig::load_full` after the fence.
pub fn load_bootstrap(args: &[String]) -> Result<BootstrapConfig, ConfigError> {
    let (path_buf, explicit, source) = select_config_source(args)?;
    let contents = match path_buf.as_deref() {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(contents) => Some(contents),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => None,
            Err(e) => return Err(ConfigError::Io(e)),
        },
        None => None,
    };
    let db_path = if let Ok(value) = std::env::var("IOTKIT_DB_PATH") {
        value
    } else {
        extract_db_path(contents.as_deref())?.unwrap_or_else(|| "iotkit.db".to_string())
    };
    if db_path.is_empty() {
        return Err(ConfigError::Validation(
            "db_path must not be empty".to_string(),
        ));
    }
    Ok(BootstrapConfig {
        source,
        contents,
        db_path,
    })
}

#[derive(Clone, Copy)]
enum MultilineString {
    Basic,
    Literal,
}

fn update_multiline_state(line: &str, state: &mut Option<MultilineString>) {
    let bytes = line.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match state {
            Some(MultilineString::Basic) => {
                if bytes[index..].starts_with(b"\"\"\"") && !is_escaped(bytes, index) {
                    *state = None;
                    index += 3;
                } else {
                    index += 1;
                }
            }
            Some(MultilineString::Literal) => {
                if bytes[index..].starts_with(b"'''") {
                    *state = None;
                    index += 3;
                } else {
                    index += 1;
                }
            }
            None => {
                if bytes[index..].starts_with(b"\"\"\"") {
                    *state = Some(MultilineString::Basic);
                    index += 3;
                } else if bytes[index..].starts_with(b"'''") {
                    *state = Some(MultilineString::Literal);
                    index += 3;
                } else if bytes[index] == b'#' {
                    break;
                } else if bytes[index] == b'\"' {
                    index += 1;
                    while index < bytes.len() {
                        if bytes[index] == b'\\' {
                            index = index.saturating_add(2);
                        } else if bytes[index] == b'\"' {
                            index += 1;
                            break;
                        } else {
                            index += 1;
                        }
                    }
                } else if bytes[index] == b'\'' {
                    index += 1;
                    while index < bytes.len() {
                        if bytes[index] == b'\'' {
                            index += 1;
                            break;
                        }
                        index += 1;
                    }
                } else {
                    index += 1;
                }
            }
        }
    }
}

fn is_escaped(bytes: &[u8], index: usize) -> bool {
    let mut backslashes = 0;
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslashes += 1;
        cursor -= 1;
    }
    backslashes % 2 == 1
}

fn extract_db_path(contents: Option<&str>) -> Result<Option<String>, ConfigError> {
    let Some(contents) = contents else {
        return Ok(None);
    };
    if let Ok(value) = toml::from_str::<toml::Value>(contents) {
        let Some(edge_node) = value.get("edge_node") else {
            return Ok(None);
        };
        let Some(edge_node) = edge_node.as_table() else {
            return Err(ConfigError::BootstrapDbPath);
        };
        return match edge_node.get("db_path") {
            None => Ok(None),
            Some(value) => value
                .as_str()
                .map(|value| Some(value.to_owned()))
                .ok_or(ConfigError::BootstrapDbPath),
        };
    }

    // The full document is intentionally allowed to be malformed here: the
    // recovery fence must still use a canonical database path when an
    // unrelated table is broken.  Only exact root dotted keys and exact
    // `[edge_node]` assignments are recognized in this fallback.
    let mut in_edge_node = false;
    let mut in_root = true;
    let mut multiline = None;
    let mut found = None;
    for line in contents.lines() {
        if multiline.is_some() {
            update_multiline_state(line, &mut multiline);
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_edge_node = trimmed == "[edge_node]";
            in_root = false;
            update_multiline_state(line, &mut multiline);
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            update_multiline_state(line, &mut multiline);
            continue;
        };
        let key = key.trim();
        let is_db_path =
            (in_edge_node && key == "db_path") || (in_root && key == "edge_node.db_path");
        if !is_db_path {
            update_multiline_state(line, &mut multiline);
            continue;
        }
        if found.is_some() {
            return Err(ConfigError::BootstrapDbPath);
        }
        let snippet = format!("db_path = {}", value.trim());
        let parsed: RawEdgeNodeConfig =
            toml::from_str(&snippet).map_err(|_| ConfigError::BootstrapDbPath)?;
        found = parsed.db_path;
        if found.is_some() {
            return Ok(found);
        }
        update_multiline_state(line, &mut multiline);
    }
    if multiline.is_some() {
        return Err(ConfigError::BootstrapDbPath);
    }
    Ok(found)
}

pub fn load_unresolved(args: &[String]) -> Result<UnresolvedConfig, ConfigError> {
    load_bootstrap(args)?.load_full()
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
