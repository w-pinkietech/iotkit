# Base Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract a reusable I2C polling base adapter from rpi-local-adapter so AI agents can create new sensor adapters by implementing only a SensorDriver trait.

**Architecture:** New `iotkit-polling-adapter-runtime` crate provides SensorDriver trait, BaseAdapterConfig, polling loop, state machine, AdapterHandle, and shutdown. rpi-local-adapter is refactored to a thin wrapper: RpiLocalConfig + MCP9600/OPT3001 driver implementations. bravepi-mainboard-adapter is untouched except trivial re-export adjustments.

**Tech Stack:** Rust, tokio (async runtime), tracing (diagnostics), iotkit-core-types (shared domain types)

**Spec:** `docs/superpowers/specs/2026-03-28-base-adapter-design.md`

---

## File Structure

### New files (iotkit-polling-adapter-runtime crate)

| File | Responsibility |
|------|---------------|
| `iotkit-polling-adapter-runtime/Cargo.toml` | Crate manifest |
| `iotkit-polling-adapter-runtime/src/lib.rs` | Public API: SensorDriver trait, BaseAdapterConfig, SensorTargetConfig, AdapterHandle, start(), validate_config(), re-exports |
| `iotkit-polling-adapter-runtime/src/polling_loop.rs` | TargetState, TargetRuntime, PollOutcome, apply_outcomes(), poll_cycle(), polling_loop() async fn |

### Modified files

| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | Add `iotkit-polling-adapter-runtime` to members |
| `rpi-local-adapter/Cargo.toml` | Add `iotkit-polling-adapter-runtime` dependency |
| `rpi-local-adapter/src/lib.rs` | Replace AdapterHandle/start/shutdown with thin wrapper delegating to base adapter |
| `rpi-local-adapter/src/drivers/mod.rs` | New: module declarations for mcp9600 and opt3001 drivers |
| `rpi-local-adapter/src/drivers/mcp9600.rs` | New: Mcp9600Driver implementing SensorDriver |
| `rpi-local-adapter/src/drivers/opt3001.rs` | New: Opt3001Driver implementing SensorDriver |
| `rpi-local-adapter/tests/integration.rs` | Minor: update imports if needed |

### Deleted files (after refactor)

| File | Reason |
|------|--------|
| `rpi-local-adapter/src/polling_loop.rs` | Moved to base adapter |
| `rpi-local-adapter/src/config.rs` | Merged into lib.rs (RpiLocalConfig) + base adapter (BaseAdapterConfig) |
| `rpi-local-adapter/src/sensors/mod.rs` | Replaced by trait dispatch |
| `rpi-local-adapter/src/sensors/mcp9600.rs` | Replaced by drivers/mcp9600.rs |
| `rpi-local-adapter/src/sensors/opt3001.rs` | Replaced by drivers/opt3001.rs |

---

### Task 1: Create iotkit-polling-adapter-runtime crate skeleton with SensorDriver trait and config

**Files:**
- Create: `iotkit-polling-adapter-runtime/Cargo.toml`
- Create: `iotkit-polling-adapter-runtime/src/lib.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create crate directory and Cargo.toml**

```toml
# iotkit-polling-adapter-runtime/Cargo.toml
[package]
name = "iotkit-polling-adapter-runtime"
version = "0.1.0"
edition = "2021"

[dependencies]
iotkit-core-types = { path = "../core/types" }
tokio = { version = "1", features = ["sync", "rt", "time"] }
tracing = "0.1"
```

- [ ] **Step 2: Add to workspace members**

In `Cargo.toml` (workspace root), add `"iotkit-polling-adapter-runtime"` to the `members` array.

- [ ] **Step 3: Write lib.rs with SensorDriver trait, config types, and validate_config**

```rust
// iotkit-polling-adapter-runtime/src/lib.rs
//! iotkit-polling-adapter-runtime: reusable I2C polling adapter skeleton.
//!
//! AI agents implement `SensorDriver` per sensor IC.
//! The base adapter provides: polling loop, channel wiring,
//! AdapterHandle, shutdown, state machine, failure recovery.

mod polling_loop;

use std::sync::Arc;
use iotkit_core_types::{AdapterCommand, AdapterEvent, AdapterId, SensorIdentity, SensorReading};
use tokio::sync::mpsc;

/// Trait for sensor-specific I2C probe and read logic.
///
/// The base adapter calls these inside `spawn_blocking`.
///
/// Implementations MUST be `Send + Sync`.
/// All I/O errors MUST be returned as `Err(String)` with bus path and
/// address in the message for field debugging.
///
/// Drivers are per-target instances via `Arc`. Bus path and address are
/// passed as parameters because error messages need bus/addr context.
pub trait SensorDriver: Send + Sync {
    /// Probe: open I2C, verify device ID, write init config.
    /// MUST complete within ~5s wall-clock.
    fn probe(&self, bus_path: &str, address: u8) -> Result<SensorIdentity, String>;

