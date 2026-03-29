# Bootstrap Config Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace hardcoded gateway configuration with TOML file + ENV override loading (bootstrap-only scope).

**Architecture:** New `config` module in `iotkit-gateway` implements a three-stage pipeline: TOML parse → ENV merge → validated `GatewayConfig`. The gateway `main.rs` consumes `GatewayConfig` to start adapters. Adapters are unchanged.

**Tech Stack:** Rust, serde + toml, thiserror

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `iotkit-gateway/src/config.rs` | Config types, TOML parse, ENV merge, validation, `load()` |
| Modify | `iotkit-gateway/src/main.rs` | Use `config::load()` instead of hardcoded values |
| Modify | `iotkit-gateway/Cargo.toml` | Add `serde`, `toml`, `thiserror` dependencies |
| Modify | `rpi-local-adapter/src/lib.rs` | Add `validate()` public function for preflight validation |

---

### Task 1: Add dependencies to `iotkit-gateway/Cargo.toml`

**Files:**
- Modify: `iotkit-gateway/Cargo.toml`

- [ ] **Step 1: Add serde, toml, thiserror dependencies**

Add to `[dependencies]` in `iotkit-gateway/Cargo.toml`:

```toml
serde = { version = "1", features = ["derive"] }
toml = "0.8"
thiserror = "2"
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p iotkit-gateway`
Expected: compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add iotkit-gateway/Cargo.toml
git commit -m "feat(iotkit-gateway): add serde, toml, thiserror dependencies for config"
```

---

### Task 2: Create `config.rs` with types and `ConfigError`

**Files:**
- Create: `iotkit-gateway/src/config.rs`

- [ ] **Step 1: Write tests for ConfigError and RawConfig deserialization**

Create `iotkit-gateway/src/config.rs` with the test module first:

```rust
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
}
```

- [ ] **Step 2: Register config module in main.rs**

Add `mod config;` at the top of `iotkit-gateway/src/main.rs` (after `mod adapter_host;`):

```rust
mod adapter_host;
mod config;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p iotkit-gateway config::tests`
Expected: 4 tests pass

- [ ] **Step 4: Commit**

```bash
git add iotkit-gateway/src/config.rs iotkit-gateway/src/main.rs
git commit -m "feat(iotkit-gateway): add config types and TOML deserialization with tests"
```

---

### Task 3: Implement `load_raw()` — TOML file loading

**Files:**
- Modify: `iotkit-gateway/src/config.rs`

- [ ] **Step 1: Write tests for load_raw**

Add to the `tests` module in `config.rs`:

```rust
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
```

- [ ] **Step 2: Add tempfile dev-dependency**

Add to `iotkit-gateway/Cargo.toml`:

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p iotkit-gateway config::tests::load_raw`
Expected: FAIL — `load_raw` function not found

- [ ] **Step 4: Implement load_raw**

Add above `#[cfg(test)]` in `config.rs`:

