# Sub-project F: RPi Local Adapter v1 / Adapter Naming — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename `bravepi-adapter` → `bravepi-mainboard-adapter`, then add `rpi-local-adapter` as a second concrete I2C polling adapter for RPi-local sensors.

**Architecture:** Config-driven I2C polling adapter with single-task `tokio::select!` loop. Blocking I2C I/O via `spawn_blocking` per poll cycle. `bravepi-sensors` used as shared decode crate. `apply_outcomes()` separates pure state transition logic from I/O for testability.

**Tech Stack:** Rust 2024, tokio (sync, rt, macros, time), rpi4b-transport (I2cTransport), bravepi-sensors (MCP9600, OPT3001), iotkit-core-types, tracing

**Spec:** `docs/superpowers/specs/2026-03-27-rpi-local-adapter-design.md`

---

## File Structure

### Rename (Task 1)

| Action | Path | Responsibility |
|--------|------|----------------|
| Rename dir | `bravepi-adapter/` → `bravepi-mainboard-adapter/` | directory rename |
| Modify | `bravepi-mainboard-adapter/Cargo.toml` | package name |
| Modify | `bravepi-mainboard-adapter/src/task/handle.rs:58` | AdapterId string |
| Modify | `bravepi-mainboard-adapter/poc/Cargo.toml` | dep path |
| Modify | `Cargo.toml` (workspace root) | member paths |
| Modify | `iotkit-gateway/Cargo.toml` | dep name + path |
| Modify | `iotkit-gateway/src/main.rs` | use + log strings |

### New crate (Tasks 2–8)

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `rpi-local-adapter/Cargo.toml` | crate manifest |
| Create | `rpi-local-adapter/src/lib.rs` | public API: `start()`, `AdapterHandle`, re-exports |
| Create | `rpi-local-adapter/src/config.rs` | `RpiLocalConfig`, `SensorTarget`, `SensorKind`, `validate_config()` |
| Create | `rpi-local-adapter/src/polling_loop.rs` | async polling loop, `TargetState`, `PollOutcome`, `apply_outcomes()` |
| Create | `rpi-local-adapter/src/sensors/mod.rs` | `probe()` / `read()` / `sensor_ic_name()` dispatch |
| Create | `rpi-local-adapter/src/sensors/mcp9600.rs` | `probe_mcp9600()` / `read_mcp9600()` |
| Create | `rpi-local-adapter/src/sensors/opt3001.rs` | `probe_opt3001()` / `read_opt3001()` |

### Gateway integration (Task 9)

| Action | Path | Responsibility |
|--------|------|----------------|
| Modify | `Cargo.toml` (workspace root) | add member |
| Modify | `iotkit-gateway/Cargo.toml` | add dep |
| Modify | `iotkit-gateway/src/main.rs` | start rpi-local, fan-in loop |

---

## Task 1: Rename bravepi-adapter → bravepi-mainboard-adapter

**Files:**
- Rename: `bravepi-adapter/` → `bravepi-mainboard-adapter/`
- Modify: `bravepi-mainboard-adapter/Cargo.toml`
- Modify: `bravepi-mainboard-adapter/src/task/handle.rs:58`
- Modify: `bravepi-mainboard-adapter/src/task/convert.rs:27,70`
- Modify: `bravepi-mainboard-adapter/src/task/event_loop.rs:161`
- Modify: `bravepi-mainboard-adapter/poc/Cargo.toml`
- Modify: `Cargo.toml` (workspace root)
- Modify: `iotkit-gateway/Cargo.toml`
- Modify: `iotkit-gateway/src/main.rs`

- [ ] **Step 1: Rename directory**

```bash
mv bravepi-adapter bravepi-mainboard-adapter
```

- [ ] **Step 2: Update package name in bravepi-mainboard-adapter/Cargo.toml**

Change line 2:

```toml
[package]
name = "bravepi-mainboard-adapter"
version = "0.1.0"
edition = "2024"

[dependencies]
iotkit-core-types = { path = "../core/types" }
rpi4b-transport = { path = "../rpi4b-driver/transport" }
bravepi-codec = { path = "codec" }
bravepi-sensors = { path = "sensors" }
tokio = { version = "1", features = ["sync", "rt", "macros", "time"] }
tracing = "0.1"
```

- [ ] **Step 3: Update AdapterId in handle.rs**

In `bravepi-mainboard-adapter/src/task/handle.rs`, change line 58:

```rust
    let id = AdapterId::new(format!("bravepi-mainboard:{}", port_path));
```

- [ ] **Step 3b: Update DeviceKey prefix in convert.rs and event_loop.rs**

In `bravepi-mainboard-adapter/src/task/convert.rs`, change `"bravepi:"` to `"bravepi-mainboard:"` in all `format!` calls that generate DeviceKey strings:

```rust
// convert.rs:27
format!("bravepi-mainboard:{}:{}", transmitter_id, handler.key_suffix)
// convert.rs:70
DeviceKey::new(format!("bravepi-mainboard:{}:{}", device_number, h.key_suffix))
```

In `bravepi-mainboard-adapter/src/task/event_loop.rs:161`:

```rust
format!("bravepi-mainboard:{}:{}", cfg.device_number, handler.key_suffix)
```

- [ ] **Step 4: Update poc/Cargo.toml dependency path**

`bravepi-mainboard-adapter/poc/Cargo.toml`:

```toml
[package]
name = "bravepi-poc"
version = "0.1.0"
edition = "2024"

[dependencies]
iotkit-core-types = { path = "../../core/types" }
bravepi-mainboard-adapter = { path = ".." }
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

Also update `bravepi-mainboard-adapter/poc/src/main.rs` — replace `bravepi_adapter::` with `bravepi_mainboard_adapter::`:

```bash
# Check what needs updating:
grep -r "bravepi_adapter" bravepi-mainboard-adapter/poc/src/
```

Apply the rename in all matching lines.

- [ ] **Step 5: Update workspace root Cargo.toml**

```toml
[workspace]
members = [
    "core/types",
    "core/engine",
    "rpi4b-driver/transport",
    "bravepi-mainboard-adapter",
    "bravepi-mainboard-adapter/codec",
    "bravepi-mainboard-adapter/sensors",
    "bravepi-mainboard-adapter/poc",
    "iotkit-gateway",
]
resolver = "3"
```

- [ ] **Step 6: Update iotkit-gateway/Cargo.toml**

```toml
[package]
name = "iotkit-gateway"
version = "0.1.0"
edition = "2024"

[dependencies]
iotkit-core-types = { path = "../core/types" }
iotkit-core-engine = { path = "../core/engine" }
bravepi-mainboard-adapter = { path = "../bravepi-mainboard-adapter" }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