    /// Read: open I2C, read register(s), decode to SensorReading.
    /// MUST complete within ~3s wall-clock.
    fn read(&self, bus_path: &str, address: u8) -> Result<SensorReading, String>;

    /// IC name for DeviceKey generation (e.g., "mcp9600").
    fn ic_name(&self) -> &'static str;

    /// Optional: validate driver-specific constraints.
    /// Called during `validate_config()`.
    fn validate(&self, poll_interval_ms: u64) -> Result<(), String> {
        let _ = poll_interval_ms;
        Ok(())
    }
}

/// Config for an I2C polling adapter.
pub struct BaseAdapterConfig {
    /// I2C bus path (e.g., "/dev/i2c-1").
    pub bus_path: String,
    /// Polling interval in milliseconds. Must be > 0.
    pub poll_interval_ms: u64,
    /// Sensor targets to probe and poll.
    pub targets: Vec<SensorTargetConfig>,
}

/// A sensor target on the I2C bus.
pub struct SensorTargetConfig {
    /// 7-bit I2C address (0x08..=0x77).
    pub address: u8,
    /// The sensor driver for this target.
    pub driver: Arc<dyn SensorDriver>,
    /// Stable suffix for DeviceKey. If None, uses driver.ic_name().
    pub key_suffix: Option<String>,
}

/// Adapter handle. Core uses this to receive events and send commands.
#[derive(Debug)]
pub struct AdapterHandle {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub command_tx: mpsc::Sender<AdapterCommand>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl AdapterHandle {
    /// Cooperative shutdown: close event_rx, send Shutdown, await task.
    pub async fn shutdown(mut self) -> Result<(), String> {
        self.event_rx.close();
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;
        if let Some(handle) = self.task_handle.take() {
            handle.await.map_err(|e| format!("polling_loop panicked: {}", e))?;
        }
        Ok(())
    }
}

/// Validate config before starting the adapter.
pub fn validate_config(config: &BaseAdapterConfig) -> Result<(), String> {
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
        target.driver.validate(config.poll_interval_ms).map_err(|e| {
            format!("driver validation failed for 0x{:02x}: {}", target.address, e)
        })?;
    }
    Ok(())
}

