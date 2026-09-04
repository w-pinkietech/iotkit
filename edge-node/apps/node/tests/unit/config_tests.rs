// SAFETY: Tests in this module mutate process-global state (env vars, cwd).
// Tests that use `with_env_vars` or `CwdGuard` are annotated `#[serial]`
// (via the `serial_test` crate) so they run one-at-a-time even under the
// default parallel test runner.
use super::*;
use serial_test::serial;

#[test]
fn parse_full_toml() {
    let toml_str = r#"
[edge_node]
id = "kitchen-edge"
db_path = "test.db"

[adapters.bravepi]
enabled = true
port = "/dev/ttyUSB0"

[adapters.rpi_local]
enabled = true
bus_path = "/dev/i2c-3"
poll_interval_ms = 500

[output.mqtt]
enabled = true
host = "edge.internal"
port = 8883
password_file = "/run/secrets/iotkit-mqtt-password"
trust_mode = "bundle_only"
ca_file = "/etc/iotkit/edge-ca.pem"

[status]
heartbeat_interval = "30s"

[pipelines]
export_path = "/var/lib/iotkit/pipelines.toml"
"#;
    let raw: RawConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(raw.edge_node.id.as_deref(), Some("kitchen-edge"));
    assert_eq!(raw.edge_node.db_path.as_deref(), Some("test.db"));
    assert_eq!(raw.status.heartbeat_interval.as_deref(), Some("30s"));
    assert_eq!(
        raw.pipelines.export_path.as_deref(),
        Some("/var/lib/iotkit/pipelines.toml")
    );
    let bp = raw.adapters.bravepi.unwrap();
    assert_eq!(bp.enabled, Some(true));
    assert_eq!(bp.port.as_deref(), Some("/dev/ttyUSB0"));
    let rpi = raw.adapters.rpi_local.unwrap();
    assert_eq!(rpi.enabled, Some(true));
    assert_eq!(rpi.bus_path.as_deref(), Some("/dev/i2c-3"));
    assert_eq!(rpi.poll_interval_ms, Some(500));
    let mqtt = raw.output.mqtt.unwrap();
    assert_eq!(mqtt.host.as_deref(), Some("edge.internal"));
    assert_eq!(
        mqtt.password_file.as_deref(),
        Some("/run/secrets/iotkit-mqtt-password")
    );
}

#[test]
fn parse_empty_toml_gives_defaults() {
    let raw: RawConfig = toml::from_str("").unwrap();
    assert!(raw.edge_node.db_path.is_none());
    assert!(raw.adapters.bravepi.is_none());
    assert!(raw.adapters.rpi_local.is_none());
}

#[test]
fn unknown_field_rejected() {
    let result: Result<RawConfig, _> = toml::from_str("[edge_node]\nunknown = true");
    assert!(result.is_err());
}

#[test]
fn legacy_gateway_root_is_rejected() {
    let result: Result<RawConfig, _> = toml::from_str("[gateway]\ndb_path = \"old.db\"");
    assert!(result.is_err());
}

#[test]
fn legacy_gateway_name_is_rejected() {
    let result: Result<RawConfig, _> = toml::from_str("[api]\ngateway_name = \"old-name\"");
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
    assert!(raw.edge_node.db_path.is_none());
}

#[test]
fn load_raw_missing_explicit_returns_error() {
    let result = load_raw(Some(Path::new("/tmp/does-not-exist.toml")), true);
    assert!(matches!(result, Err(ConfigError::Io(_))));
}

#[test]
fn load_raw_valid_file() {
    let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
    write!(
        tmpfile,
        "[edge_node]\nid = \"test-node\"\ndb_path = \"from-file.db\""
    )
    .unwrap();
    let raw = load_raw(Some(tmpfile.path()), true).unwrap();
    assert_eq!(raw.edge_node.db_path.as_deref(), Some("from-file.db"));
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
    assert!(raw.edge_node.db_path.is_none());
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
    assert_eq!(raw.edge_node.db_path.as_deref(), Some("env.db"));
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
    let mut raw: RawConfig =
        toml::from_str("[edge_node]\nid = \"test-node\"\ndb_path = \"from-toml.db\"").unwrap();
    assert_eq!(raw.edge_node.db_path.as_deref(), Some("from-toml.db"));
    with_env_vars(&[("IOTKIT_DB_PATH", "from-env.db")], || {
        apply_env(&mut raw).unwrap();
    });
    assert_eq!(raw.edge_node.db_path.as_deref(), Some("from-env.db"));
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
    let mut raw = RawConfig::default();
    raw.edge_node.id = Some("test-node".to_string());
    raw
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
    assert!(config.mqtt_output.is_none());
}