- [ ] **Step 7: Update iotkit-gateway/src/main.rs**

Replace `bravepi_adapter` with `bravepi_mainboard_adapter`:

```rust
//! iotkit-gateway: composition root。
//! adapter を起動し、core/engine に event を渡す。

use iotkit_core_engine::{Engine, EngineEvent};
use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let port_path =
        std::env::var("BRAVEPI_PORT").unwrap_or_else(|_| "/dev/ttyAMA0".to_string());

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(run(port_path));
}

async fn run(port_path: String) {
    let engine = Engine::new();

    let mut handle = match bravepi_mainboard_adapter::task::start(port_path) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "Failed to start BravePI mainboard adapter");
            std::process::exit(1);
        }
    };
    let adapter_id = handle.id.clone();
    tracing::info!(adapter_id = %adapter_id, "BravePI mainboard adapter started");

    loop {
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutdown signal received");
                if let Err(e) = handle.shutdown().await {
                    tracing::error!(error = %e, "Adapter shutdown error");
                }
                break;
            }
            event = handle.event_rx.recv() => {
                match event {
                    Some(ev) => {
                        tracing::debug!(event = ?ev, "Received adapter event");
                        engine.apply(EngineEvent {
                            adapter_id: adapter_id.clone(),
                            event: ev,
                        }).await;
                    }
                    None => {
                        tracing::info!("Adapter event channel closed");
                        break;
                    }
                }
            }
        }
    }

    let devices = engine.devices().await;
    tracing::info!(device_count = devices.len(), "Engine state at shutdown");
}
```

- [ ] **Step 8: Build and run existing tests**

```bash
cargo build --workspace 2>&1
cargo test --workspace 2>&1
```

Expected: all compile, all tests pass. The rename is transparent to runtime behavior.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor: rename bravepi-adapter to bravepi-mainboard-adapter

Clarifies that this adapter's boundary is the BravePI mainboard UART
connection, not all BravePI-related functionality. Prepares for adding
rpi-local-adapter as a second concrete adapter.

- Rename crate directory and package name
- Update AdapterId to bravepi-mainboard:{port_path}
- Update DeviceKey prefix from bravepi: to bravepi-mainboard:
- Update workspace members and gateway dependency"
```

---

## Task 2: Create rpi-local-adapter crate skeleton with config and validation

**Files:**
- Create: `rpi-local-adapter/Cargo.toml`
- Create: `rpi-local-adapter/src/lib.rs`
- Create: `rpi-local-adapter/src/config.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Write the failing test for validate_config**

Create `rpi-local-adapter/src/config.rs`:

```rust
//! Adapter configuration and validation.

/// Re-export ThermocoupleType so users don't depend on bravepi-sensors directly.
pub use bravepi_sensors::mcp9600::ThermocoupleType;

/// Adapter configuration. Passed to `start()`.
#[derive(Debug, Clone)]
pub struct RpiLocalConfig {
    /// I2C bus path, e.g. "/dev/i2c-1".
    pub bus_path: String,
    /// Polling interval in milliseconds. Must be > 0.
    pub poll_interval_ms: u64,
    /// Sensor targets to probe and poll.
    pub targets: Vec<SensorTarget>,
}

/// A single sensor target on the I2C bus.
#[derive(Debug, Clone)]
pub struct SensorTarget {
    /// 7-bit I2C address.
    pub address: u8,
    /// Sensor IC kind and its configuration.
    pub kind: SensorKind,
}

/// Sensor IC type with IC-specific configuration.
#[derive(Debug, Clone)]
pub enum SensorKind {
    MCP9600 {
        thermocouple_type: ThermocoupleType,
    },
    OPT3001,
}

/// Returns the IC name string for DeviceKey generation and duplicate detection.
pub fn sensor_ic_name(kind: &SensorKind) -> &'static str {
    match kind {
        SensorKind::MCP9600 { .. } => "mcp9600",
        SensorKind::OPT3001 => "opt3001",
    }
}

/// Validates config before starting the adapter.
pub fn validate_config(config: &RpiLocalConfig) -> Result<(), String> {
    if config.bus_path.is_empty() {
        return Err("bus_path must not be empty".to_string());
    }
    if config.poll_interval_ms == 0 {
        return Err("poll_interval_ms must be > 0".to_string());
    }

    let mut seen_addresses = std::collections::HashSet::new();
    for target in &config.targets {
        if !(0x08..=0x77).contains(&target.address) {
            return Err(format!(
                "address 0x{:02x} outside valid 7-bit I2C range (0x08..=0x77)",
                target.address,
            ));
        }
        if !seen_addresses.insert(target.address) {
            return Err(format!(
                "duplicate address 0x{:02x}: same bus cannot have two devices at one address",
                target.address,
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> RpiLocalConfig {
        RpiLocalConfig {
            bus_path: "/dev/i2c-1".to_string(),
            poll_interval_ms: 1000,
            targets: vec![
                SensorTarget {
                    address: 0x60,
                    kind: SensorKind::MCP9600 {
                        thermocouple_type: ThermocoupleType::K,
                    },
                },
                SensorTarget {
                    address: 0x44,
                    kind: SensorKind::OPT3001,
                },
            ],
        }
    }

    #[test]
    fn valid_config_passes() {
        assert!(validate_config(&valid_config()).is_ok());
    }

    #[test]
    fn zero_poll_interval_is_rejected() {
        let mut config = valid_config();
        config.poll_interval_ms = 0;
        let err = validate_config(&config).unwrap_err();
        assert!(err.contains("poll_interval_ms"), "error: {}", err);
    }

    #[test]
    fn duplicate_address_is_rejected() {
        let mut config = valid_config();
        config.targets.push(SensorTarget {
            address: 0x60,
            kind: SensorKind::OPT3001,
        });
        let err = validate_config(&config).unwrap_err();
        assert!(err.contains("duplicate"), "error: {}", err);
    }

    #[test]
    fn address_out_of_range_is_rejected() {
        let mut config = valid_config();
        config.targets[0].address = 0x80;
        let err = validate_config(&config).unwrap_err();
        assert!(err.contains("outside valid"), "error: {}", err);
    }

    #[test]
    fn empty_bus_path_is_rejected() {
        let mut config = valid_config();
        config.bus_path = String::new();
        let err = validate_config(&config).unwrap_err();
        assert!(err.contains("bus_path"), "error: {}", err);
    }

    #[test]
    fn empty_targets_is_valid() {
        let mut config = valid_config();
        config.targets.clear();
        assert!(validate_config(&config).is_ok());
    }
}
```

- [ ] **Step 2: Create Cargo.toml**