```rust
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
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p iotkit-gateway config::tests`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add iotkit-gateway/src/config.rs iotkit-gateway/Cargo.toml
git commit -m "feat(iotkit-gateway): implement load_raw() for TOML file loading"
```

---

### Task 4: Implement `apply_env()` — ENV override

**Files:**
- Modify: `iotkit-gateway/src/config.rs`

- [ ] **Step 1: Write tests for apply_env**

Add to the `tests` module in `config.rs`:

```rust
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
    ///
    /// This ensures hermetic tests regardless of ambient environment.
    /// Restoration is guaranteed by the `EnvGuard` Drop impl.
    ///
    /// SAFETY: env var mutation is unsafe in edition 2024 because it is not
    /// thread-safe. We mitigate this by running env-mutating tests with
    /// `--test-threads=1`.
    fn with_env_vars<F: FnOnce()>(vars: &[(&str, &str)], f: F) {
        // Save ALL config-related env vars.
        let prior: Vec<(&'static str, Option<String>)> = CONFIG_ENV_KEYS
            .iter()
            .map(|k| (*k, std::env::var(k).ok()))
            .collect();
        let _guard = EnvGuard { prior };
        // Clear ALL config-related env vars.
        for k in CONFIG_ENV_KEYS {
            // SAFETY: tests run single-threaded (--test-threads=1).
            unsafe { std::env::remove_var(k); }
        }
        // Set only the vars requested by this test.
        for (k, v) in vars {
            // SAFETY: tests run single-threaded (--test-threads=1).
            unsafe { std::env::set_var(k, v); }
        }
        f();
        // _guard dropped here (or on panic), restoring all env vars.
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
        // Proves ENV > TOML precedence: TOML sets db_path to "from-toml.db",
        // ENV overrides it to "from-env.db".
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p iotkit-gateway config::tests::apply_env`
Expected: FAIL — `apply_env` function not found

- [ ] **Step 3: Implement apply_env**

Add above `#[cfg(test)]` in `config.rs`:

```rust
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

```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p iotkit-gateway config::tests -- --test-threads=1`
Expected: all tests pass (serial execution required because tests modify env vars)

- [ ] **Step 5: Commit**

```bash
git add iotkit-gateway/src/config.rs
git commit -m "feat(iotkit-gateway): implement apply_env() for ENV overrides"
```

---

### Task 5: Implement `resolve()` — validation and defaults

**Files:**
- Modify: `iotkit-gateway/src/config.rs`

- [ ] **Step 1: Write tests for resolve**

Add to the `tests` module in `config.rs`:

```rust
    fn raw_with_defaults() -> RawConfig {
        RawConfig::default()
    }

    #[test]
    fn resolve_all_defaults() {
        let raw = raw_with_defaults();
        let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
        assert_eq!(config.db_path, "iotkit.db");
        // bravepi enabled by default
        let bp = config.bravepi.as_ref().unwrap();
        assert_eq!(bp.port, "/dev/ttyAMA0");
        // rpi_local disabled by default
        assert!(config.rpi_local.is_none());
    }

    #[test]
    fn resolve_bravepi_disabled() {
        let mut raw = raw_with_defaults();
        raw.adapters.bravepi = Some(RawBravepiConfig {
            enabled: Some(false),
            port: None,
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
        assert!(matches!(result, Err(ConfigError::Validation(msg)) if msg.contains("poll_interval_ms")));
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
    fn resolve_rejects_all_adapters_disabled() {
        let mut raw = raw_with_defaults();
        raw.adapters.bravepi = Some(RawBravepiConfig {
            enabled: Some(false),
            port: None,
        });
        // rpi_local defaults to disabled
        let result = resolve(raw, ConfigSource::DefaultsOnly);
        assert!(matches!(result, Err(ConfigError::Validation(msg)) if msg.contains("at least one adapter")));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p iotkit-gateway config::tests::resolve`
Expected: FAIL — `resolve` function not found

- [ ] **Step 3: Implement resolve**

Add above `#[cfg(test)]` in `config.rs`:

```rust
/// Resolve a `RawConfig` into a validated `GatewayConfig`.
///
/// Applies defaults to `None` fields, validates constraints,
/// and returns `Err(ConfigError::Validation)` on invalid values.
pub fn resolve(raw: RawConfig, source: ConfigSource) -> Result<GatewayConfig, ConfigError> {
    let db_path = raw.gateway.db_path.unwrap_or_else(|| "iotkit.db".to_string());
    if db_path.is_empty() {
        return Err(ConfigError::Validation("db_path must not be empty".to_string()));
    }

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

    if bravepi.is_none() && rpi_local.is_none() {
        return Err(ConfigError::Validation(
            "at least one adapter must be enabled".to_string(),
        ));
    }

    Ok(GatewayConfig {
        config_source: source,
        db_path,
        bravepi,
        rpi_local,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p iotkit-gateway config::tests -- --test-threads=1`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add iotkit-gateway/src/config.rs
git commit -m "feat(iotkit-gateway): implement resolve() with validation and defaults"
```

---

### Task 6: Implement `load()` — the public entry point

**Files:**
- Modify: `iotkit-gateway/src/config.rs`

- [ ] **Step 1: Write tests for load**

Add to the `tests` module in `config.rs`:

```rust
    #[test]
    fn load_with_explicit_missing_file_errors() {
        let args = vec!["gateway".to_string(), "--config".to_string(), "/tmp/no-such-file.toml".to_string()];
        let result = load(&args);
        assert!(result.is_err());
    }

    #[test]
    fn load_with_config_flag_but_no_path_errors() {
        let args = vec!["gateway".to_string(), "--config".to_string()];
        let result = load(&args);
        assert!(matches!(result, Err(ConfigError::Validation(msg)) if msg.contains("--config")));
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
    fn load_with_no_args_and_no_env_uses_defaults() {
        // Use a temp dir as cwd to avoid picking up an ambient iotkit.toml.
        let tmp = tempfile::tempdir().unwrap();
        let _cwd_guard = CwdGuard { prev: std::env::current_dir().unwrap() };
        std::env::set_current_dir(tmp.path()).unwrap();
        with_env_vars(&[], || {
            let args = vec!["gateway".to_string()];
            let config = load(&args).unwrap();
            assert_eq!(config.db_path, "iotkit.db");
        });
        // _cwd_guard restores cwd on drop.
    }

    #[test]
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
    fn load_with_valid_file() {
        let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
        write!(tmpfile, r#"
[gateway]
db_path = "loaded.db"

[adapters.bravepi]
port = "/dev/ttyUSB0"
"#).unwrap();
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

    /// Integration test: load a full TOML file with all sections, verify
    /// the complete GatewayConfig fields. Uses with_env_vars to ensure
    /// hermetic environment (all config env vars cleared).
    #[test]
    fn load_integration_full_toml() {
        let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
        write!(tmpfile, r#"
[gateway]
db_path = "integration.db"

[adapters.bravepi]
enabled = true
port = "/dev/ttyUSB1"

[adapters.rpi_local]
enabled = true
bus_path = "/dev/i2c-3"
poll_interval_ms = 750
"#).unwrap();
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
    fn load_with_implicit_iotkit_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let toml_path = tmp.path().join("iotkit.toml");
        std::fs::write(&toml_path, "[gateway]\ndb_path = \"implicit.db\"").unwrap();
        let _cwd_guard = CwdGuard { prev: std::env::current_dir().unwrap() };
        std::env::set_current_dir(tmp.path()).unwrap();
        with_env_vars(&[], || {
            let args = vec!["gateway".to_string()];
            let config = load(&args).unwrap();
            assert_eq!(config.db_path, "implicit.db");
            assert!(matches!(config.config_source, ConfigSource::ImplicitFile(_)));
        });
        // _cwd_guard restores cwd on drop.
    }

    #[test]
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p iotkit-gateway config::tests::load`
Expected: FAIL — `load` function not found

- [ ] **Step 3: Implement load**

Add above `#[cfg(test)]` in `config.rs`:

```rust
/// Load gateway config from TOML file + ENV overrides.
///
/// Config source resolution order:
/// 1. `--config <path>` CLI arg → must exist
/// 2. `IOTKIT_CONFIG_PATH` ENV → must exist
/// 3. `./iotkit.toml` → optional (silently skipped if absent)
/// 4. No file → all defaults
pub fn load(args: &[String]) -> Result<GatewayConfig, ConfigError> {
    // Determine config file path and whether it was explicitly requested.
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
        // Use try_exists() to distinguish "not found" from permission errors.
        // Permission/IO errors on the implicit path are surfaced, not silently
        // swallowed (unlike exists() which returns false on errors).
        match implicit.try_exists() {
            Ok(true) => Found::ImplicitFile(implicit),
            Ok(false) => Found::DefaultsOnly,
            Err(e) => return Err(ConfigError::Io(e)),
        }
    };

    let (path_buf, explicit, source) = match &found {
        Found::CliArg(p) => (Some(p.clone()), true, ConfigSource::CliArg(p.clone())),
        Found::EnvVar(p) => (Some(p.clone()), true, ConfigSource::EnvVar(p.clone())),
        Found::ImplicitFile(p) => (Some(p.clone()), false, ConfigSource::ImplicitFile(p.clone())),
        Found::DefaultsOnly => (None, false, ConfigSource::DefaultsOnly),
    };

    let mut raw = load_raw(path_buf.as_deref(), explicit)?;
    apply_env(&mut raw)?;
    resolve(raw, source)
}

/// Parse `--config <path>` from CLI args.
///
/// Returns `Ok(Some(path))` if `--config <path>` is present.
/// Returns `Ok(None)` if `--config` is not present.
/// Returns `Err` if `--config` is present but no path follows.
fn parse_config_arg<'a>(args: &'a [String]) -> Result<Option<&'a str>, ConfigError> {
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p iotkit-gateway config::tests -- --test-threads=1`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add iotkit-gateway/src/config.rs
git commit -m "feat(iotkit-gateway): implement load() entry point with CLI/ENV/implicit file resolution"
```

---

### Task 7: Add `validate()` public API to rpi-local-adapter

**Files:**
- Modify: `rpi-local-adapter/src/lib.rs`

- [ ] **Step 1: Write test for validate**

Add to the `tests` module in `rpi-local-adapter/src/lib.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p rpi-local-adapter validate`
Expected: FAIL — `validate` function not found

- [ ] **Step 3: Implement validate**

Add after the `start` function in `rpi-local-adapter/src/lib.rs`:

```rust
/// Validate an `RpiLocalConfig` without starting the adapter.
///
/// Converts to `PollingAdapterConfig` internally and delegates to
/// `iotkit_polling_adapter_runtime::validate_config()`. Used for
/// preflight validation in the gateway before `start()`.
pub fn validate(config: &RpiLocalConfig) -> Result<(), String> {
    let polling_config = to_polling_config(config);
    iotkit_polling_adapter_runtime::validate_config(&polling_config)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p rpi-local-adapter`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add rpi-local-adapter/src/lib.rs
git commit -m "feat(rpi-local-adapter): add validate() public API for preflight validation"
```

---

### Task 8: Integrate config into `main.rs`

**Files:**
- Modify: `iotkit-gateway/src/main.rs`

- [ ] **Step 1: Replace hardcoded values with config::load()**

Replace the entire `main.rs` content with:

```rust
//! iotkit-gateway: composition root。
//! adapter を起動し、core/engine に event を渡す。

mod adapter_host;
mod config;

use adapter_host::{AdapterHost, AdapterHostEvent};
use iotkit_core_engine::Engine;
use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    let config = match config::load(&args) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to load config");
            std::process::exit(1);
        }
    };

    tracing::info!(source = ?config.config_source, "config loaded");
    tracing::info!(
        db_path = %config.db_path,
        bravepi_enabled = config.bravepi.is_some(),
        bravepi_port = config.bravepi.as_ref().map(|b| b.port.as_str()).unwrap_or("N/A"),
        rpi_local_enabled = config.rpi_local.is_some(),
        rpi_local_bus_path = config.rpi_local.as_ref().map(|r| r.bus_path.as_str()).unwrap_or("N/A"),
        rpi_local_poll_interval_ms = config.rpi_local.as_ref().map(|r| r.poll_interval_ms).unwrap_or(0),
        "effective config"
    );

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(run(config));
}

async fn run(config: config::GatewayConfig) {
    let engine = Engine::new();
    let mut host = AdapterHost::new();

    // BravePI mainboard adapter
    if let Some(bp) = &config.bravepi {
        match bravepi_mainboard_adapter::task::start(bp.port.clone()) {
            Ok(h) => {
                tracing::info!(adapter_id = %h.id, port = %bp.port, "BravePI mainboard adapter started");
                let parts = h.into_parts();
                host.register(
                    parts.id,
                    parts.event_rx,
                    {
                        let sh = parts.shutdown;
                        move || Box::pin(async move { sh.shutdown().await })
                    },
                )
                .expect("duplicate adapter ID");
            }
            Err(e) => {
                tracing::error!(error = %e, port = %bp.port, "Failed to start BravePI mainboard adapter");
                std::process::exit(1);
            }
        }
    } else {
        tracing::info!("BravePI mainboard adapter disabled");
    }

    // RPi local I2C adapter
    if let Some(rpi) = &config.rpi_local {
        let targets = hardcoded_rpi_local_targets();
        tracing::info!(
            bus_path = %rpi.bus_path,
            poll_interval_ms = rpi.poll_interval_ms,
            target_count = targets.len(),
            "rpi-local using hardcoded targets: MCP9600@0x60(K-type), OPT3001@0x44 (until auto-detection #35)"
        );
        let adapter_config = rpi_local_adapter::RpiLocalConfig {
            bus_path: rpi.bus_path.clone(),
            poll_interval_ms: rpi.poll_interval_ms,
            targets,
        };
        // Preflight: catch driver-level validation before spawning background tasks
        if let Err(e) = rpi_local_adapter::validate(&adapter_config) {
            tracing::error!(error = %e, "RPi local adapter config validation failed");
            std::process::exit(1);
        }
        match rpi_local_adapter::start(adapter_config) {
            Ok(rpi_handle) => {
                tracing::info!(adapter_id = %rpi_handle.id, "RPi local adapter started");
                let parts = rpi_handle.into_parts();
                host.register(
                    parts.id,
                    parts.event_rx,
                    {
                        let sh = parts.shutdown;
                        move || Box::pin(async move { sh.shutdown().await })
                    },
                )
                .expect("duplicate adapter ID");
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    bus_path = %rpi.bus_path,
                    "Failed to start RPi local adapter"
                );
                std::process::exit(1);
            }
        }
    } else {
        tracing::info!("RPi local adapter disabled");
    }

    // Unified fan-in loop
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutdown signal received");
                break;
            }
            event = host.next_event() => {
                match event {
                    Some(AdapterHostEvent::Event(ev)) => {
                        tracing::debug!(
                            adapter = %ev.adapter_id,
                            event = ?ev.event,
                            "Adapter event"
                        );
                        engine.apply(ev).await;
                    }
                    Some(AdapterHostEvent::AdapterClosed(id)) => {
                        tracing::warn!(
                            adapter = %id,
                            "Adapter channel closed unexpectedly"
                        );
                    }
                    None => {
                        tracing::info!("All adapter channels closed");
                        break;
                    }
                }
            }
        }
    }

    host.shutdown_all().await;

    let devices = engine.devices().await;
    tracing::info!(device_count = devices.len(), "Engine state at shutdown");
}

/// Hardcoded sensor targets for the v1 RPi4B hardware profile.
///
/// Deployment inventory — lives in the gateway composition root, not the
/// adapter crate. Replaced by #35 auto-detection or #23 device-config-service.
fn hardcoded_rpi_local_targets() -> Vec<rpi_local_adapter::RpiLocalTarget> {
    use rpi_local_adapter::ThermocoupleType;
    vec![
        rpi_local_adapter::RpiLocalTarget::MCP9600 {
            address: 0x60,
            thermocouple_type: ThermocoupleType::K,
        },
        rpi_local_adapter::RpiLocalTarget::OPT3001 {
            address: 0x44,
        },
    ]
}
```

- [ ] **Step 2: Remove the old rpi_local_config function**

The old `rpi_local_config()` function and the direct `BRAVEPI_PORT` env var reading are now replaced by the config system. Verify they are no longer in the new `main.rs`.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p iotkit-gateway`
Expected: compiles with no errors

- [ ] **Step 4: Run all workspace tests**

Run: `cargo test --workspace -- --test-threads=1`
Expected: all tests pass (single-threaded because config tests use unsafe env mutation)

- [ ] **Step 5: Commit**

```bash
git add iotkit-gateway/src/main.rs
git commit -m "feat(iotkit-gateway): integrate config::load() replacing hardcoded values"
```

---

## Self-Review

**Spec coverage check:**
- ✅ TOML schema (`[gateway]`, `[adapters.bravepi]`, `[adapters.rpi_local]`) — Task 2
- ✅ ENV override rules (all 6 vars) — Task 4
- ✅ Default values — Task 5 (resolve)
- ✅ Raw/Resolved types — Task 2
- ✅ Config pipeline (load_raw → apply_env → resolve) — Tasks 3, 4, 5, 6
- ✅ Validation rules — Task 5
- ✅ ConfigError — Task 2
- ✅ Gateway integration — Task 8
- ✅ Adapter ID unchanged (rpi-local:default) — Task 8 (no changes)
- ✅ Startup failure policy (fatal for enabled adapters) — Task 8
- ✅ Sensor target transition (hardcoded_rpi_local_targets in gateway) — Task 8
- ✅ Preflight validation (rpi_local_adapter::validate) — Task 7 + Task 8
- ✅ At least one adapter enabled validation — Task 5
- ✅ Target set logging — Task 8
- ✅ TOML file location (--config / ENV / implicit / defaults) — Task 6
- ✅ Config source logging — Task 8
- ✅ ENV parse error messages with var name — Task 4
- ✅ deny_unknown_fields — Task 2
- ✅ Testing strategy — covered across Tasks 2-6
- ✅ ENV > TOML precedence test — Task 4 (apply_env_overrides_toml_value)
- ✅ Integration test with real TOML file — Task 6 (load_integration_full_toml)
- ✅ rpi_local.enabled = false → None — Task 5 (resolve_rpi_local_disabled_explicit)
- ✅ Effective config logging with all adapter settings — Task 8
- ✅ IOTKIT_CONFIG_PATH positive test — Task 6 (load_with_env_config_path_valid_file)
- ✅ Implicit ./iotkit.toml positive test — Task 6 (load_with_implicit_iotkit_toml)
- ✅ --config takes precedence over IOTKIT_CONFIG_PATH — Task 6 (load_cli_arg_takes_precedence_over_env)
- ✅ --config without path errors — Task 6 (load_with_config_flag_but_no_path_errors)
- ✅ Invalid BRAVEPI_ENABLED bool error — Task 4 (apply_env_invalid_bool_returns_error)

**Placeholder scan:** No TBD/TODO/placeholders found.

**Type consistency:** `GatewayConfig`, `BravepiConfig`, `RpiLocalResolvedConfig`, `ConfigSource`, `ConfigError` — consistent across all tasks. `load()` signature `&[String]` consistent in Task 6 and Task 8.

**Parallelizable tasks:** Task 7 (rpi-local-adapter validate()) is independent of Tasks 3-6 (config.rs) and can be executed in parallel.

**Rust 2024 edition note:** `std::env::set_var` / `remove_var` are `unsafe` in edition 2024 (Rust 1.94+). All env-mutating test helpers use `unsafe` blocks with `--test-threads=1` to ensure safety.