#[test]
fn resolve_requires_edge_node_id() {
    let raw = RawConfig::default();
    assert!(matches!(
        resolve(raw, ConfigSource::DefaultsOnly),
        Err(ConfigError::Validation(msg)) if msg.contains("edge_node.id is required")
    ));
}

#[test]
fn resolve_rejects_edge_node_id_outside_contract_grammar() {
    for id in ["", "Rpi1", "rpi_1", "-rpi1", "rpi1-", "rpi/1", "端末1"] {
        let mut raw = RawConfig::default();
        raw.edge_node.id = Some(id.to_string());
        assert!(
            matches!(
                resolve(raw, ConfigSource::DefaultsOnly),
                Err(ConfigError::Validation(msg)) if msg.starts_with("edge_node.id ")
            ),
            "{id:?}"
        );
    }
    let mut raw = RawConfig::default();
    raw.edge_node.id = Some("a".repeat(65));
    assert!(matches!(
        resolve(raw, ConfigSource::DefaultsOnly),
        Err(ConfigError::Validation(msg)) if msg.contains("64 bytes")
    ));
}

#[test]
fn resolve_exposes_edge_node_id_to_the_api() {
    let mut raw = RawConfig::default();
    raw.edge_node.id = Some("press-line-rpi1".to_string());
    let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
    assert_eq!(config.edge_node_id.as_str(), "press-line-rpi1");
    assert_eq!(config.api.edge_node_id, config.edge_node_id);
}

#[test]
fn parse_rejects_retired_exit_mqtt_and_api_edge_node_name() {
    assert!(
        toml::from_str::<RawConfig>("[exit.mqtt]\nenabled = true\n").is_err(),
        "[exit.mqtt] was renamed to [output.mqtt]"
    );
    assert!(
        toml::from_str::<RawConfig>("[api]\nedge_node_name = \"old\"\n").is_err(),
        "api.edge_node_name was replaced by edge_node.id"
    );
}

#[test]
fn resolve_status_defaults_to_sixty_second_heartbeat() {
    let config = resolve(raw_with_defaults(), ConfigSource::DefaultsOnly).unwrap();
    assert_eq!(config.status.heartbeat_interval, Duration::from_secs(60));
}

#[test]
fn resolve_status_accepts_units_within_range() {
    for (text, expected) in [
        ("5s", Duration::from_secs(5)),
        ("5000ms", Duration::from_secs(5)),
        ("2m", Duration::from_secs(120)),
        ("1h", Duration::from_secs(3600)),
        (" 45s ", Duration::from_secs(45)),
    ] {
        let mut raw = raw_with_defaults();
        raw.status.heartbeat_interval = Some(text.to_string());
        let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
        assert_eq!(config.status.heartbeat_interval, expected, "{text:?}");
    }
}

#[test]
fn resolve_status_rejects_out_of_range_or_malformed_interval() {
    for text in [
        "4999ms", "4s", "3601s", "61m", "2h", "60", "60 s", "s", "-60s", "1.5m", "60S",
    ] {
        let mut raw = raw_with_defaults();
        raw.status.heartbeat_interval = Some(text.to_string());
        assert!(
            matches!(
                resolve(raw, ConfigSource::DefaultsOnly),
                Err(ConfigError::Validation(msg)) if msg.contains("status.heartbeat_interval")
            ),
            "{text:?}"
        );
    }
}