Create `rpi-local-adapter/Cargo.toml`:

```toml
[package]
name = "rpi-local-adapter"
version = "0.1.0"
edition = "2024"

[dependencies]
iotkit-core-types = { path = "../core/types" }
rpi4b-transport = { path = "../rpi4b-driver/transport" }
bravepi-sensors = { path = "../bravepi-mainboard-adapter/sensors" }
tokio = { version = "1", features = ["sync", "rt", "macros", "time"] }
tracing = "0.1"
```

- [ ] **Step 3: Create lib.rs stub**

Create `rpi-local-adapter/src/lib.rs`:

```rust
//! rpi-local-adapter: RPi ローカル直結 hardware の adapter。
//! v1 は I2C slice のみ。

pub mod config;

pub use config::{RpiLocalConfig, SensorKind, SensorTarget, ThermocoupleType};
```

- [ ] **Step 4: Add workspace member**

In workspace root `Cargo.toml`, add `"rpi-local-adapter"` to members:

```toml
[workspace]
members = [
    "core/types",
    "core/engine",
    "rpi4b-driver/transport",
    "bravepi-mainboard-adapter",
    "bravepi-mainboard-adapter/codec",
    "bravepi-mainboard-adapter/sensors",
    "bravepi-mainboard-adapter/poc",
    "rpi-local-adapter",
    "iotkit-gateway",
]
resolver = "3"
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p rpi-local-adapter 2>&1
```

Expected: 6 tests pass (valid_config_passes, zero_poll_interval_is_rejected, duplicate_address_is_rejected, address_out_of_range_is_rejected, empty_bus_path_is_rejected, empty_targets_is_valid).

- [ ] **Step 6: Commit**

```bash
git add rpi-local-adapter/ Cargo.toml
git commit -m "feat(rpi-local-adapter): add crate skeleton with config and validation

- RpiLocalConfig, SensorTarget, SensorKind types
- validate_config() with poll_interval and duplicate target checks
- sensor_ic_name() for DeviceKey generation
- 5 unit tests for validation"
```

---

## Task 3: Per-sensor I2C probe and read functions

**Files:**
- Create: `rpi-local-adapter/src/sensors/mod.rs`
- Create: `rpi-local-adapter/src/sensors/mcp9600.rs`
- Create: `rpi-local-adapter/src/sensors/opt3001.rs`
- Modify: `rpi-local-adapter/src/lib.rs`

- [ ] **Step 1: Create MCP9600 probe/read**

Create `rpi-local-adapter/src/sensors/mcp9600.rs`:

```rust
//! MCP9600 I2C probe and read.

use iotkit_core_types::{ConnectionInfo, ConnectionKind, SensorIdentity, SensorReading};
use rpi4b_transport::{I2cConfig, I2cTransport};
use bravepi_sensors::mcp9600::{self, ThermocoupleType};
use std::collections::BTreeMap;

pub fn probe_mcp9600(
    bus: &str,
    addr: u8,
    thermocouple_type: ThermocoupleType,
) -> Result<SensorIdentity, String> {
    let mut t = I2cTransport::open(bus, &I2cConfig { address: addr as u16 })
        .map_err(|e| format!("I2C open 0x{:02x}: {}", addr, e))?;

    // Read device ID register
    let mut id_buf = [0u8; 2];
    t.read_register(mcp9600::REG_DEVICE_ID, &mut id_buf)
        .map_err(|e| format!("read REG_DEVICE_ID: {}", e))?;

    // Verify device ID (upper byte)
    if id_buf[0] != mcp9600::DEVICE_ID {
        return Err(format!(
            "MCP9600 device ID mismatch: expected 0x{:02x}, got 0x{:02x}",
            mcp9600::DEVICE_ID, id_buf[0],
        ));
    }

    // Write thermocouple type configuration
    let config_val = mcp9600::config_value(thermocouple_type);
    t.write_register(mcp9600::REG_SENSOR_CONFIGURATION, &[config_val])
        .map_err(|e| format!("write REG_SENSOR_CONFIGURATION: {}", e))?;

    let connection = ConnectionInfo {
        kind: ConnectionKind::I2c,
        parameters: BTreeMap::from([
            ("bus".into(), bus.to_string()),
            ("address".into(), format!("0x{:02x}", addr)),
        ]),
    };
    Ok(mcp9600::identity(connection))
}

pub fn read_mcp9600(bus: &str, addr: u8) -> Result<SensorReading, String> {
    let mut t = I2cTransport::open(bus, &I2cConfig { address: addr as u16 })
        .map_err(|e| format!("I2C open 0x{:02x}: {}", addr, e))?;

    let mut raw = [0u8; 2];
    t.read_register(mcp9600::REG_HOT_JUNCTION, &mut raw)
        .map_err(|e| format!("read REG_HOT_JUNCTION: {}", e))?;

    Ok(mcp9600::from_i2c_raw(&raw))
}
```

- [ ] **Step 2: Create OPT3001 probe/read**

Create `rpi-local-adapter/src/sensors/opt3001.rs`:

```rust
//! OPT3001 I2C probe and read.

use iotkit_core_types::{ConnectionInfo, ConnectionKind, SensorIdentity, SensorReading};
use rpi4b_transport::{I2cConfig, I2cTransport};
use bravepi_sensors::opt3001;
use std::collections::BTreeMap;

pub fn probe_opt3001(bus: &str, addr: u8) -> Result<SensorIdentity, String> {
    let mut t = I2cTransport::open(bus, &I2cConfig { address: addr as u16 })
        .map_err(|e| format!("I2C open 0x{:02x}: {}", addr, e))?;

    // Read device ID register
    let mut id_buf = [0u8; 2];
    t.read_register(opt3001::REG_DEVICE_ID, &mut id_buf)
        .map_err(|e| format!("read REG_DEVICE_ID: {}", e))?;

    // OPT3001 device ID is a u16 in big-endian register order.
    // Transport returns raw bytes [MSB, LSB], so use from_be_bytes.
    let device_id = u16::from_be_bytes(id_buf);
    if device_id != opt3001::DEVICE_ID {
        return Err(format!(
            "OPT3001 device ID mismatch: expected 0x{:04x}, got 0x{:04x}",
            opt3001::DEVICE_ID, device_id,
        ));
    }

    // Write init config to start measurement.
    // OPT3001 registers are big-endian on wire, but the existing parser
    // (from_i2c_raw) expects SMBus byte-swapped words. For write_register
    // we send raw bytes, so we use LE to match the SMBus word convention.
    let config_bytes = opt3001::INIT_CONFIG.to_le_bytes();
    t.write_register(opt3001::REG_CONFIG, &config_bytes)
        .map_err(|e| format!("write REG_CONFIG: {}", e))?;

    let connection = ConnectionInfo {
        kind: ConnectionKind::I2c,
        parameters: BTreeMap::from([
            ("bus".into(), bus.to_string()),
            ("address".into(), format!("0x{:02x}", addr)),
        ]),
    };
    Ok(opt3001::identity(connection))
}

pub fn read_opt3001(bus: &str, addr: u8) -> Result<SensorReading, String> {
    let mut t = I2cTransport::open(bus, &I2cConfig { address: addr as u16 })
        .map_err(|e| format!("I2C open 0x{:02x}: {}", addr, e))?;

    let mut raw = [0u8; 2];
    t.read_register(opt3001::REG_RESULT, &mut raw)
        .map_err(|e| format!("read REG_RESULT: {}", e))?;

    // Normalize to SMBus byte-swapped u16 for existing parser.
    let swapped = u16::from_le_bytes(raw);
    Ok(opt3001::from_i2c_raw(swapped))
}
```

