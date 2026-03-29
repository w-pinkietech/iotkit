# Bootstrap Config Design (#20)

## Goal

Replace hardcoded configuration in `iotkit-gateway/src/main.rs` with TOML file + ENV override loading. Bootstrap-only scope: infrastructure settings needed to start adapters. Sensor targets are NOT in TOML — managed by SQLite + auto-detection in later issues.

## Architecture

Gateway gains a `config` module that implements a three-stage pipeline: TOML parse → ENV merge → validated config. The gateway `main.rs` consumes only the validated `GatewayConfig` and converts it to adapter-specific config structs. Adapters themselves are unchanged — they do not know about TOML.

## TOML Schema

```toml
[gateway]
db_path = "iotkit.db"

[adapters.bravepi]
enabled = true
port = "/dev/ttyAMA0"

[adapters.rpi_local]
enabled = true
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
```

### Sections

- **`[gateway]`** — Cross-cutting infrastructure. `db_path` for SQLite (consumed by later issues #21+).
- **`[adapters.bravepi]`** — BravePI mainboard adapter. `enabled` flag, serial port path.
- **`[adapters.rpi_local]`** — RPi local I2C adapter. `enabled` flag, single I2C bus path (v1), poll interval.

### Design Decisions

- **`[adapters.*]` namespace**: Adapter-specific sections under a shared namespace. Each adapter owns its config shape. Future platforms (ESP32, RISC-V) add new TOML sections like `[adapters.esp32_ble]` without touching existing TOML sections. **Note:** adding a new adapter still requires changes to `RawAdaptersConfig`, `apply_env()`, `resolve()`, and `main.rs` startup wiring — TOML extensibility does not imply zero-touch code extensibility.
- **No sensor targets**: TOML holds only infrastructure. Sensor inventory, overrides, and per-sensor settings are managed by SQLite (#23 device-config-service).
- **No MQTT config**: Deferred to #28 notification-service.
- **No GPIO config**: No GPIO adapter exists yet. Added when GPIO adapter is implemented (YAGNI).
- **No API-driven config changes**: Bootstrap config is read at startup only. Changes require file edit + restart.
- **Single bus per adapter instance (v1)**: `bus_path` is singular. Multi-bus support requires instance-aware `AdapterId` design (#33 DeviceKey) and is deferred. When multi-bus is needed, `bus_path` migrates to `bus_paths` list or `[[adapters.rpi_local.buses]]` array-of-tables.

## ENV Override Rules

| ENV Variable | TOML Field | Format |
|---|---|---|
| `IOTKIT_DB_PATH` | `gateway.db_path` | String |
| `BRAVEPI_ENABLED` | `adapters.bravepi.enabled` | `true`/`false`/`1`/`0` |
| `BRAVEPI_PORT` | `adapters.bravepi.port` | String |
| `RPI_LOCAL_ENABLED` | `adapters.rpi_local.enabled` | `true`/`false`/`1`/`0` |
| `RPI_LOCAL_BUS_PATH` | `adapters.rpi_local.bus_path` | String |
| `RPI_LOCAL_POLL_INTERVAL_MS` | `adapters.rpi_local.poll_interval_ms` | Integer |

**Precedence:** ENV > TOML > hardcoded defaults.

**`BRAVEPI_PORT`** keeps its existing name for backward compatibility with current deployments.

## Default Values (no TOML, no ENV)

| Field | Default |
|---|---|
| `gateway.db_path` | `"iotkit.db"` |
| `adapters.bravepi.enabled` | `true` |
| `adapters.bravepi.port` | `"/dev/ttyAMA0"` |
| `adapters.rpi_local.enabled` | `false` |
| `adapters.rpi_local.bus_path` | `"/dev/i2c-1"` |
| `adapters.rpi_local.poll_interval_ms` | `1000` |

When no TOML file exists, the system runs with all defaults (same behavior as current hardcoded state, except `rpi_local.enabled` defaults to `false` matching current `RPI_LOCAL_ENABLED` behavior).

## Rust Types

### Raw Config (serde deserialization target)

```rust
// iotkit-gateway/src/config.rs

#[derive(Debug, Deserialize)]
pub struct RawConfig {
    #[serde(default)]
    pub gateway: RawGatewayConfig,
    #[serde(default)]
    pub adapters: RawAdaptersConfig,
}

#[derive(Debug, Deserialize, Default)]
pub struct RawGatewayConfig {
    pub db_path: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct RawAdaptersConfig {
    pub bravepi: Option<RawBravepiConfig>,
    pub rpi_local: Option<RawRpiLocalConfig>,
}

#[derive(Debug, Deserialize)]
pub struct RawBravepiConfig {
    pub enabled: Option<bool>,
    pub port: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RawRpiLocalConfig {
    pub enabled: Option<bool>,
    pub bus_path: Option<String>,
    pub poll_interval_ms: Option<u64>,
}
```

### Resolved Config (validated, no Option)

```rust
pub struct GatewayConfig {
    pub config_source: ConfigSource,                // for logging
    pub db_path: String,
    pub bravepi: Option<BravepiConfig>,             // None when enabled=false
    pub rpi_local: Option<RpiLocalResolvedConfig>,  // None when enabled=false
}

/// Tracks which TOML file was loaded (not the full effective provenance).
/// ENV overrides are logged separately in the startup "effective config" log line.
#[derive(Debug)]
pub enum ConfigSource {
    CliArg(PathBuf),
    EnvVar(PathBuf),
    ImplicitFile(PathBuf),
    DefaultsOnly,
}

pub struct BravepiConfig {
    pub port: String,
}

pub struct RpiLocalResolvedConfig {
    pub bus_path: String,             // non-empty
    pub poll_interval_ms: u64,        // > 0
}
```

## Config Pipeline

```
TOML file (optional) → RawConfig (serde) → apply_env() → resolve() → GatewayConfig
```

1. **`load_raw(path: Option<&Path>, explicit: bool)`** — Read and parse TOML file. If `explicit` is true and the file does not exist, return `ConfigError::Io`. If `explicit` is false and the file does not exist, return default `RawConfig`. If file exists but is invalid TOML or has unknown fields, return error.
2. **`apply_env(raw: &mut RawConfig)`** — For each ENV variable, if set, overwrite the corresponding `Option` field. ENV parse failures include the variable name and raw value in the error message.
3. **`resolve(raw: RawConfig) -> Result<GatewayConfig, ConfigError>`** — Apply defaults to remaining `None` fields. Validate constraints. Return `GatewayConfig` or `ConfigError`.

### Validation Rules

- `db_path`: non-empty string
- `bravepi.port`: if enabled, non-empty string
- `rpi_local.bus_path`: if enabled, non-empty string
- `rpi_local.poll_interval_ms`: if enabled, > 0
- At least one adapter must be enabled (both disabled → `ConfigError::Validation`). A gateway with zero adapters has no work to do; this is almost certainly a misconfiguration.

### ConfigError

```rust
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid config: {0}")]
    Validation(String),
}
```

## Gateway Integration

`main.rs` changes:

```rust
// Before (current)
let port_path = std::env::var("BRAVEPI_PORT").unwrap_or_else(|_| "/dev/ttyAMA0".to_string());
// ... hardcoded rpi_local_config()

// After
let args: Vec<String> = std::env::args().collect();
let config = config::load(&args)?;  // parse --config, then TOML + ENV + validate
tracing::info!(?config.config_source, "loaded gateway config");

// BravePI — started when enabled (true by default)
if let Some(bp) = &config.bravepi {
    let bravepi = bravepi_mainboard_adapter::task::start(bp.port.clone());
    // ... register with AdapterHost
}

// RPi local — single bus, started when enabled
if let Some(rpi) = &config.rpi_local {
    let targets = hardcoded_rpi_local_targets();  // gateway-owned deployment inventory
    let adapter_config = rpi_local_adapter::RpiLocalConfig {
        bus_path: rpi.bus_path.clone(),
        poll_interval_ms: rpi.poll_interval_ms,
        targets,
    };
    // Preflight: catch driver-level validation before spawning background tasks.
    // Uses adapter-level public API; PollingAdapterConfig conversion stays private.
    rpi_local_adapter::validate(&adapter_config)?;
    match rpi_local_adapter::start(adapter_config) {
        Ok(h) => {
            tracing::info!(adapter_id = %h.id, "RPi local adapter started");
            // ... register with AdapterHost
        }
        Err(e) => {
            // enabled=true but failed to start → fatal (explicit intent, not silent)
            tracing::error!(bus_path = %rpi.bus_path, error = %e, "failed to start rpi-local adapter");
            std::process::exit(1);
        }
    }
}
```

### Adapter ID

v1 uses a single bus, so `AdapterId` remains `rpi-local:default` (unchanged from current). Multi-bus support is deferred to #33 (DeviceKey bus identity), which will design instance-aware adapter IDs.

### Startup Failure Policy

- **BravePI**: if enabled and `start()` fails synchronously, gateway exits (fatal — same as current behavior). **Known limitation:** `bravepi_mainboard_adapter::task::start()` currently returns `Ok` before the serial port is actually opened (port open happens asynchronously in the reader thread). This means a bad port path is not caught at startup. Fixing this (synchronous initial-open validation) is tracked separately as a follow-up to the transport retry design; this spec does not change the existing behavior.
- **RPi local**: if enabled and `start()` fails, gateway exits (fatal — explicit user intent to enable this adapter). When disabled (`enabled=false`), adapter is skipped entirely.
- **Config source logging**: on startup, log which config source was used (`--config`, `IOTKIT_CONFIG_PATH`, `./iotkit.toml`, or defaults-only) and the effective resolved values for all adapter settings.
- **ENV parse errors**: `ConfigError::Validation` includes the env var name and raw value for every override parse failure (e.g., `"invalid value for RPI_LOCAL_POLL_INTERVAL_MS: 'abc'"`).

### Preflight Validation

After `resolve()` returns a valid `GatewayConfig`, the gateway performs a **preflight validation** step before calling `start()` on any adapter. For rpi-local, this means calling `rpi_local_adapter::validate(&RpiLocalConfig)` — a new public function that converts to `PollingAdapterConfig` internally and delegates to `iotkit_polling_adapter_runtime::validate_config()`. This catches driver-level constraints (e.g., OPT3001 rejects `poll_interval_ms < 200`) before the adapter spawns any background tasks. `start()` retains its own `validate_config()` call as defense-in-depth. The `PollingAdapterConfig` conversion remains private to the adapter crate. `config::load()` accepts `&[String]` (CLI args) and owns `--config` parsing internally.

### Sensor Target Transition

This spec preserves the current hardcoded sensor targets until #35 auto-detection or #23 device-config-service provides a replacement. The adapter is NOT started with empty targets — that would regress the "device never discovered" lifecycle contract. The hardcoded target set (MCP9600@0x60 K-type, OPT3001@0x44) is defined in gateway composition code (`main.rs`), not in the adapter crate. The adapter crate owns `RpiLocalTarget`, driver construction, and validation, but "which sensors are on this board" is deployment inventory belonging to the composition root.

**Logging**: At startup, the gateway logs the hardcoded target set explicitly so operators know which sensors are expected: `"rpi-local using hardcoded targets: MCP9600@0x60(K-type), OPT3001@0x44 (until auto-detection #35)"`.

**Known limitation — partial config surface**: This spec intentionally creates a transitional state where bus/interval are configurable but sensor targets are hardcoded. An operator who changes `bus_path` to a different I2C bus must ensure the hardcoded targets (MCP9600 at 0x60, OPT3001 at 0x44) exist on that bus. The MCP9600 driver writes thermocouple type (K) to hardware on probe, so mismatched targets produce wrong readings, not errors. This is acceptable for v1 because: (a) the project has a single known hardware configuration, (b) the target set is logged at startup, and (c) #35 auto-detection will eliminate this gap. If multiple hardware configurations emerge before #35, sensor targets must be exposed in config immediately.

## TOML File Location

`config::load()` searches in order:
1. `--config <path>` CLI argument (if provided) — **must exist, error if missing**
2. `IOTKIT_CONFIG_PATH` ENV variable — **must exist, error if missing**
3. `./iotkit.toml` (working directory) — optional probe, silently skipped if absent

If none found via the implicit probe (step 3), all defaults are used (no error). Explicit paths (steps 1-2) always error on missing file to prevent silent misconfiguration.

**Working directory dependency:** Both the implicit config path (`./iotkit.toml`) and the default `db_path` (`"iotkit.db"`) are cwd-relative. In production, the systemd unit file sets `WorkingDirectory=/opt/iotkit` (or equivalent) to provide a stable anchor. This is acceptable for v1 because deployment is single-target Raspberry Pi with a known launch configuration. When containerized or multi-instance deployment is needed (#31), absolute path defaults or a `--base-dir` flag should be introduced.

## Dependencies

- **New crate dependencies for `iotkit-gateway`**: `serde` (with `derive`), `toml`
- **Existing crate `thiserror`**: for `ConfigError`

## Scope Boundaries

### In Scope
- `config.rs` module in `iotkit-gateway`
- TOML parse + ENV merge + validation pipeline
- `GatewayConfig` type consumed by `main.rs`
- Update `main.rs` to use config instead of hardcoded values
- Unit tests for parse, ENV override, validation, defaults
- Refactor `rpi_local_config()` in `main.rs` into a `hardcoded_rpi_local_targets()` helper (gateway-owned)
- Preflight validation: call `validate_config()` on constructed adapter config before `start()`
- Config source + resolved values logging at startup

### Out of Scope
- Sensor targets in TOML (→ #23 device-config-service + #35 auto-detection)
- MQTT/notification config (→ #28)
- GPIO adapter config (YAGNI)
- API-driven config changes
- Default config file generation (→ #31 deployment)
- SQLite path usage (→ #21, config only stores the path)

## Testing Strategy

- **Unit tests** in `config.rs`:
  - Parse valid TOML → correct `RawConfig`
  - Parse empty/missing TOML → defaults
  - ENV overrides take precedence over TOML
  - `RPI_LOCAL_BUS_PATH` override
  - Validation rejects: empty `bus_path`, `poll_interval_ms = 0`, empty strings
  - ENV parse failure includes var name and raw value in error message
  - `enabled = false` → `rpi_local` is `None` in resolved config
  - `bravepi.enabled = false` → `bravepi` is `None` in resolved config
  - Unknown TOML fields → error (strict parsing via `serde(deny_unknown_fields)`)
  - Explicit `--config` path to non-existent file → error
  - Explicit `IOTKIT_CONFIG_PATH` to non-existent file → error
  - Implicit `./iotkit.toml` missing → defaults (no error)
  - Both adapters disabled → `ConfigError::Validation` (at least one required)
- **Integration test**: Load a real TOML file, verify `GatewayConfig` fields