#[test]
fn resolve_pipelines_export_path_defaults_next_to_the_database() {
    let mut raw = raw_with_defaults();
    raw.edge_node.db_path = Some("/var/lib/iotkit/iotkit.db".to_string());
    let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
    assert_eq!(
        config.pipelines.export_path,
        PathBuf::from("/var/lib/iotkit/pipelines.toml")
    );

    let config = resolve(raw_with_defaults(), ConfigSource::DefaultsOnly).unwrap();
    assert_eq!(
        config.pipelines.export_path,
        PathBuf::from("pipelines.toml")
    );
}

#[test]
fn resolve_pipelines_export_path_accepts_override_and_rejects_db_path() {
    let mut raw = raw_with_defaults();
    raw.pipelines.export_path = Some("/backup/pipelines.toml".to_string());
    let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
    assert_eq!(
        config.pipelines.export_path,
        PathBuf::from("/backup/pipelines.toml")
    );

    let mut raw = raw_with_defaults();
    raw.pipelines.export_path = Some("  ".to_string());
    assert!(matches!(
        resolve(raw, ConfigSource::DefaultsOnly),
        Err(ConfigError::Validation(msg)) if msg.contains("pipelines.export_path")
    ));

    let mut raw = raw_with_defaults();
    raw.pipelines.export_path = Some("iotkit.db".to_string());
    assert!(matches!(
        resolve(raw, ConfigSource::DefaultsOnly),
        Err(ConfigError::Validation(msg)) if msg.contains("must differ from edge_node.db_path")
    ));
}

#[test]
fn resolve_mqtt_output_uses_edge_identity_as_implicit_username() {
    let mut raw = raw_with_defaults();
    raw.output.mqtt = Some(RawMqttOutputConfig {
        enabled: Some(true),
        host: Some("edge.internal".to_string()),
        port: Some(8883),
        password_file: Some("/run/secrets/iotkit-mqtt-password".to_string()),
        trust_mode: Some("bundle_only".to_string()),
        ca_file: Some("/etc/iotkit/edge-ca.pem".to_string()),
        allow_insecure: None,
    });

    let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
    assert_eq!(
        config.mqtt_output,
        Some(MqttOutputConfig {
            host: "edge.internal".to_string(),
            port: 8883,
            password_file: PathBuf::from("/run/secrets/iotkit-mqtt-password"),
            trust_mode: MqttTrustMode::BundleOnly,
            ca_file: Some(PathBuf::from("/etc/iotkit/edge-ca.pem")),
            allow_insecure: false,
        })
    );
}

#[test]
fn resolve_mqtt_output_requires_explicit_trust_mode_for_tls() {
    let mut raw = raw_with_defaults();
    raw.output.mqtt = Some(RawMqttOutputConfig {
        enabled: Some(true),
        password_file: Some("/run/secrets/mqtt-password".into()),
        ..RawMqttOutputConfig::default()
    });
    assert!(matches!(
        resolve(raw, ConfigSource::DefaultsOnly),
        Err(ConfigError::Validation(message)) if message.contains("trust_mode")
    ));
}

#[test]
fn resolve_mqtt_output_rejects_ambiguous_trust_inputs() {
    for (trust_mode, ca_file) in [
        ("system_roots", Some("/etc/iotkit/ca.pem")),
        ("bundle_only", None),
        ("automatic", None),
    ] {
        let mut raw = raw_with_defaults();
        raw.output.mqtt = Some(RawMqttOutputConfig {
            enabled: Some(true),
            password_file: Some("/run/secrets/mqtt-password".into()),
            trust_mode: Some(trust_mode.into()),
            ca_file: ca_file.map(str::to_owned),
            ..RawMqttOutputConfig::default()
        });
        assert!(resolve(raw, ConfigSource::DefaultsOnly).is_err());
    }
}

#[test]
fn resolve_mqtt_output_accepts_bundle_only() {
    let mut raw = raw_with_defaults();
    raw.output.mqtt = Some(RawMqttOutputConfig {
        enabled: Some(true),
        host: Some("broker.factory.example".into()),
        password_file: Some("/run/secrets/mqtt-password".into()),
        trust_mode: Some("bundle_only".into()),
        ca_file: Some("/etc/iotkit/broker-ca.pem".into()),
        ..RawMqttOutputConfig::default()
    });
    let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
    assert_eq!(
        config.mqtt_output.unwrap().trust_mode,
        MqttTrustMode::BundleOnly
    );
}