- [ ] **Step 3: Create sensors/mod.rs dispatch**

Create `rpi-local-adapter/src/sensors/mod.rs`:

```rust
//! Per-sensor I2C probe and read dispatch.

pub mod mcp9600;
pub mod opt3001;

use iotkit_core_types::{SensorIdentity, SensorReading};
use crate::config::SensorKind;

pub fn probe(kind: &SensorKind, bus: &str, addr: u8) -> Result<SensorIdentity, String> {
    match kind {
        SensorKind::MCP9600 { thermocouple_type } => {
            mcp9600::probe_mcp9600(bus, addr, *thermocouple_type)
        }
        SensorKind::OPT3001 => opt3001::probe_opt3001(bus, addr),
    }
}

pub fn read(kind: &SensorKind, bus: &str, addr: u8) -> Result<SensorReading, String> {
    match kind {
        SensorKind::MCP9600 { .. } => mcp9600::read_mcp9600(bus, addr),
        SensorKind::OPT3001 => opt3001::read_opt3001(bus, addr),
    }
}
```

- [ ] **Step 4: Register module in lib.rs**

Update `rpi-local-adapter/src/lib.rs`:

```rust
//! rpi-local-adapter: RPi ローカル直結 hardware の adapter。
//! v1 は I2C slice のみ。

pub mod config;
mod sensors;

pub use config::{RpiLocalConfig, SensorKind, SensorTarget};
```

- [ ] **Step 5: Build**

```bash
cargo build -p rpi-local-adapter 2>&1
```

Expected: compiles. No unit tests for sensor I/O (requires hardware).

- [ ] **Step 6: Commit**

```bash
git add rpi-local-adapter/src/sensors/
git commit -m "feat(rpi-local-adapter): add per-sensor I2C probe and read

- probe_mcp9600: device ID check + thermocouple config write
- read_mcp9600: hot junction register → from_i2c_raw
- probe_opt3001: device ID check + init config write
- read_opt3001: result register → LE byte swap → from_i2c_raw
- SensorKind dispatch via probe()/read() in sensors/mod.rs"
```

---

## Task 4: PollOutcome and apply_outcomes with tests

**Files:**
- Create: `rpi-local-adapter/src/polling_loop.rs`
- Modify: `rpi-local-adapter/src/lib.rs`

- [ ] **Step 1: Write PollOutcome, TargetState, and apply_outcomes**

Create `rpi-local-adapter/src/polling_loop.rs`:

```rust
//! Polling loop internals: state management, outcome processing, and the async loop.

use iotkit_core_types::{AdapterCommand, AdapterEvent, DeviceKey, SensorIdentity, SensorReading};
use tokio::sync::mpsc;

use crate::config::{sensor_ic_name, RpiLocalConfig, SensorTarget};

/// Per-target discovery state.
#[derive(Debug, Clone)]
pub(crate) enum TargetState {
    /// Not yet discovered (DeviceDiscovered not sent).
    Pending,
    /// Discovery complete; holds the DeviceKey for event generation.
    Active(DeviceKey),
}

/// Result of a single target's probe or read within a spawn_blocking cycle.
#[derive(Debug)]
pub(crate) enum PollOutcome {
    Discovered {
        target_index: usize,
        key: DeviceKey,
        identity: SensorIdentity,
    },
    Reading {
        key: DeviceKey,
        reading: SensorReading,
    },
    ReadError {
        key: DeviceKey,
        message: String,
    },
    ProbeFailed {
        target_index: usize,
        message: String,
    },
}

/// Builds a DeviceKey from a SensorTarget.
pub(crate) fn device_key_for(target: &SensorTarget) -> DeviceKey {
    DeviceKey::new(format!(
        "i2c:0x{:02x}:{}",
        target.address,
        sensor_ic_name(&target.kind),
    ))
}

/// Pure function: applies poll outcomes to target states, returns events to send.
///
/// Rules:
/// - Discovered → state becomes Active, emit DeviceDiscovered
/// - Reading → emit SensorData (state unchanged)
/// - ReadError → emit AdapterError (state stays Active)
/// - ProbeFailed → log only (state stays Pending)
pub(crate) fn apply_outcomes(
    outcomes: Vec<PollOutcome>,
    states: &mut [TargetState],
) -> Vec<AdapterEvent> {
    let mut events = Vec::new();

    for outcome in outcomes {
        match outcome {
            PollOutcome::Discovered { target_index, key, identity } => {
                states[target_index] = TargetState::Active(key.clone());
                events.push(AdapterEvent::DeviceDiscovered {
                    device_key: key,
                    identity,
                });
            }
            PollOutcome::Reading { key, reading } => {
                events.push(AdapterEvent::SensorData {
                    device_key: key,
                    reading,
                    rssi: None,
                    battery_pct: None,
                });
            }
            PollOutcome::ReadError { key, message } => {
                events.push(AdapterEvent::AdapterError {
                    device_key: Some(key),
                    error: message,
                });
            }
            PollOutcome::ProbeFailed { target_index: _, message } => {
                tracing::warn!(error = %message, "Probe failed (no event)");
            }
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_types::{ConnectionInfo, ConnectionKind, SensorType};
    use std::collections::BTreeMap;

    fn test_identity() -> SensorIdentity {
        SensorIdentity {
            manufacturer: "Test".into(),
            ic_part_number: "MCP9600".into(),
            sensor_type: SensorType::Temperature,
            connection: ConnectionInfo {
                kind: ConnectionKind::I2c,
                parameters: BTreeMap::new(),
            },
        }
    }

    fn test_reading() -> SensorReading {
        SensorReading::new(SensorType::Temperature, vec![22.5], vec!["celsius"])
    }

    #[test]
    fn probe_success_transitions_to_active_and_emits_discovered() {
        let mut states = vec![TargetState::Pending];
        let key = DeviceKey::new("i2c:0x60:mcp9600");
        let outcomes = vec![PollOutcome::Discovered {
            target_index: 0,
            key: key.clone(),
            identity: test_identity(),
        }];

        let events = apply_outcomes(outcomes, &mut states);

        assert!(matches!(states[0], TargetState::Active(_)));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AdapterEvent::DeviceDiscovered { .. }));
    }

    #[test]
    fn read_success_emits_sensor_data() {
        let key = DeviceKey::new("i2c:0x60:mcp9600");
        let mut states = vec![TargetState::Active(key.clone())];
        let outcomes = vec![PollOutcome::Reading {
            key: key.clone(),
            reading: test_reading(),
        }];

        let events = apply_outcomes(outcomes, &mut states);

        assert!(matches!(states[0], TargetState::Active(_)));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AdapterEvent::SensorData { .. }));
    }

    #[test]
    fn read_failure_keeps_active_state_and_emits_error() {
        let key = DeviceKey::new("i2c:0x60:mcp9600");
        let mut states = vec![TargetState::Active(key.clone())];
        let outcomes = vec![PollOutcome::ReadError {
            key: key.clone(),
            message: "I/O error".into(),
        }];

        let events = apply_outcomes(outcomes, &mut states);

        assert!(matches!(states[0], TargetState::Active(_)));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AdapterEvent::AdapterError { .. }));
    }

    #[test]
    fn probe_failure_emits_no_event() {
        let mut states = vec![TargetState::Pending];
        let outcomes = vec![PollOutcome::ProbeFailed {
            target_index: 0,
            message: "device not found".into(),
        }];

        let events = apply_outcomes(outcomes, &mut states);

        assert!(matches!(states[0], TargetState::Pending));
        assert!(events.is_empty());
    }

    #[test]
    fn discovered_only_emits_discovered_no_read_in_same_cycle() {
        let mut states = vec![TargetState::Pending];
        let key = DeviceKey::new("i2c:0x60:mcp9600");
        let outcomes = vec![PollOutcome::Discovered {
            target_index: 0,
            key: key.clone(),
            identity: test_identity(),
        }];

        let events = apply_outcomes(outcomes, &mut states);

        assert!(matches!(states[0], TargetState::Active(_)));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AdapterEvent::DeviceDiscovered { .. }));
    }

    #[test]
    fn multiple_targets_independent() {
        let key_a = DeviceKey::new("i2c:0x60:mcp9600");
        let key_b = DeviceKey::new("i2c:0x44:opt3001");
        let mut states = vec![
            TargetState::Active(key_a.clone()),
            TargetState::Pending,
        ];
        let outcomes = vec![
            PollOutcome::Reading {
                key: key_a.clone(),
                reading: test_reading(),
            },
            PollOutcome::ProbeFailed {
                target_index: 1,
                message: "not found".into(),
            },
        ];

        let events = apply_outcomes(outcomes, &mut states);

        assert!(matches!(states[0], TargetState::Active(_)));
        assert!(matches!(states[1], TargetState::Pending));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AdapterEvent::SensorData { .. }));
    }
}
```

- [ ] **Step 2: Register module in lib.rs**

Update `rpi-local-adapter/src/lib.rs`:

```rust
//! rpi-local-adapter: RPi ローカル直結 hardware の adapter。
//! v1 は I2C slice のみ。

pub mod config;
mod polling_loop;
mod sensors;

pub use config::{RpiLocalConfig, SensorKind, SensorTarget};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p rpi-local-adapter 2>&1
```

Expected: 12 tests pass (6 config + 6 polling_loop).

- [ ] **Step 4: Commit**

```bash
git add rpi-local-adapter/src/polling_loop.rs rpi-local-adapter/src/lib.rs
git commit -m "feat(rpi-local-adapter): add PollOutcome, TargetState, apply_outcomes

Pure state transition logic separated from I/O:
- TargetState: Pending | Active(DeviceKey)
- PollOutcome: Discovered | Reading | ReadError | ProbeFailed
- apply_outcomes(): outcomes × states → events
- 7 unit tests covering all state transitions"
```

---

## Task 5: Blocking poll cycle function

**Files:**
- Modify: `rpi-local-adapter/src/polling_loop.rs`

- [ ] **Step 1: Add the blocking poll_cycle function**

Add to `rpi-local-adapter/src/polling_loop.rs`, above the `#[cfg(test)]` block:

```rust
/// Executes one poll cycle synchronously (called inside spawn_blocking).
/// For each target: Active → read, Pending → probe only (first read is next tick).
pub(crate) fn poll_cycle(
    targets: &[SensorTarget],
    states: &[TargetState],
    bus_path: &str,
) -> Vec<PollOutcome> {
    let mut outcomes = Vec::new();

    for (i, target) in targets.iter().enumerate() {
        match &states[i] {
            TargetState::Pending => {
                match crate::sensors::probe(&target.kind, bus_path, target.address) {
                    Ok(identity) => {
                        let key = device_key_for(target);
                        outcomes.push(PollOutcome::Discovered {
                            target_index: i,
                            key,
                            identity,
                        });
                        // Do NOT read in the same cycle — sensors like OPT3001
                        // need conversion latency after init. First read happens
                        // on the next poll tick.
                    }
                    Err(msg) => {
                        outcomes.push(PollOutcome::ProbeFailed {
                            target_index: i,
                            message: msg,
                        });
                    }
                }
            }
            TargetState::Active(key) => {
                match crate::sensors::read(&target.kind, bus_path, target.address) {
                    Ok(reading) => {
                        outcomes.push(PollOutcome::Reading {
                            key: key.clone(),
                            reading,
                        });
                    }
                    Err(msg) => {
                        outcomes.push(PollOutcome::ReadError {
                            key: key.clone(),
                            message: msg,
                        });
                    }
                }
            }
        }
    }

    outcomes
}
```

- [ ] **Step 2: Build**

```bash
cargo build -p rpi-local-adapter 2>&1
```

Expected: compiles. poll_cycle is not unit-testable without I2C hardware (tested via integration tests later).

- [ ] **Step 3: Commit**

```bash
git add rpi-local-adapter/src/polling_loop.rs
git commit -m "feat(rpi-local-adapter): add poll_cycle blocking function

Synchronous poll cycle for spawn_blocking:
- Pending targets: probe → on success, discover + read
- Active targets: read → SensorData or ReadError
- Returns Vec<PollOutcome> for async side to process"
```