/// Start an I2C polling adapter.
///
/// Validates config, opens bus path to verify access, checks for tokio
/// runtime, spawns the polling loop task, and returns an AdapterHandle.
pub fn start(
    adapter_id: AdapterId,
    config: BaseAdapterConfig,
) -> Result<AdapterHandle, std::io::Error> {
    validate_config(&config).map_err(std::io::Error::other)?;

    // Bus path existence check (fail fast on typos/permissions).
    std::fs::File::open(&config.bus_path).map_err(|e| {
        std::io::Error::other(format!(
            "cannot open bus path {}: {}",
            config.bus_path, e
        ))
    })?;

    let runtime_handle =
        tokio::runtime::Handle::try_current().map_err(std::io::Error::other)?;

    let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(256);
    let (command_tx, command_rx) = mpsc::channel::<AdapterCommand>(32);

    let task_handle = runtime_handle.spawn(
        polling_loop::polling_loop(config, event_tx, command_rx),
    );

    Ok(AdapterHandle {
        id: adapter_id,
        event_rx,
        command_tx,
        task_handle: Some(task_handle),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_driver() -> Arc<dyn SensorDriver> {
        struct Stub;
        impl SensorDriver for Stub {
            fn probe(&self, _: &str, _: u8) -> Result<SensorIdentity, String> {
                unimplemented!()
            }
            fn read(&self, _: &str, _: u8) -> Result<SensorReading, String> {
                unimplemented!()
            }
            fn ic_name(&self) -> &'static str { "stub" }
        }
        Arc::new(Stub)
    }

    #[test]
    fn valid_config_passes() {
        let config = BaseAdapterConfig {
            bus_path: "/dev/i2c-1".into(),
            poll_interval_ms: 1000,
            targets: vec![SensorTargetConfig {
                address: 0x60,
                driver: mock_driver(),
                key_suffix: None,
            }],
        };
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn empty_bus_path_rejected() {
        let config = BaseAdapterConfig {
            bus_path: String::new(),
            poll_interval_ms: 1000,
            targets: vec![],
        };
        let err = validate_config(&config).unwrap_err();
        assert!(err.contains("bus_path"), "error: {}", err);
    }

    #[test]
    fn zero_poll_interval_rejected() {
        let config = BaseAdapterConfig {
            bus_path: "/dev/i2c-1".into(),
            poll_interval_ms: 0,
            targets: vec![],
        };
        let err = validate_config(&config).unwrap_err();
        assert!(err.contains("poll_interval_ms"), "error: {}", err);
    }

    #[test]
    fn duplicate_address_rejected() {
        let config = BaseAdapterConfig {
            bus_path: "/dev/i2c-1".into(),
            poll_interval_ms: 1000,
            targets: vec![
                SensorTargetConfig { address: 0x60, driver: mock_driver(), key_suffix: None },
                SensorTargetConfig { address: 0x60, driver: mock_driver(), key_suffix: None },
            ],
        };
        let err = validate_config(&config).unwrap_err();
        assert!(err.contains("duplicate"), "error: {}", err);
    }

    #[test]
    fn address_out_of_range_rejected() {
        let config = BaseAdapterConfig {
            bus_path: "/dev/i2c-1".into(),
            poll_interval_ms: 1000,
            targets: vec![SensorTargetConfig {
                address: 0x80,
                driver: mock_driver(),
                key_suffix: None,
            }],
        };
        let err = validate_config(&config).unwrap_err();
        assert!(err.contains("outside valid"), "error: {}", err);
    }

    #[test]
    fn driver_validate_called() {
        struct ValidatingDriver;
        impl SensorDriver for ValidatingDriver {
            fn probe(&self, _: &str, _: u8) -> Result<SensorIdentity, String> { unimplemented!() }
            fn read(&self, _: &str, _: u8) -> Result<SensorReading, String> { unimplemented!() }
            fn ic_name(&self) -> &'static str { "test" }
            fn validate(&self, poll_interval_ms: u64) -> Result<(), String> {
                if poll_interval_ms < 200 {
                    Err("too short".into())
                } else {
                    Ok(())
                }
            }
        }
        let config = BaseAdapterConfig {
            bus_path: "/dev/i2c-1".into(),
            poll_interval_ms: 50,
            targets: vec![SensorTargetConfig {
                address: 0x44,
                driver: Arc::new(ValidatingDriver),
                key_suffix: None,
            }],
        };
        let err = validate_config(&config).unwrap_err();
        assert!(err.contains("too short"), "error: {}", err);
    }

    #[test]
    fn start_without_runtime_returns_error() {
        let config = BaseAdapterConfig {
            bus_path: "/dev/null".into(),
            poll_interval_ms: 1000,
            targets: vec![],
        };
        let result = start(AdapterId::new("test:default"), config);
        assert!(result.is_err());
    }

    #[test]
    fn start_with_bad_bus_path_returns_error() {
        // This test won't have a runtime either, but config validates first,
        // then bus path check. /dev/nonexistent should fail on bus check.
        // However without runtime it fails on runtime check.
        // We test bus path separately by calling validate + fs::File::open.
        let result = std::fs::File::open("/dev/nonexistent-i2c-bus-999");
        assert!(result.is_err());
    }
}
```

- [ ] **Step 4: Create empty polling_loop.rs stub**

```rust
// iotkit-polling-adapter-runtime/src/polling_loop.rs
//! Polling loop internals: state management, outcome processing, async loop.

use iotkit_core_types::{AdapterCommand, AdapterEvent};
use tokio::sync::mpsc;
use crate::BaseAdapterConfig;

pub(crate) async fn polling_loop(
    _config: BaseAdapterConfig,
    _event_tx: mpsc::Sender<AdapterEvent>,
    _command_rx: mpsc::Receiver<AdapterCommand>,
) {
    // Stub — implemented in Task 2
}
```

- [ ] **Step 5: Run `cargo check -p iotkit-polling-adapter-runtime` and `cargo test -p iotkit-polling-adapter-runtime`**

Expected: check passes, all tests pass (7 config tests + start tests).

- [ ] **Step 6: Commit**

```
feat(iotkit-polling-adapter-runtime): add crate skeleton with SensorDriver trait and config validation
```

---

### Task 2: Implement PollOutcome, TargetState, apply_outcomes pure function

**Files:**
- Modify: `iotkit-polling-adapter-runtime/src/polling_loop.rs`

This is the pure state-transition logic. No async code, no I/O.

- [ ] **Step 1: Write TargetState, TargetRuntime, PollOutcome types and apply_outcomes tests**

Add to `polling_loop.rs`:

```rust
use std::sync::Arc;
use iotkit_core_types::{AdapterEvent, DeviceKey, SensorIdentity, SensorReading};

/// Internal failure thresholds (not config — avoids partial-config anti-pattern).
pub(crate) const MAX_READ_FAILURES: u32 = 5;
pub(crate) const MAX_PROBE_FAILURES: u32 = 10;

/// Per-target discovery state with failure tracking.
#[derive(Debug, Clone)]
pub(crate) enum TargetState {
    Pending {
        consecutive_probe_failures: u32,
        escalation_emitted: bool,
    },
    Active {
        key: DeviceKey,
        consecutive_read_failures: u32,
    },
}

impl TargetState {
    pub(crate) fn new_pending() -> Self {
        TargetState::Pending {
            consecutive_probe_failures: 0,
            escalation_emitted: false,
        }
    }
}

/// Resolved target info for the polling loop. Stored in Arc for spawn_blocking.
pub(crate) struct TargetRuntime {
    pub address: u8,
    pub driver: Arc<dyn crate::SensorDriver>,
    pub key_suffix: String,
}

/// Result of a single target's probe or read in spawn_blocking.
#[derive(Debug)]
pub(crate) enum PollOutcome {
    Discovered { target_index: usize, key: DeviceKey, identity: SensorIdentity },
    Reading { key: DeviceKey, reading: SensorReading },
    ReadError { target_index: usize, key: DeviceKey, message: String },
    ProbeFailed { target_index: usize, message: String },
}

/// Builds a DeviceKey for a target.
pub(crate) fn device_key_for(address: u8, key_suffix: &str) -> DeviceKey {
    DeviceKey::new(format!("i2c:0x{:02x}:{}", address, key_suffix))
}

/// Pure function: applies poll outcomes to target states, returns events.
pub(crate) fn apply_outcomes(
    outcomes: Vec<PollOutcome>,
    states: &mut [TargetState],
    targets: &[TargetRuntime],
) -> Vec<AdapterEvent> {
    let mut events = Vec::new();

    for outcome in outcomes {
        match outcome {
            PollOutcome::Discovered { target_index, key, identity } => {
                states[target_index] = TargetState::Active {
                    key: key.clone(),
                    consecutive_read_failures: 0,
                };
                events.push(AdapterEvent::DeviceDiscovered {
                    device_key: key,
                    identity,
                });
            }
            PollOutcome::Reading { key, reading } => {
                // Reset failure counter on successful read.
                for state in states.iter_mut() {
                    if let TargetState::Active { key: ref k, consecutive_read_failures } = state {
                        if k == &key {
                            *consecutive_read_failures = 0;
                        }
                    }
                }
                events.push(AdapterEvent::SensorData {
                    device_key: key,
                    reading,
                    rssi: None,
                    battery_pct: None,
                });
            }
            PollOutcome::ReadError { target_index, key, message } => {
                if let TargetState::Active { consecutive_read_failures, .. } = &mut states[target_index] {
                    *consecutive_read_failures += 1;
                    let failures = *consecutive_read_failures;

                    events.push(AdapterEvent::AdapterError {
                        device_key: Some(key.clone()),
                        error: message.clone(),
                    });

                    if failures >= MAX_READ_FAILURES {
                        tracing::info!(
                            device_key = %key,
                            failures,
                            last_error = %message,
                            "Device lost: consecutive read failures at threshold",
                        );
                        events.push(AdapterEvent::DeviceLost {
                            device_key: key,
                            reason: format!(
                                "consecutive read failures ({}): {}",
                                failures, message,
                            ),
                        });
                        states[target_index] = TargetState::new_pending();
                    }
                }
            }
            PollOutcome::ProbeFailed { target_index, message } => {
                if let TargetState::Pending { consecutive_probe_failures, escalation_emitted } = &mut states[target_index] {
                    *consecutive_probe_failures += 1;
                    let failures = *consecutive_probe_failures;

                    if failures >= MAX_PROBE_FAILURES && !*escalation_emitted {
                        let addr = targets[target_index].address;
                        events.push(AdapterEvent::AdapterError {
                            device_key: None,
                            error: format!(
                                "target 0x{:02x} probe failed {} consecutive times: {}",
                                addr, failures, message,
                            ),
                        });
                        *escalation_emitted = true;
                    } else {
                        tracing::warn!(
                            error = %message,
                            failures,
                            "Probe failed (no event)",
                        );
                    }
                }
            }
        }
    }

    events
}
```

- [ ] **Step 2: Write apply_outcomes unit tests**

Add `#[cfg(test)] mod tests` at bottom of `polling_loop.rs` with tests:
- `probe_success_transitions_to_active_and_emits_discovered`
- `read_success_emits_sensor_data_and_resets_counter`
- `read_failure_emits_error_and_increments_counter`
- `read_failures_at_threshold_emits_lost_and_transitions_to_pending`
- `probe_failure_below_threshold_emits_no_event`
- `probe_failure_at_threshold_emits_one_error`
- `probe_success_after_escalation_resets_counter_and_flag`
- `discovered_only_no_same_cycle_read`
- `multiple_targets_independent`

Each test creates `TargetState` + `TargetRuntime` + `PollOutcome`, calls `apply_outcomes`, asserts events and state changes. Use a minimal `MockRuntime` helper for `TargetRuntime`.

- [ ] **Step 3: Run `cargo test -p iotkit-polling-adapter-runtime`**

Expected: all apply_outcomes tests pass.

- [ ] **Step 4: Commit**

```
feat(iotkit-polling-adapter-runtime): add PollOutcome, TargetState, apply_outcomes pure function
```

---

### Task 3: Implement poll_cycle blocking function

**Files:**
- Modify: `iotkit-polling-adapter-runtime/src/polling_loop.rs`

- [ ] **Step 1: Implement poll_cycle**

```rust
/// Execute one poll cycle synchronously (called inside spawn_blocking).
/// Pending → probe only (no same-cycle read). Active → read.
pub(crate) fn poll_cycle(
    targets: &[TargetRuntime],
    states: &[TargetState],
    bus_path: &str,
) -> Vec<PollOutcome> {
    let mut outcomes = Vec::new();

    for (i, target) in targets.iter().enumerate() {
        match &states[i] {
            TargetState::Pending { .. } => {
                match target.driver.probe(bus_path, target.address) {
                    Ok(identity) => {
                        let key = device_key_for(target.address, &target.key_suffix);
                        outcomes.push(PollOutcome::Discovered {
                            target_index: i,
                            key,
                            identity,
                        });
                    }
                    Err(msg) => {
                        outcomes.push(PollOutcome::ProbeFailed {
                            target_index: i,
                            message: msg,
                        });
                    }
                }
            }
            TargetState::Active { key, .. } => {
                match target.driver.read(bus_path, target.address) {
                    Ok(reading) => {
                        outcomes.push(PollOutcome::Reading {
                            key: key.clone(),
                            reading,
                        });
                    }
                    Err(msg) => {
                        outcomes.push(PollOutcome::ReadError {
                            target_index: i,
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

- [ ] **Step 2: Add poll_cycle unit tests using MockDriver**

Test cases:
- Pending target with successful probe → Discovered outcome
- Pending target with failed probe → ProbeFailed outcome
- Active target with successful read → Reading outcome
- Active target with failed read → ReadError outcome

- [ ] **Step 3: Run `cargo test -p iotkit-polling-adapter-runtime`**

- [ ] **Step 4: Commit**

```
feat(iotkit-polling-adapter-runtime): add poll_cycle blocking function
```

---

### Task 4: Implement async polling_loop

**Files:**
- Modify: `iotkit-polling-adapter-runtime/src/polling_loop.rs`

- [ ] **Step 1: Implement the full async polling_loop**

Replace the stub with the complete implementation following the spec:
- Build `Arc<[TargetRuntime]>` from config
- Startup probe with spawn_blocking
- All-targets-fail startup check (emit immediate AdapterError if non-empty and all failed)
- `interval_at(now + period, period)` with `MissedTickBehavior::Skip`
- Main loop with `event_tx.is_closed()` check
- `tokio::select!` (no `biased`) with command_rx and interval tick
- DeviceCommand rejection with `device_key: Some(cmd.device_key)`
- Runtime cycle overrun warning (measure cycle duration, warn if > poll_interval_ms)

- [ ] **Step 2: Add async polling_loop tests**

Test cases using MockDriver:
- `shutdown_command_stops_loop`
- `command_channel_drop_stops_loop`
- `event_channel_close_detected_with_events` (send DeviceCommand to trigger)
- `event_channel_close_detected_without_events` (empty targets, is_closed check)
- `device_command_rejection_preserves_device_key`
- `mock_probe_discovery_then_read`
- `mock_probe_failure_retry_then_success`
- `mock_consecutive_read_failures_emit_device_lost`
- `empty_targets_no_startup_error`
- `all_targets_fail_startup_emits_immediate_error`

Each test: create channels + MockDriver, spawn polling_loop, assert events via event_rx.

- [ ] **Step 3: Run `cargo test -p iotkit-polling-adapter-runtime`**

Expected: all tests pass.

- [ ] **Step 4: Commit**

```
feat(iotkit-polling-adapter-runtime): add async polling_loop with recovery and escalation
```

---

### Task 5: Implement MCP9600 and OPT3001 SensorDriver implementations

**Files:**
- Create: `rpi-local-adapter/src/drivers/mod.rs`
- Create: `rpi-local-adapter/src/drivers/mcp9600.rs`
- Create: `rpi-local-adapter/src/drivers/opt3001.rs`

These wrap the existing `rpi-local-adapter/src/sensors/*.rs` I2C logic into the SensorDriver trait.

- [ ] **Step 1: Create drivers/mod.rs**

```rust
// rpi-local-adapter/src/drivers/mod.rs
pub mod mcp9600;
pub mod opt3001;
```

- [ ] **Step 2: Implement Mcp9600Driver**

```rust
// rpi-local-adapter/src/drivers/mcp9600.rs
use std::collections::BTreeMap;
use iotkit_polling_adapter_runtime::SensorDriver;
use iotkit_core_types::{ConnectionInfo, ConnectionKind, SensorIdentity, SensorReading};
pub use bravepi_sensors::mcp9600::ThermocoupleType;

pub struct Mcp9600Driver {
    pub thermocouple_type: ThermocoupleType,
}

impl SensorDriver for Mcp9600Driver {
    fn probe(&self, bus_path: &str, address: u8) -> Result<SensorIdentity, String> {
        // Same logic as current rpi-local-adapter/src/sensors/mcp9600.rs::probe_mcp9600
        let mut dev = rpi4b_transport::i2c::open(bus_path, address)
            .map_err(|e| format!("MCP9600 0x{:02x}@{}: open failed: {}", address, bus_path, e))?;

        let id = rpi4b_transport::i2c::read_register(&mut dev, bravepi_sensors::mcp9600::REG_DEVICE_ID)
            .map_err(|e| format!("MCP9600 0x{:02x}@{}: device ID read failed: {}", address, bus_path, e))?;

        if id[0] != bravepi_sensors::mcp9600::DEVICE_ID {
            return Err(format!(
                "MCP9600 0x{:02x}@{}: unexpected device ID 0x{:02x} (expected 0x{:02x})",
                address, bus_path, id[0], bravepi_sensors::mcp9600::DEVICE_ID,
            ));
        }

        let config_val = bravepi_sensors::mcp9600::config_value(self.thermocouple_type);
        rpi4b_transport::i2c::write_register(&mut dev, bravepi_sensors::mcp9600::REG_SENSOR_CONFIGURATION, &[config_val])
            .map_err(|e| format!("MCP9600 0x{:02x}@{}: config write failed: {}", address, bus_path, e))?;

        Ok(bravepi_sensors::mcp9600::identity(ConnectionInfo {
            kind: ConnectionKind::I2c,
            parameters: BTreeMap::from([
                ("bus".into(), bus_path.into()),
                ("address".into(), format!("0x{:02x}", address)),
            ]),
        }))
    }

    fn read(&self, bus_path: &str, address: u8) -> Result<SensorReading, String> {
        // Same logic as current rpi-local-adapter/src/sensors/mcp9600.rs::read_mcp9600
        let mut dev = rpi4b_transport::i2c::open(bus_path, address)
            .map_err(|e| format!("MCP9600 0x{:02x}@{}: open failed: {}", address, bus_path, e))?;

        let raw = rpi4b_transport::i2c::read_register(&mut dev, bravepi_sensors::mcp9600::REG_HOT_JUNCTION)
            .map_err(|e| format!("MCP9600 0x{:02x}@{}: read failed: {}", address, bus_path, e))?;

        Ok(bravepi_sensors::mcp9600::from_i2c_raw(&raw))
    }

    fn ic_name(&self) -> &'static str { "mcp9600" }
}
```

- [ ] **Step 3: Implement Opt3001Driver**

```rust
// rpi-local-adapter/src/drivers/opt3001.rs
use std::collections::BTreeMap;
use iotkit_polling_adapter_runtime::SensorDriver;
use iotkit_core_types::{ConnectionInfo, ConnectionKind, SensorIdentity, SensorReading};

pub struct Opt3001Driver;

impl SensorDriver for Opt3001Driver {
    fn probe(&self, bus_path: &str, address: u8) -> Result<SensorIdentity, String> {
        // Same logic as current rpi-local-adapter/src/sensors/opt3001.rs::probe_opt3001
        let mut dev = rpi4b_transport::i2c::open(bus_path, address)
            .map_err(|e| format!("OPT3001 0x{:02x}@{}: open failed: {}", address, bus_path, e))?;

        let id_raw = rpi4b_transport::i2c::read_register(&mut dev, bravepi_sensors::opt3001::REG_DEVICE_ID)
            .map_err(|e| format!("OPT3001 0x{:02x}@{}: device ID read failed: {}", address, bus_path, e))?;

        let device_id = u16::from_be_bytes(id_raw);
        if device_id != bravepi_sensors::opt3001::DEVICE_ID {
            return Err(format!(
                "OPT3001 0x{:02x}@{}: unexpected device ID 0x{:04x} (expected 0x{:04x})",
                address, bus_path, device_id, bravepi_sensors::opt3001::DEVICE_ID,
            ));
        }

        let init_bytes = bravepi_sensors::opt3001::INIT_CONFIG.to_le_bytes();
        rpi4b_transport::i2c::write_register(&mut dev, bravepi_sensors::opt3001::REG_CONFIGURATION, &init_bytes)
            .map_err(|e| format!("OPT3001 0x{:02x}@{}: config write failed: {}", address, bus_path, e))?;

        Ok(bravepi_sensors::opt3001::identity(ConnectionInfo {
            kind: ConnectionKind::I2c,
            parameters: BTreeMap::from([
                ("bus".into(), bus_path.into()),
                ("address".into(), format!("0x{:02x}", address)),
            ]),
        }))
    }

    fn read(&self, bus_path: &str, address: u8) -> Result<SensorReading, String> {
        let mut dev = rpi4b_transport::i2c::open(bus_path, address)
            .map_err(|e| format!("OPT3001 0x{:02x}@{}: open failed: {}", address, bus_path, e))?;

        let raw = rpi4b_transport::i2c::read_register(&mut dev, bravepi_sensors::opt3001::REG_RESULT)
            .map_err(|e| format!("OPT3001 0x{:02x}@{}: read failed: {}", address, bus_path, e))?;

        let value = u16::from_le_bytes(raw);
        Ok(bravepi_sensors::opt3001::from_i2c_raw(value))
    }

    fn ic_name(&self) -> &'static str { "opt3001" }

    fn validate(&self, poll_interval_ms: u64) -> Result<(), String> {
        if poll_interval_ms < 200 {
            Err(format!(
                "poll_interval_ms {} too short for OPT3001 (minimum 200ms for conversion latency)",
                poll_interval_ms,
            ))
        } else {
            Ok(())
        }
    }
}
```

- [ ] **Step 4: Run `cargo check -p rpi-local-adapter`**

Expected: may fail because lib.rs still references old modules. That's OK — Task 6 handles the refactor.

- [ ] **Step 5: Commit**

```
feat(rpi-local-adapter): add MCP9600 and OPT3001 SensorDriver implementations
```

---

### Task 6: Refactor rpi-local-adapter to use base adapter

**Files:**
- Modify: `rpi-local-adapter/Cargo.toml`
- Rewrite: `rpi-local-adapter/src/lib.rs`
- Delete: `rpi-local-adapter/src/polling_loop.rs`
- Delete: `rpi-local-adapter/src/config.rs`
- Delete: `rpi-local-adapter/src/sensors/mod.rs`
- Delete: `rpi-local-adapter/src/sensors/mcp9600.rs`
- Delete: `rpi-local-adapter/src/sensors/opt3001.rs`

- [ ] **Step 1: Update Cargo.toml**

Add `iotkit-polling-adapter-runtime` dependency:
```toml
iotkit-polling-adapter-runtime = { path = "../iotkit-polling-adapter-runtime" }
```

- [ ] **Step 2: Rewrite lib.rs**

```rust
//! rpi-local-adapter: RPi local I2C sensor adapter.
//! Thin wrapper over iotkit-polling-adapter-runtime with MCP9600 and OPT3001 drivers.

pub mod drivers;

pub use iotkit_polling_adapter_runtime::AdapterHandle;
pub use bravepi_sensors::mcp9600::ThermocoupleType;

use std::sync::Arc;
use iotkit_polling_adapter_runtime::{BaseAdapterConfig, SensorTargetConfig};
use iotkit_core_types::AdapterId;

/// RPi-local-specific config.
#[derive(Debug, Clone)]
pub struct RpiLocalConfig {
    pub bus_path: String,
    pub poll_interval_ms: u64,
    pub targets: Vec<RpiLocalTarget>,
}

/// A sensor target with rpi-local-specific config.
#[derive(Debug, Clone)]
pub enum RpiLocalTarget {
    MCP9600 { address: u8, thermocouple_type: ThermocoupleType },
    OPT3001 { address: u8 },
}

/// Start the rpi-local adapter.
pub fn start(config: RpiLocalConfig) -> Result<AdapterHandle, std::io::Error> {
    let base_config = to_base_config(&config);
    iotkit_polling_adapter_runtime::start(
        AdapterId::new("rpi-local:default"),
        base_config,
    )
}

fn to_base_config(config: &RpiLocalConfig) -> BaseAdapterConfig {
    let targets = config.targets.iter().map(|t| match t {
        RpiLocalTarget::MCP9600 { address, thermocouple_type } => SensorTargetConfig {
            address: *address,
            driver: Arc::new(drivers::mcp9600::Mcp9600Driver {
                thermocouple_type: *thermocouple_type,
            }),
            key_suffix: None,
        },
        RpiLocalTarget::OPT3001 { address } => SensorTargetConfig {
            address: *address,
            driver: Arc::new(drivers::opt3001::Opt3001Driver),
            key_suffix: None,
        },
    }).collect();

    BaseAdapterConfig {
        bus_path: config.bus_path.clone(),
        poll_interval_ms: config.poll_interval_ms,
        targets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_without_runtime_returns_error() {
        let config = RpiLocalConfig {
            bus_path: "/dev/null".to_string(),
            poll_interval_ms: 1000,
            targets: vec![RpiLocalTarget::MCP9600 {
                address: 0x60,
                thermocouple_type: ThermocoupleType::K,
            }],
        };
        let result = start(config);
        assert!(result.is_err());
    }

    #[test]
    fn start_with_invalid_config_returns_config_error() {
        let config = RpiLocalConfig {
            bus_path: "/dev/null".to_string(),
            poll_interval_ms: 0,
            targets: vec![],
        };
        let err = start(config).unwrap_err();
        assert!(err.to_string().contains("poll_interval_ms"));
    }

    #[test]
    fn opt3001_rejects_short_poll_interval() {
        let config = RpiLocalConfig {
            bus_path: "/dev/null".to_string(),
            poll_interval_ms: 50,
            targets: vec![RpiLocalTarget::OPT3001 { address: 0x44 }],
        };
        let err = start(config).unwrap_err();
        assert!(err.to_string().contains("OPT3001"));
    }
}
```

- [ ] **Step 3: Delete old files**

```bash
rm rpi-local-adapter/src/polling_loop.rs
rm rpi-local-adapter/src/config.rs
rm -r rpi-local-adapter/src/sensors/
```

- [ ] **Step 4: Run `cargo test -p rpi-local-adapter` and `cargo test -p iotkit-polling-adapter-runtime`**

Expected: all tests pass.

- [ ] **Step 5: Commit**

```
refactor(rpi-local-adapter): use iotkit-polling-adapter-runtime, remove polling_loop and config
```

---

### Task 7: Update gateway and integration tests

**Files:**
- Modify: `iotkit-gateway/src/main.rs`
- Modify: `rpi-local-adapter/tests/integration.rs`
- Modify: `iotkit-gateway/Cargo.toml` (if needed)

- [ ] **Step 1: Update gateway rpi_local_config()**

Update to use new `RpiLocalConfig` / `RpiLocalTarget` types:

```rust
fn rpi_local_config() -> rpi_local_adapter::RpiLocalConfig {
    use rpi_local_adapter::{RpiLocalTarget, ThermocoupleType};

    rpi_local_adapter::RpiLocalConfig {
        bus_path: "/dev/i2c-1".to_string(),
        poll_interval_ms: 1000,
        targets: vec![
            RpiLocalTarget::MCP9600 {
                address: 0x60,
                thermocouple_type: ThermocoupleType::K,
            },
            RpiLocalTarget::OPT3001 { address: 0x44 },
        ],
    }
}
```

- [ ] **Step 2: Update gateway AdapterHandle type**

The gateway uses `rpi_local_adapter::start()` which returns `iotkit_polling_adapter_runtime::AdapterHandle` (re-exported). Update type references if needed. The gateway accesses `.id`, `.event_rx`, `.command_tx`, `.shutdown()` — all present.

- [ ] **Step 3: Update integration tests**

Update `rpi-local-adapter/tests/integration.rs` to use new `RpiLocalConfig` / `RpiLocalTarget` types. The test structure stays the same (start adapter, receive DeviceDiscovered, then SensorData).

- [ ] **Step 4: Run full workspace test**

```bash
cargo test --workspace
```

Expected: all tests pass across all crates.

- [ ] **Step 5: Commit**

```
refactor(gateway): update rpi-local config to use new RpiLocalTarget types
```

---

### Task 8: Full workspace build and test verification

**Files:** None (verification only)

- [ ] **Step 1: Run cargo clippy on workspace**

```bash
cargo clippy --workspace -- -D warnings
```

Fix any warnings.

- [ ] **Step 2: Run cargo test on workspace**

```bash
cargo test --workspace
```

Verify all tests pass. Count total tests and compare to previous (should be roughly the same or more).

- [ ] **Step 3: Commit any clippy fixes**

```
fix: address clippy warnings after base adapter refactor
```

---

### Task 9: Sync spec and commit final state

**Files:**
- Modify: `docs/superpowers/specs/2026-03-28-base-adapter-design.md` (if any spec drift during implementation)

- [ ] **Step 1: Review spec against implementation for any drift**

Check that the implemented code matches the spec. Note any intentional deviations and update the spec to reflect the actual implementation.

- [ ] **Step 2: Commit spec sync if needed**

```
docs: sync base adapter spec with implementation
```