#[test]
fn resolve_mqtt_output_requires_password_file() {
    let mut raw = raw_with_defaults();
    raw.output.mqtt = Some(RawMqttOutputConfig {
        enabled: Some(true),
        ..RawMqttOutputConfig::default()
    });

    let result = resolve(raw, ConfigSource::DefaultsOnly);
    assert!(matches!(result, Err(ConfigError::Validation(msg)) if msg.contains("password_file")));
}

#[test]
fn resolve_mqtt_output_rejects_zero_port() {
    let mut raw = raw_with_defaults();
    raw.output.mqtt = Some(RawMqttOutputConfig {
        enabled: Some(true),
        port: Some(0),
        password_file: Some("/run/secrets/iotkit-mqtt-password".to_string()),
        ..RawMqttOutputConfig::default()
    });

    let result = resolve(raw, ConfigSource::DefaultsOnly);
    assert!(matches!(result, Err(ConfigError::Validation(msg)) if msg.contains("port")));
}

#[test]
fn resolve_health_json_path_defaults_to_db_parent() {
    let mut raw = raw_with_defaults();
    raw.edge_node.db_path = Some("var/lib/iotkit/iotkit.db".to_string());
    let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
    assert_eq!(
        config.health_json_path,
        PathBuf::from("var/lib/iotkit/health.json")
    );
}

#[test]
fn resolve_retention_days_clamps_to_minimum_seven() {
    let mut raw = raw_with_defaults();
    raw.edge_node.retention_days = Some(3);
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
    assert_eq!(raw.edge_node.retention_days, Some(120));
    assert_eq!(raw.edge_node.quarantine_ttl_days, Some(14));
    assert_eq!(
        raw.edge_node.health_json_path.as_deref(),
        Some("/tmp/iotkit-health.json")
    );
    assert_eq!(raw.edge_node.disk_high_watermark_pct, Some(85));
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
fn explicit_instances_support_multiple_same_type_adapters() {
    let raw: RawConfig = toml::from_str(
        r#"
edge_node.id = "test-node"

[adapters.instances.line_a]
type = "bravepi-mainboard"
enabled = true
config_schema_version = 1
source = "input:bravepi-mainboard:line_a"
port = "/dev/serial0"

[adapters.instances.line_b]
type = "bravepi-mainboard"
enabled = true
config_schema_version = 1
source = "input:bravepi-mainboard:line_b"
port = "/dev/serial1"
"#,
    )
    .unwrap();
    let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
    assert_eq!(config.adapter_instances.len(), 2);
    assert_eq!(config.adapter_instances[0].instance_id().as_str(), "line_a");
    assert_eq!(
        config.adapter_instances[1].source().as_str(),
        "input:bravepi-mainboard:line_b"
    );
    assert!(config.bravepi.is_none());
}

#[test]
fn explicit_rpi_local_instance_uses_its_configured_device_list() {
    let raw: RawConfig = toml::from_str(
        r#"
edge_node.id = "test-node"

[adapters.instances.local_i2c]
type = "rpi-local"
enabled = true
config_schema_version = 1
source = "input:rpi-local:local_i2c"
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000

[[adapters.instances.local_i2c.devices]]
model = "mcp9600"
address = 0x61
thermocouple_type = "T"

[[adapters.instances.local_i2c.devices]]
model = "opt3001"
address = 0x45
"#,
    )
    .unwrap();

    let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
    let inventory = config.adapter_instances[0].positional_inventory();

    assert_eq!(inventory.len(), 2);
    assert_eq!(
        inventory[0].hardware_id,
        "input:rpi-local:local_i2c:i2c:0x61"
    );
    assert_eq!(inventory[0].model_id, "mcp9600");
    assert_eq!(
        inventory[1].hardware_id,
        "input:rpi-local:local_i2c:i2c:0x45"
    );
    assert_eq!(inventory[1].model_id, "opt3001");
}

#[test]
fn explicit_rpi_local_instance_rejects_invalid_device_configuration() {
    let unknown_model: RawConfig = toml::from_str(
        r#"
edge_node.id = "test-node"

[adapters.instances.local_i2c]
type = "rpi-local"
config_schema_version = 1
source = "input:rpi-local:local_i2c"
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000

[[adapters.instances.local_i2c.devices]]
model = "unknown"
address = 0x44
"#,
    )
    .unwrap();
    let error = resolve(unknown_model, ConfigSource::DefaultsOnly).unwrap_err();
    assert!(error.to_string().contains("unsupported device model"));

    let duplicate_address: RawConfig = toml::from_str(
        r#"
edge_node.id = "test-node"

[adapters.instances.local_i2c]
type = "rpi-local"
config_schema_version = 1
source = "input:rpi-local:local_i2c"
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000

[[adapters.instances.local_i2c.devices]]
model = "mcp9600"
address = 0x44
thermocouple_type = "K"

[[adapters.instances.local_i2c.devices]]
model = "opt3001"
address = 0x44
"#,
    )
    .unwrap();
    let error = resolve(duplicate_address, ConfigSource::DefaultsOnly).unwrap_err();
    assert!(error.to_string().contains("duplicate address"));
}

#[test]
fn model_specific_scalar_types_are_validated_by_the_adapter_catalog() {
    let raw: RawConfig = toml::from_str(
        r#"
edge_node.id = "test-node"

[adapters.instances.local_i2c]
type = "rpi-local"
config_schema_version = 1
source = "input:rpi-local:local_i2c"
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000

[[adapters.instances.local_i2c.devices]]
model = "mcp9600"
address = 0x60
thermocouple_type = 7
"#,
    )
    .unwrap();

    let error = resolve(raw, ConfigSource::DefaultsOnly).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("thermocouple_type must be a string")
    );
}