---

## Task 6: Async polling_loop

**Files:**
- Modify: `rpi-local-adapter/src/polling_loop.rs`

- [ ] **Step 1: Add the async polling_loop function**

Add to `rpi-local-adapter/src/polling_loop.rs`, below the `poll_cycle` function and above `#[cfg(test)]`:

```rust
/// The main async polling loop. Runs as a spawned tokio task.
pub(crate) async fn polling_loop(
    config: RpiLocalConfig,
    event_tx: mpsc::Sender<AdapterEvent>,
    mut command_rx: mpsc::Receiver<AdapterCommand>,
) {
    let period = std::time::Duration::from_millis(config.poll_interval_ms);
    let bus_path = config.bus_path.clone();
    let targets = config.targets.clone();

    // Initialize all targets as Pending.
    let mut states: Vec<TargetState> = vec![TargetState::Pending; targets.len()];

    // Startup probe: one spawn_blocking call for all targets.
    {
        let bus = bus_path.clone();
        let tgts = targets.clone();
        let st = states.clone();
        match tokio::task::spawn_blocking(move || poll_cycle(&tgts, &st, &bus)).await {
            Ok(outcomes) => {
                let events = apply_outcomes(outcomes, &mut states);
                for event in events {
                    if event_tx.send(event).await.is_err() {
                        tracing::warn!("Event channel closed during startup probe");
                        return;
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Startup probe spawn_blocking failed");
                let _ = event_tx.send(AdapterEvent::AdapterError {
                    device_key: None,
                    error: format!("startup probe failed: {}", e),
                }).await;
                return;
            }
        }
    }

    tracing::info!(
        active = states.iter().filter(|s| matches!(s, TargetState::Active(_))).count(),
        pending = states.iter().filter(|s| matches!(s, TargetState::Pending)).count(),
        "Startup probe complete, entering poll loop",
    );

    // Use interval_at to avoid immediate first tick after startup probe.
    let start = tokio::time::Instant::now() + period;
    let mut interval = tokio::time::interval_at(start, period);

    loop {
        tokio::select! {
            biased;
            cmd = command_rx.recv() => {
                match cmd {
                    Some(AdapterCommand::Shutdown) | None => {
                        tracing::info!("rpi-local-adapter shutting down");
                        return;
                    }
                    Some(AdapterCommand::DeviceCommand(dev_cmd)) => {
                        let _ = event_tx.send(AdapterEvent::AdapterError {
                            device_key: Some(dev_cmd.device_key),
                            error: "unsupported command: rpi-local-adapter v1 does not handle DeviceCommand".to_string(),
                        }).await;
                    }
                }
            }
            _ = interval.tick() => {
                let bus = bus_path.clone();
                let tgts = targets.clone();
                let st = states.clone();
                match tokio::task::spawn_blocking(move || poll_cycle(&tgts, &st, &bus)).await {
                    Ok(outcomes) => {
                        let events = apply_outcomes(outcomes, &mut states);
                        for event in events {
                            if event_tx.send(event).await.is_err() {
                                tracing::warn!("Event channel closed, exiting poll loop");
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Poll cycle spawn_blocking failed");
                        let _ = event_tx.send(AdapterEvent::AdapterError {
                            device_key: None,
                            error: format!("poll cycle failed: {}", e),
                        }).await;
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Add required use at top of file**

Ensure these are at the top of `polling_loop.rs`:

```rust
use std::time::Duration;
```

(The other imports — `AdapterCommand`, `AdapterEvent`, `DeviceKey`, `SensorIdentity`, `SensorReading`, `mpsc` — should already be present from Task 4.)

- [ ] **Step 3: Build**

```bash
cargo build -p rpi-local-adapter 2>&1
```

Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add rpi-local-adapter/src/polling_loop.rs
git commit -m "feat(rpi-local-adapter): add async polling_loop

Single-task tokio::select! loop:
- Startup probe via spawn_blocking
- interval_at to avoid immediate tick after startup
- Poll cycle via spawn_blocking per tick
- Shutdown on AdapterCommand::Shutdown or command_rx close
- DeviceCommand rejected with AdapterError"
```

---

## Task 7: AdapterHandle and start()

**Files:**
- Modify: `rpi-local-adapter/src/lib.rs`

- [ ] **Step 1: Write the start_without_runtime test**

Add to the bottom of `rpi-local-adapter/src/lib.rs`:

```rust
//! rpi-local-adapter: RPi ローカル直結 hardware の adapter。
//! v1 は I2C slice のみ。

pub mod config;
mod polling_loop;
mod sensors;

pub use config::{RpiLocalConfig, SensorKind, SensorTarget, ThermocoupleType};

use iotkit_core_types::{AdapterCommand, AdapterEvent, AdapterId};
use tokio::sync::mpsc;

/// Adapter handle. Core uses this to receive events and send commands.
pub struct AdapterHandle {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub command_tx: mpsc::Sender<AdapterCommand>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl AdapterHandle {
    /// Cooperative shutdown: close event_rx, send Shutdown, await task completion.
    /// Waits for any in-progress spawn_blocking (probe/poll cycle) to finish.
    pub async fn shutdown(mut self) -> Result<(), String> {
        self.event_rx.close();
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;
        if let Some(handle) = self.task_handle.take() {
            handle
                .await
                .map_err(|e| format!("polling_loop panicked: {}", e))?;
        }
        Ok(())
    }
}

/// Start the rpi-local-adapter.
///
/// Validates config first, then checks for tokio runtime, spawns the polling
/// loop task, and returns an AdapterHandle.
///
/// I2C bus open/probe/read failures are reported as AdapterEvent::AdapterError,
/// not as start() errors.
pub fn start(config: RpiLocalConfig) -> Result<AdapterHandle, std::io::Error> {
    // Validate config before runtime check so config errors are always
    // reported as config errors, regardless of runtime presence.
    config::validate_config(&config).map_err(std::io::Error::other)?;

    let runtime_handle =
        tokio::runtime::Handle::try_current().map_err(std::io::Error::other)?;

    let id = AdapterId::new("rpi-local:default");
    let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(256);
    let (command_tx, command_rx) = mpsc::channel::<AdapterCommand>(32);

    let task_handle =
        runtime_handle.spawn(polling_loop::polling_loop(config, event_tx, command_rx));

    Ok(AdapterHandle {
        id,
        event_rx,
        command_tx,
        task_handle: Some(task_handle),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tokio runtime が無い状態で start() を呼ぶと panic せず Err を返す。
    /// #[tokio::test] ではなく plain #[test] で実行することで runtime 不在を保証する。
    #[test]
    fn start_without_runtime_returns_error() {
        let config = RpiLocalConfig {
            bus_path: "/dev/i2c-1".to_string(),
            poll_interval_ms: 1000,
            targets: vec![SensorTarget {
                address: 0x60,
                kind: SensorKind::MCP9600 {
                    thermocouple_type: ThermocoupleType::K,
                },
            }],
        };
        let result = start(config);
        assert!(result.is_err(), "start() should return Err without tokio runtime");
    }

    /// Config validation runs before runtime check, so this test verifies
    /// that invalid config produces a config-specific error message even
    /// without a tokio runtime.
    #[test]
    fn start_with_invalid_config_returns_config_error() {
        let config = RpiLocalConfig {
            bus_path: "/dev/i2c-1".to_string(),
            poll_interval_ms: 0,
            targets: vec![],
        };
        let err = start(config).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("poll_interval_ms"),
            "expected config validation error, got: {}",
            msg,
        );
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p rpi-local-adapter 2>&1
```

Expected: 14 tests pass (6 config + 6 polling_loop + 2 lib).

- [ ] **Step 3: Commit**

```bash
git add rpi-local-adapter/src/lib.rs
git commit -m "feat(rpi-local-adapter): add AdapterHandle and start()

- AdapterHandle with id, event_rx, command_tx, shutdown()
- start() validates config, spawns polling_loop, returns handle
- AdapterId: rpi-local:default
- 2 unit tests: start without runtime, start with invalid config"
```

---

## Task 8: Integration test (RPi-only)

**Files:**
- Create: `rpi-local-adapter/tests/integration.rs`

- [ ] **Step 1: Write integration test**

Create `rpi-local-adapter/tests/integration.rs`:

```rust
//! Integration tests — require real I2C hardware.
//! Run with: cargo test -p rpi-local-adapter --test integration -- --ignored

use std::time::Duration;

use iotkit_core_types::AdapterEvent;
use rpi_local_adapter::{RpiLocalConfig, SensorKind, SensorTarget, ThermocoupleType};

#[tokio::test]
#[ignore]
async fn real_i2c_discovers_and_reads_mcp9600() {
    let config = RpiLocalConfig {
        bus_path: "/dev/i2c-1".to_string(),
        poll_interval_ms: 1000,
        targets: vec![SensorTarget {
            address: 0x60,
            kind: SensorKind::MCP9600 {
                thermocouple_type: ThermocoupleType::K,
            },
        }],
    };

    let mut handle = rpi_local_adapter::start(config).expect("start() should succeed");

    // First event should be DeviceDiscovered
    let event = tokio::time::timeout(Duration::from_secs(5), handle.event_rx.recv())
        .await
        .expect("timeout waiting for DeviceDiscovered")
        .expect("channel should not be closed");
    assert!(
        matches!(event, AdapterEvent::DeviceDiscovered { .. }),
        "expected DeviceDiscovered, got {:?}",
        event,
    );

    // SensorData arrives on next poll tick (not same cycle as probe,
    // because sensors may need conversion latency after init).
    let event = tokio::time::timeout(Duration::from_secs(5), handle.event_rx.recv())
        .await
        .expect("timeout waiting for SensorData")
        .expect("channel should not be closed");
    assert!(
        matches!(event, AdapterEvent::SensorData { .. }),
        "expected SensorData, got {:?}",
        event,
    );

    // Shutdown cleanly
    handle.shutdown().await.expect("shutdown should succeed");
}

#[tokio::test]
#[ignore]
async fn real_i2c_discovers_and_reads_opt3001() {
    let config = RpiLocalConfig {
        bus_path: "/dev/i2c-1".to_string(),
        poll_interval_ms: 1000,
        targets: vec![SensorTarget {
            address: 0x44,
            kind: SensorKind::OPT3001,
        }],
    };

    let mut handle = rpi_local_adapter::start(config).expect("start() should succeed");

    // DeviceDiscovered from startup probe.
    let event = tokio::time::timeout(Duration::from_secs(5), handle.event_rx.recv())
        .await
        .expect("timeout waiting for DeviceDiscovered")
        .expect("channel should not be closed");
    assert!(
        matches!(event, AdapterEvent::DeviceDiscovered { .. }),
        "expected DeviceDiscovered, got {:?}",
        event,
    );

    // SensorData arrives on next poll tick (conversion latency).
    let event = tokio::time::timeout(Duration::from_secs(5), handle.event_rx.recv())
        .await
        .expect("timeout waiting for SensorData")
        .expect("channel should not be closed");
    assert!(
        matches!(event, AdapterEvent::SensorData { .. }),
        "expected SensorData, got {:?}",
        event,
    );

    handle.shutdown().await.expect("shutdown should succeed");
}
```

- [ ] **Step 2: Build the integration test (don't run)**

```bash
cargo test -p rpi-local-adapter --test integration --no-run 2>&1
```

Expected: compiles. Tests are `#[ignore]` so `cargo test` skips them by default.

- [ ] **Step 3: Commit**

```bash
git add rpi-local-adapter/tests/
git commit -m "test(rpi-local-adapter): add integration tests for real I2C

#[ignore] tests for RPi-only execution:
- real_i2c_discovers_and_reads_mcp9600
- real_i2c_discovers_and_reads_opt3001
Run with: cargo test -p rpi-local-adapter --test integration -- --ignored"
```

---

## Task 9: Gateway integration — fan-in with both adapters

**Files:**
- Modify: `iotkit-gateway/Cargo.toml`
- Modify: `iotkit-gateway/src/main.rs`

- [ ] **Step 1: Add rpi-local-adapter dependency**

Update `iotkit-gateway/Cargo.toml`:

```toml
[package]
name = "iotkit-gateway"
version = "0.1.0"
edition = "2024"

[dependencies]
iotkit-core-types = { path = "../core/types" }
iotkit-core-engine = { path = "../core/engine" }
bravepi-mainboard-adapter = { path = "../bravepi-mainboard-adapter" }
rpi-local-adapter = { path = "../rpi-local-adapter" }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

- [ ] **Step 2: Rewrite main.rs with fan-in loop**

Replace `iotkit-gateway/src/main.rs` with:

```rust
//! iotkit-gateway: composition root。
//! adapter を起動し、core/engine に event を渡す。

use iotkit_core_engine::{Engine, EngineEvent};
use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let port_path =
        std::env::var("BRAVEPI_PORT").unwrap_or_else(|_| "/dev/ttyAMA0".to_string());

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(run(port_path));
}