#[test]
fn rpi_local_public_config_rejects_the_documented_invalid_matrix() {
    let cases = [
        (
            "empty device list",
            r#"
edge_node.id = "test-node"

[adapters.instances.local_i2c]
type = "rpi-local"
config_schema_version = 1
source = "input:rpi-local:local_i2c"
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
devices = []
"#,
            "targets must not be empty",
        ),
        (
            "invalid I2C address",
            r#"
edge_node.id = "test-node"

[adapters.instances.local_i2c]
type = "rpi-local"
config_schema_version = 1
source = "input:rpi-local:local_i2c"
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000

[[adapters.instances.local_i2c.devices]]
model = "opt3001"
address = 0x07
"#,
            "outside valid I2C range",
        ),
        (
            "unsupported setting",
            r#"
edge_node.id = "test-node"

[adapters.instances.local_i2c]
type = "rpi-local"
config_schema_version = 1
source = "input:rpi-local:local_i2c"
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000

[[adapters.instances.local_i2c.devices]]
model = "opt3001"
address = 0x44
gain = true
"#,
            "unsupported setting",
        ),
        (
            "invalid thermocouple type",
            r#"
edge_node.id = "test-node"

[adapters.instances.local_i2c]
type = "rpi-local"
config_schema_version = 1
source = "input:rpi-local:local_i2c"
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000

[[adapters.instances.local_i2c.devices]]
model = "mcp9600"
address = 0x60
thermocouple_type = "X"
"#,
            "unsupported thermocouple_type",
        ),
        (
            "driver polling limit",
            r#"
edge_node.id = "test-node"

[adapters.instances.local_i2c]
type = "rpi-local"
config_schema_version = 1
source = "input:rpi-local:local_i2c"
bus_path = "/dev/i2c-1"
poll_interval_ms = 50

[[adapters.instances.local_i2c.devices]]
model = "opt3001"
address = 0x44
"#,
            "poll_interval_ms",
        ),
    ];

    for (name, input, expected) in cases {
        let raw: RawConfig =
            toml::from_str(input).unwrap_or_else(|error| panic!("{name}: {error}"));
        let error = resolve(raw, ConfigSource::DefaultsOnly)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(expected),
            "{name}: expected {expected:?} in {error:?}"
        );
    }
}