/// Hardcoded rpi-local-adapter config for v1.
/// Env vars for bus path and poll interval only — no target parsing DSL.
/// Config-driven target list is deferred to sub-project C (orchestrator).
fn rpi_local_config() -> rpi_local_adapter::RpiLocalConfig {
    use rpi_local_adapter::{SensorKind, SensorTarget, ThermocoupleType};

    let bus_path = std::env::var("RPI_LOCAL_I2C_BUS")
        .unwrap_or_else(|_| "/dev/i2c-1".to_string());

    let poll_interval_ms: u64 = std::env::var("RPI_LOCAL_POLL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);

    rpi_local_adapter::RpiLocalConfig {
        bus_path,
        poll_interval_ms,
        targets: vec![
            SensorTarget {
                address: 0x60,
                kind: SensorKind::MCP9600 {
                    thermocouple_type: ThermocoupleType::K,
                },
            },
            SensorTarget {
                address: 0x44,
                kind: SensorKind::OPT3001,
            },
        ],
    }
}

async fn run(port_path: String) {
    let engine = Engine::new();

    // BravePI mainboard adapter is required: start failure is fatal.
    let mut bravepi = match bravepi_mainboard_adapter::task::start(port_path) {
        Ok(h) => {
            tracing::info!(adapter_id = %h.id, "BravePI mainboard adapter started");
            h
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to start BravePI mainboard adapter");
            std::process::exit(1);
        }
    };
    let bravepi_id = bravepi.id.clone();

    // RPi local adapter is optional: start failure is a warning.
    let mut rpi_local = match rpi_local_adapter::start(rpi_local_config()) {
        Ok(h) => {
            tracing::info!(adapter_id = %h.id, "RPi local adapter started");
            Some(h)
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to start RPi local adapter, continuing without it");
            None
        }
    };

    // Track whether each adapter's channel is still open.
    // Handles are kept even after channel close for shutdown cleanup.
    let mut bravepi_open = true;
    let mut rpi_local_open = rpi_local.is_some();

    loop {
        tokio::select! {
            biased;

            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutdown signal received");
                break;
            }

            event = bravepi.event_rx.recv(), if bravepi_open => {
                match event {
                    Some(ev) => {
                        tracing::debug!(adapter = %bravepi_id, event = ?ev, "BravePI event");
                        engine.apply(EngineEvent {
                            adapter_id: bravepi_id.clone(),
                            event: ev,
                        }).await;
                    }
                    None => {
                        tracing::info!("BravePI adapter channel closed");
                        bravepi_open = false;
                        if !rpi_local_open {
                            tracing::info!("All adapter channels closed, exiting");
                            break;
                        }
                    }
                }
            }

            event = async {
                match rpi_local.as_mut() {
                    Some(h) => h.event_rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if rpi_local_open => {
                match event {
                    Some(ev) => {
                        let adapter_id = rpi_local.as_ref().unwrap().id.clone();
                        tracing::debug!(adapter = %adapter_id, event = ?ev, "RPi local event");
                        engine.apply(EngineEvent {
                            adapter_id,
                            event: ev,
                        }).await;
                    }
                    None => {
                        tracing::info!("RPi local adapter channel closed");
                        rpi_local_open = false;
                        if !bravepi_open {
                            tracing::info!("All adapter channels closed, exiting");
                            break;
                        }
                    }
                }
            }
        }
    }

    // Shutdown all adapters (handles kept even after channel close).
    if let Err(e) = bravepi.shutdown().await {
        tracing::error!(error = %e, "BravePI adapter shutdown error");
    }
    if let Some(h) = rpi_local {
        if let Err(e) = h.shutdown().await {
            tracing::error!(error = %e, "RPi local adapter shutdown error");
        }
    }

    let devices = engine.devices().await;
    tracing::info!(device_count = devices.len(), "Engine state at shutdown");
}
```

- [ ] **Step 3: Build**

```bash
cargo build --workspace 2>&1
```

Expected: compiles.

- [ ] **Step 4: Run all tests**

```bash
cargo test --workspace 2>&1
```

Expected: all tests pass. Integration tests skipped (marked `#[ignore]`).

- [ ] **Step 5: Commit**

```bash
git add iotkit-gateway/
git commit -m "feat(gateway): integrate rpi-local-adapter with fan-in loop

- bravepi-mainboard is required (fatal on failure), rpi-local is optional
- Fan-in loop: continue when one adapter closes, exit when all close
- Fuse closed channels via bool flag, keep handles for shutdown cleanup
- Hardcoded target list for v1, env vars for bus_path and poll_interval only
- Graceful shutdown for all adapters on ctrl-c"
```

---

## Self-Review Checklist

### Spec coverage

| Spec section | Task |
|---|---|
| Scope & Naming / rename | Task 1 |
| Scope & Naming / rpi-local-adapter boundary | Tasks 2–8 |
| 依存方向 | Task 2 (Cargo.toml deps), Task 9 (gateway) |
| v1 スコープ | All tasks collectively |
| Config Model | Task 2 |
| AdapterId / DeviceKey | Task 4 (`device_key_for`), Task 7 (`start`) |
| Discovery Flow | Task 5 (`poll_cycle`), Task 6 (`polling_loop`) |
| Polling Loop | Task 6 |
| I2C Blocking I/O | Task 5, Task 6 |
| TargetState | Task 4 |
| PollOutcome | Task 4 |
| Event Generation Rules | Task 4 (`apply_outcomes`), Task 6 (polling_loop send) |
| Sensor Integration | Task 3 |
| MCP9600 | Task 3 (`sensors/mcp9600.rs`) |
| OPT3001 + byte order | Task 3 (`sensors/opt3001.rs`) |
| AdapterHandle Contract | Task 7 |
| Shutdown | Task 7 |
| Gateway Integration / fan-in | Task 9 |
| Testing Strategy: unit | Tasks 2, 4, 7 |
| Testing Strategy: integration | Task 8 |
| DeviceCommand rejection | Task 6 (polling_loop) |
| validate_config duplicate check | Task 2 |

### Placeholder scan

No TBD, TODO, "implement later", "similar to Task N", or missing code blocks found.

### Type consistency

- `RpiLocalConfig` / `SensorTarget` / `SensorKind`: defined in Task 2, used consistently in Tasks 3–9
- `TargetState` / `PollOutcome` / `apply_outcomes`: defined in Task 4, used in Tasks 5–6
- `device_key_for`: defined in Task 4, used in Task 5
- `poll_cycle`: defined in Task 5, used in Task 6
- `polling_loop`: defined in Task 6, used in Task 7
- `AdapterHandle` / `start`: defined in Task 7, used in Tasks 8–9
- `sensor_ic_name`: defined in Task 2, used in Tasks 4 (indirectly via `device_key_for`)
- `probe` / `read`: defined in Task 3, used in Task 5