#[test]
fn legacy_and_explicit_rpi_local_forms_preserve_the_same_identity_recipe() {
    let legacy: RawConfig = toml::from_str(
        r#"
edge_node.id = "test-node"

[adapters.bravepi]
enabled = false

[adapters.rpi_local]
enabled = true
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
"#,
    )
    .unwrap();
    let legacy = resolve(legacy, ConfigSource::DefaultsOnly).unwrap();
    let legacy_adapter = &legacy.adapter_instances[0];

    let explicit: RawConfig = toml::from_str(
        r#"
edge_node.id = "test-node"

[adapters.instances.rpi_local_default]
type = "rpi-local"
enabled = true
config_schema_version = 1
source = "rpi-local:default"
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
"#,
    )
    .unwrap();
    let explicit = resolve(explicit, ConfigSource::DefaultsOnly).unwrap();
    let explicit_adapter = &explicit.adapter_instances[0];

    assert_eq!(legacy_adapter.adapter_type(), "rpi-local");
    assert_eq!(explicit_adapter.adapter_type(), "rpi-local");
    assert_eq!(legacy_adapter.source().as_str(), "rpi-local:default");
    assert_eq!(
        legacy_adapter.source().as_str(),
        explicit_adapter.source().as_str()
    );
    assert_eq!(
        legacy_adapter.positional_inventory(),
        explicit_adapter.positional_inventory()
    );
    assert_eq!(
        legacy_adapter
            .positional_inventory()
            .into_iter()
            .map(|device| device.hardware_id)
            .collect::<Vec<_>>(),
        ["rpi-local:default:i2c:0x60", "rpi-local:default:i2c:0x44",]
    );
}

#[test]
fn explicit_and_legacy_adapter_forms_are_mutually_exclusive() {
    let raw: RawConfig = toml::from_str(
        r#"
edge_node.id = "test-node"

[adapters.bravepi]
enabled = false

[adapters.instances.local]
type = "rpi-local"
config_schema_version = 1
source = "input:rpi-local:local"
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
"#,
    )
    .unwrap();
    let error = resolve(raw, ConfigSource::DefaultsOnly).unwrap_err();
    assert!(error.to_string().contains("cannot be combined"));
}

#[test]
fn explicit_instances_reject_duplicate_source_and_unknown_fields() {
    let duplicate: RawConfig = toml::from_str(
        r#"
edge_node.id = "test-node"

[adapters.instances.one]
type = "bravepi-mainboard"
config_schema_version = 1
source = "input:same"
port = "/dev/serial0"

[adapters.instances.two]
type = "bravepi-mainboard"
config_schema_version = 1
source = "input:same"
port = "/dev/serial1"
"#,
    )
    .unwrap();
    assert!(
        resolve(duplicate, ConfigSource::DefaultsOnly)
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );

    assert!(
        toml::from_str::<RawConfig>(
            r#"
edge_node.id = "test-node"

[adapters.instances.one]
type = "bravepi-mainboard"
config_schema_version = 1
source = "input:one"
port = "/dev/serial0"
secret_magic = "forbidden"
"#
        )
        .is_err()
    );
}

#[test]
fn legacy_resolution_pins_existing_identity_values() {
    let config = resolve(raw_with_defaults(), ConfigSource::DefaultsOnly).unwrap();
    assert_eq!(config.adapter_instances.len(), 1);
    let instance = &config.adapter_instances[0];
    assert_eq!(instance.adapter_type(), "bravepi-mainboard");
    assert_eq!(instance.instance_id().as_str(), "bravepi_main");
    assert_eq!(instance.source().as_str(), "bravepi-mainboard:/dev/ttyAMA0");
}

#[test]
fn resolve_rejects_empty_db_path() {
    let mut raw = raw_with_defaults();
    raw.edge_node.db_path = Some(String::new());
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
        matches!(result, Err(ConfigError::Validation(msg)) if msg.contains("at least one adapter, api, or MQTT output"))
    );
}

#[test]
fn resolve_allows_mqtt_output_only_mode() {
    let mut raw = raw_with_defaults();
    raw.adapters.bravepi = Some(RawBravepiConfig {
        enabled: Some(false),
        port: None,
    });
    raw.api.enabled = Some(false);
    raw.output.mqtt = Some(RawMqttOutputConfig {
        enabled: Some(true),
        password_file: Some("/run/secrets/iotkit-mqtt-password".to_string()),
        allow_insecure: Some(true),
        ..RawMqttOutputConfig::default()
    });

    let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
    assert!(config.bravepi.is_none());
    assert!(!config.api.enabled);
    assert!(config.mqtt_output.is_some());
}

// ── load tests ─────────────────────────────────────

#[test]
#[serial]
fn load_with_explicit_missing_file_errors() {
    with_env_vars(&[], || {
        let args = vec![
            "edge_node".to_string(),
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
        let args = vec!["edge_node".to_string(), "--config".to_string()];
        let result = load(&args);
        assert!(matches!(result, Err(ConfigError::Validation(msg)) if msg.contains("--config")));
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
fn load_with_no_args_and_no_env_requires_edge_node_id() {
    let tmp = tempfile::tempdir().unwrap();
    let _cwd_guard = CwdGuard {
        prev: std::env::current_dir().unwrap(),
    };
    std::env::set_current_dir(tmp.path()).unwrap();
    with_env_vars(&[], || {
        let args = vec!["edge_node".to_string()];
        let result = load(&args);
        assert!(
            matches!(result, Err(ConfigError::Validation(msg)) if msg.contains("edge_node.id is required"))
        );
    });
}

#[test]
#[serial]
fn load_with_env_config_path_missing_file_errors() {
    with_env_vars(
        &[("IOTKIT_CONFIG_PATH", "/tmp/nonexistent-config.toml")],
        || {
            let args = vec!["edge_node".to_string()];
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
[edge_node]
id = "test-node"
db_path = "loaded.db"

[adapters.bravepi]
port = "/dev/ttyUSB0"
"#
    )
    .unwrap();
    with_env_vars(&[], || {
        let args = vec![
            "edge_node".to_string(),
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
fn bootstrap_extracts_db_path_without_validating_unrelated_fields_and_reuses_file_bytes() {
    let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
    write!(
        tmpfile,
        "[edge_node]\nid = \"test-node\"\ndb_path = \"bootstrap.db\"\n[api]\nenabled = false\n"
    )
    .unwrap();
    with_env_vars(&[], || {
        let args = vec![
            "edge_node".to_string(),
            "--config".to_string(),
            tmpfile.path().to_str().unwrap().to_string(),
        ];
        let bootstrap = load_bootstrap(&args).unwrap();
        assert_eq!(bootstrap.db_path().unwrap(), "bootstrap.db");
        // The bootstrap owns the original bytes.  A post-fence full load must
        // not re-read a potentially replaced config path.
        std::fs::write(
            tmpfile.path(),
            "[edge_node]\nid = \"test-node\"\ndb_path = \"replaced.db\"\n[adapters.instances.bad]\nsource = \"bad\"\n",
        )
        .unwrap();
        let unresolved = bootstrap.load_full().unwrap();
        assert_eq!(unresolved.db_path().unwrap(), "bootstrap.db");
    });
}

#[test]
#[serial]
fn bootstrap_extracts_db_path_when_unrelated_toml_is_syntactically_broken() {
    let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
    write!(
        tmpfile,
        "[edge_node]\nid = \"test-node\"\ndb_path = \"bootstrap.db\"\n[unrelated]\nbroken = {{\n"
    )
    .unwrap();
    with_env_vars(&[], || {
        let args = vec![
            "edge_node".to_string(),
            "--config".to_string(),
            tmpfile.path().to_str().unwrap().to_string(),
        ];
        let bootstrap = load_bootstrap(&args).unwrap();
        assert_eq!(bootstrap.db_path().unwrap(), "bootstrap.db");
        assert!(matches!(bootstrap.load_full(), Err(ConfigError::Toml(_))));
    });
}

#[test]
#[serial]
fn bootstrap_fallback_ignores_multiline_string_content() {
    let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
    write!(
        tmpfile,
        "[edge_node]\nid = \"test-node\"\ndb_path = \"canonical.db\"\n[unrelated]\nbroken = \"\"\"\ndb_path = \"sentinel.db\"\n"
    )
    .unwrap();
    with_env_vars(&[], || {
        let args = vec![
            "edge_node".to_string(),
            "--config".to_string(),
            tmpfile.path().to_str().unwrap().to_string(),
        ];
        assert_eq!(
            load_bootstrap(&args).unwrap().db_path().unwrap(),
            "canonical.db"
        );
    });
}

#[test]
#[serial]
fn bootstrap_fallback_fails_closed_for_unterminated_multiline_before_candidate() {
    let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
    write!(
        tmpfile,
        "[unrelated]\nbroken = \"\"\"\ndb_path = \"sentinel.db\"\n"
    )
    .unwrap();
    with_env_vars(&[], || {
        let args = vec![
            "edge_node".to_string(),
            "--config".to_string(),
            tmpfile.path().to_str().unwrap().to_string(),
        ];
        assert!(matches!(
            load_bootstrap(&args),
            Err(ConfigError::BootstrapDbPath)
        ));
    });
}

#[test]
#[serial]
fn bootstrap_extracts_all_valid_db_path_forms() {
    for (contents, expected) in [
        ("edge_node = { db_path = \"inline.db\" }\n", "inline.db"),
        ("edge_node.db_path = 'dotted.db'\n", "dotted.db"),
        (
            "[edge_node]\nid = \"test-node\"\ndb_path = 'single-quoted.db'\n",
            "single-quoted.db",
        ),
    ] {
        let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
        write!(tmpfile, "{contents}").unwrap();
        with_env_vars(&[], || {
            let args = vec![
                "edge_node".to_string(),
                "--config".to_string(),
                tmpfile.path().to_str().unwrap().to_string(),
            ];
            assert_eq!(load_bootstrap(&args).unwrap().db_path().unwrap(), expected);
        });
    }
}

#[test]
#[serial]
fn bootstrap_rejects_effective_db_path_change_before_full_load() {
    let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
    write!(
        tmpfile,
        "[edge_node]\nid = \"test-node\"\ndb_path = \"bootstrap.db\"\n"
    )
    .unwrap();
    with_env_vars(&[], || {
        let args = vec![
            "edge_node".to_string(),
            "--config".to_string(),
            tmpfile.path().to_str().unwrap().to_string(),
        ];
        let bootstrap = load_bootstrap(&args).unwrap();
        unsafe { std::env::set_var("IOTKIT_DB_PATH", "changed-after-fence.db") };
        assert!(matches!(
            bootstrap.load_full(),
            Err(ConfigError::BootstrapDbPathMismatch)
        ));
    });
}

#[test]
#[serial]
fn load_integration_full_toml() {
    let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
    write!(
        tmpfile,
        r#"
[edge_node]
id = "test-node"
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
            "edge_node".to_string(),
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
    write!(
        tmpfile,
        "[edge_node]\nid = \"test-node\"\ndb_path = \"env-path.db\""
    )
    .unwrap();
    with_env_vars(
        &[("IOTKIT_CONFIG_PATH", tmpfile.path().to_str().unwrap())],
        || {
            let args = vec!["edge_node".to_string()];
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
    std::fs::write(
        &toml_path,
        "[edge_node]\nid = \"test-node\"\ndb_path = \"implicit.db\"",
    )
    .unwrap();
    let _cwd_guard = CwdGuard {
        prev: std::env::current_dir().unwrap(),
    };
    std::env::set_current_dir(tmp.path()).unwrap();
    with_env_vars(&[], || {
        let args = vec!["edge_node".to_string()];
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
    write!(
        cli_file,
        "[edge_node]\nid = \"test-node\"\ndb_path = \"from-cli.db\""
    )
    .unwrap();
    let mut env_file = tempfile::NamedTempFile::new().unwrap();
    write!(
        env_file,
        "[edge_node]\nid = \"test-node\"\ndb_path = \"from-env.db\""
    )
    .unwrap();
    with_env_vars(
        &[("IOTKIT_CONFIG_PATH", env_file.path().to_str().unwrap())],
        || {
            let args = vec![
                "edge_node".to_string(),
                "--config".to_string(),
                cli_file.path().to_str().unwrap().to_string(),
            ];
            let config = load(&args).unwrap();
            assert_eq!(config.db_path, "from-cli.db");
            assert!(matches!(config.config_source, ConfigSource::CliArg(_)));
        },
    );
}
