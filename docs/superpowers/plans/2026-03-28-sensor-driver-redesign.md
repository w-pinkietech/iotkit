# SensorDriver Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split `SensorDriver::probe()` into read-only `detect()` + separate `init()` for safe I2C bus scanning.

**Architecture:** Replace `probe()` with `detect()` + `init()` in the `SensorDriver` trait. Add `Detected` intermediate state to polling loop. Same-cycle detect+init preserves current single-tick discovery latency.

**Tech Stack:** Rust, tokio

---

## Task Dependencies

```
Task 1 (all production code) ──> Task 2 (update existing tests) ──> Task 3 (add new tests)
```

- **Task 1** updates the trait (lib.rs), both drivers (mcp9600.rs, opt3001.rs), AND all polling_loop.rs production code (state machine, poll_cycle, apply_outcomes, startup, main loop). 4 files. **Gate:** `cargo check --workspace` passes.
- **Task 2** updates all existing tests in polling_loop.rs to use new names/types. 1 file. **Gate:** `cargo test -p iotkit-polling-adapter-runtime` passes.
- **Task 3** adds new init-failure and Detected-state tests. 1 file. **Gate:** `cargo test --workspace` passes.

**Green-state guarantee:** Every task ends with its listed gate command passing. No intermediate broken commits.

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| Modify | `iotkit-polling-adapter-runtime/src/lib.rs` | Trait: `probe()` → `detect()` + `init()`. Test drivers updated. |
| Modify | `iotkit-polling-adapter-runtime/src/polling_loop.rs` | State machine: `Detected` state, 3-phase `poll_cycle()`, `PollOutcome` updated. |
| Modify | `rpi-local-adapter/src/drivers/mcp9600.rs` | Split `probe()` into `detect()` + `init()`. |
| Modify | `rpi-local-adapter/src/drivers/opt3001.rs` | Split `probe()` into `detect()` + `init()`. |

---

### Task 1: All production code changes (trait + drivers + state machine + loop)

This task covers ALL production code changes. It is subdivided into phases for clarity, but forms a single green commit.

**Files:**
- Modify: `iotkit-polling-adapter-runtime/src/lib.rs`
- Modify: `rpi-local-adapter/src/drivers/mcp9600.rs`
- Modify: `rpi-local-adapter/src/drivers/opt3001.rs`
- Modify: `iotkit-polling-adapter-runtime/src/polling_loop.rs`

#### Phase A: Update SensorDriver trait and test drivers

**File:** `iotkit-polling-adapter-runtime/src/lib.rs`

- [ ] **Step 1: Update SensorDriver trait**

Replace the `probe` method with `detect` and `init` in the trait definition at `iotkit-polling-adapter-runtime/src/lib.rs`:

```rust
pub trait SensorDriver: Send + Sync {
    /// Read-only detection. Must NOT write to hardware.
    /// Reads device ID registers and returns identity on match.
    ///
    /// Error strings **must** include the bus path and address for field debugging.
    fn detect(&self, bus_path: &str, address: u8) -> Result<SensorIdentity, String>;

    /// Initialize hardware by writing config registers.
    /// Called only after detect() succeeds. Must be idempotent.
    ///
    /// Error strings **must** include the bus path and address for field debugging.
    fn init(&self, bus_path: &str, address: u8) -> Result<(), String>;

    /// Read the sensor at the given I2C address. Returns a reading on success.
    ///
    /// Error strings **must** include the bus path and address for field debugging.
    fn read(&self, bus_path: &str, address: u8) -> Result<SensorReading, String>;

    /// Return the IC part name (e.g. "opt3001", "mcp9600").
    fn ic_name(&self) -> &'static str;
    fn validate(&self, poll_interval_ms: u64) -> Result<(), String> {
        let _ = poll_interval_ms;
        Ok(())
    }
}
```

Also update the panic safety doc comment above the trait to reference `detect()`, `init()`, and `read()` instead of `probe()` and `read()`.

- [ ] **Step 2: Update StubDriver test impl**

In the `tests` module, replace the `probe` method on `StubDriver`:

```rust
impl SensorDriver for StubDriver {
    fn detect(&self, _bus_path: &str, _address: u8) -> Result<SensorIdentity, String> {
        Ok(SensorIdentity {
            manufacturer: "Test".into(),
            ic_part_number: "STUB".into(),
            sensor_type: SensorType::Temperature,
            connection: ConnectionInfo {
                kind: ConnectionKind::I2c,
                parameters: BTreeMap::new(),
            },
        })
    }
    fn init(&self, _bus_path: &str, _address: u8) -> Result<(), String> {
        Ok(())
    }
    fn read(&self, _bus_path: &str, _address: u8) -> Result<SensorReading, String> {
        Ok(SensorReading::empty(SensorType::Temperature))
    }
    fn ic_name(&self) -> &'static str {
        "STUB"
    }
}
```

- [ ] **Step 3: Update StrictDriver test impl**

Replace the `probe` method on `StrictDriver`:

```rust
impl SensorDriver for StrictDriver {
    fn detect(&self, _bus_path: &str, _address: u8) -> Result<SensorIdentity, String> {
        Ok(SensorIdentity {
            manufacturer: "Test".into(),
            ic_part_number: "STRICT".into(),
            sensor_type: SensorType::Temperature,
            connection: ConnectionInfo {
                kind: ConnectionKind::I2c,
                parameters: BTreeMap::new(),
            },
        })
    }
    fn init(&self, _bus_path: &str, _address: u8) -> Result<(), String> {
        Ok(())
    }
    fn read(&self, _bus_path: &str, _address: u8) -> Result<SensorReading, String> {
        Ok(SensorReading::empty(SensorType::Temperature))
    }
    fn ic_name(&self) -> &'static str {
        "STRICT"
    }
    fn validate(&self, poll_interval_ms: u64) -> Result<(), String> {
        if poll_interval_ms < self.min_interval_ms {
            return Err(format!(
                "poll_interval_ms {} too short, minimum is {}",
                poll_interval_ms, self.min_interval_ms,
            ));
        }
        Ok(())
    }
}
```

#### Phase B: Split MCP9600 driver

**File:** `rpi-local-adapter/src/drivers/mcp9600.rs`

- [ ] **Step 1: Replace probe() with detect() and init()**

Replace the entire `SensorDriver` impl block in `mcp9600.rs`:

```rust
impl SensorDriver for Mcp9600Driver {
    fn detect(&self, bus_path: &str, address: u8) -> Result<SensorIdentity, String> {
        let mut t = I2cTransport::open(bus_path, &I2cConfig { address: address as u16 })
            .map_err(|e| format!("MCP9600 0x{:02x}@{}: I2C open: {}", address, bus_path, e))?;

        let mut id_buf = [0u8; 2];
        t.read_register(mcp9600::REG_DEVICE_ID, &mut id_buf)
            .map_err(|e| {
                format!(
                    "MCP9600 0x{:02x}@{}: read REG_DEVICE_ID: {}",
                    address, bus_path, e
                )
            })?;

        if id_buf[0] != mcp9600::DEVICE_ID {
            return Err(format!(
                "MCP9600 0x{:02x}@{}: device ID mismatch: expected 0x{:02x}, got 0x{:02x}",
                address,
                bus_path,
                mcp9600::DEVICE_ID,
                id_buf[0],
            ));
        }

        let connection = ConnectionInfo {
            kind: ConnectionKind::I2c,
            parameters: BTreeMap::from([
                ("bus".into(), bus_path.to_string()),
                ("address".into(), format!("0x{:02x}", address)),
            ]),
        };
        Ok(mcp9600::identity(connection))
    }

    fn init(&self, bus_path: &str, address: u8) -> Result<(), String> {
        let mut t = I2cTransport::open(bus_path, &I2cConfig { address: address as u16 })
            .map_err(|e| format!("MCP9600 0x{:02x}@{}: I2C open: {}", address, bus_path, e))?;

        let config_val = mcp9600::config_value(self.thermocouple_type);
        t.write_register(mcp9600::REG_SENSOR_CONFIGURATION, &[config_val])
            .map_err(|e| {
                format!(
                    "MCP9600 0x{:02x}@{}: write REG_SENSOR_CONFIGURATION: {}",
                    address, bus_path, e
                )
            })?;

        Ok(())
    }

    fn read(&self, bus_path: &str, address: u8) -> Result<SensorReading, String> {
        let mut t = I2cTransport::open(bus_path, &I2cConfig { address: address as u16 })
            .map_err(|e| format!("MCP9600 0x{:02x}@{}: I2C open: {}", address, bus_path, e))?;

        let mut raw = [0u8; 2];
        t.read_register(mcp9600::REG_HOT_JUNCTION, &mut raw)
            .map_err(|e| {
                format!(
                    "MCP9600 0x{:02x}@{}: read REG_HOT_JUNCTION: {}",
                    address, bus_path, e
                )
            })?;

        Ok(mcp9600::from_i2c_raw(&raw))
    }

    fn ic_name(&self) -> &'static str {
        "mcp9600"
    }
}
```

- [ ] **Step 2: Verify detect() is read-only (reproducible check)**

Run: `sed -n '/fn detect/,/fn init/p' rpi-local-adapter/src/drivers/mcp9600.rs | grep -c write_register`
Expected: `0` (no write_register calls in detect body). If non-zero, the split was done incorrectly.

Also verify init has the write: `sed -n '/fn init/,/fn read/p' rpi-local-adapter/src/drivers/mcp9600.rs | grep -c write_register`
Expected: `1` (REG_SENSOR_CONFIGURATION write).

#### Phase C: Split OPT3001 driver

**File:** `rpi-local-adapter/src/drivers/opt3001.rs`

- [ ] **Step 1: Replace probe() with detect() and init()**

Replace the entire `SensorDriver` impl block in `opt3001.rs`:

```rust
impl SensorDriver for Opt3001Driver {
    fn detect(&self, bus_path: &str, address: u8) -> Result<SensorIdentity, String> {
        let mut t = I2cTransport::open(bus_path, &I2cConfig { address: address as u16 })
            .map_err(|e| format!("OPT3001 0x{:02x}@{}: I2C open: {}", address, bus_path, e))?;

        let mut id_buf = [0u8; 2];
        t.read_register(opt3001::REG_DEVICE_ID, &mut id_buf)
            .map_err(|e| {
                format!(
                    "OPT3001 0x{:02x}@{}: read REG_DEVICE_ID: {}",
                    address, bus_path, e
                )
            })?;

        let device_id = u16::from_be_bytes(id_buf);
        if device_id != opt3001::DEVICE_ID {
            return Err(format!(
                "OPT3001 0x{:02x}@{}: device ID mismatch: expected 0x{:04x}, got 0x{:04x}",
                address, bus_path, opt3001::DEVICE_ID, device_id,
            ));
        }

        let connection = ConnectionInfo {
            kind: ConnectionKind::I2c,
            parameters: BTreeMap::from([
                ("bus".into(), bus_path.to_string()),
                ("address".into(), format!("0x{:02x}", address)),
            ]),
        };
        Ok(opt3001::identity(connection))
    }

    fn init(&self, bus_path: &str, address: u8) -> Result<(), String> {
        let mut t = I2cTransport::open(bus_path, &I2cConfig { address: address as u16 })
            .map_err(|e| format!("OPT3001 0x{:02x}@{}: I2C open: {}", address, bus_path, e))?;

        let config_bytes = opt3001::INIT_CONFIG.to_le_bytes();
        t.write_register(opt3001::REG_CONFIG, &config_bytes)
            .map_err(|e| {
                format!(
                    "OPT3001 0x{:02x}@{}: write REG_CONFIG: {}",
                    address, bus_path, e
                )
            })?;

        Ok(())
    }

    fn read(&self, bus_path: &str, address: u8) -> Result<SensorReading, String> {
        let mut t = I2cTransport::open(bus_path, &I2cConfig { address: address as u16 })
            .map_err(|e| format!("OPT3001 0x{:02x}@{}: I2C open: {}", address, bus_path, e))?;

        let mut raw = [0u8; 2];
        t.read_register(opt3001::REG_RESULT, &mut raw)
            .map_err(|e| {
                format!(
                    "OPT3001 0x{:02x}@{}: read REG_RESULT: {}",
                    address, bus_path, e
                )
            })?;

        let swapped = u16::from_le_bytes(raw);
        Ok(opt3001::from_i2c_raw(swapped))
    }

    fn ic_name(&self) -> &'static str {
        "opt3001"
    }

    fn validate(&self, poll_interval_ms: u64) -> Result<(), String> {
        if poll_interval_ms < 200 {
            return Err(format!(
                "poll_interval_ms {} too short for OPT3001 (minimum 200ms for conversion latency)",
                poll_interval_ms,
            ));
        }
        Ok(())
    }
}
```

- [ ] **Step 2: Verify detect() is read-only (reproducible check)**

Run: `sed -n '/fn detect/,/fn init/p' rpi-local-adapter/src/drivers/opt3001.rs | grep -c write_register`
Expected: `0` (no write_register calls in detect body). If non-zero, the split was done incorrectly.

Also verify init has the write: `sed -n '/fn init/,/fn read/p' rpi-local-adapter/src/drivers/opt3001.rs | grep -c write_register`
Expected: `1` (REG_CONFIG write).

#### Phase D: Update polling_loop state machine, PollOutcome, poll_cycle, apply_outcomes

**File:** `iotkit-polling-adapter-runtime/src/polling_loop.rs`

This is the largest phase. It updates the core state machine.

- [ ] **Step 1: Update constants**

Replace `MAX_PROBE_FAILURES` with `MAX_DETECT_FAILURES` and add `MAX_INIT_FAILURES`:

```rust
const MAX_READ_FAILURES: u32 = 5;
const MAX_DETECT_FAILURES: u32 = 10;
const MAX_INIT_FAILURES: u32 = 5;
```

- [ ] **Step 2: Update TargetState**

Replace the `TargetState` enum and its `impl`:

```rust
#[derive(Clone)]
pub(crate) enum TargetState {
    Pending {
        consecutive_detect_failures: u32,
        escalation_emitted: bool,
    },
    Detected {
        identity: SensorIdentity,
        consecutive_init_failures: u32,
    },
    Active {
        key: DeviceKey,
        consecutive_read_failures: u32,
    },
}

impl TargetState {
    pub(crate) fn new_pending() -> Self {
        TargetState::Pending {
            consecutive_detect_failures: 0,
            escalation_emitted: false,
        }
    }
}
```

Note: `SensorIdentity` must derive `Clone` for the `Detected` state to hold it. It already does (`core/types/src/lib.rs` line 80). No changes to `core/types` needed.

- [ ] **Step 3: Update PollOutcome**

Replace the entire `PollOutcome` enum:

```rust
#[derive(Debug)]
pub(crate) enum PollOutcome {
    /// detect() + init() both succeeded. Emit DeviceDiscovered.
    Discovered {
        target_index: usize,
        key: DeviceKey,
        identity: SensorIdentity,
    },
    /// detect() succeeded but init() failed. Enter/maintain Detected state.
    InitFailed {
        target_index: usize,
        identity: SensorIdentity,
        message: String,
        is_panic: bool,
    },
    /// Successful sensor reading.
    Reading {
        key: DeviceKey,
        reading: SensorReading,
        observed_at: std::time::SystemTime,
    },
    /// read() failed.
    ReadError {
        target_index: usize,
        key: DeviceKey,
        message: String,
        #[allow(dead_code)]
        is_panic: bool,
    },
    /// detect() failed.
    DetectFailed {
        target_index: usize,
        message: String,
        is_panic: bool,
    },
}
```

- [ ] **Step 4: Update poll_cycle for 3-phase flow**

Replace the entire `poll_cycle` function:

```rust
pub(crate) fn poll_cycle(
    targets: &[TargetRuntime],
    states: &[TargetState],
    bus_path: &str,
) -> Vec<PollOutcome> {
    let mut outcomes = Vec::new();
    for (i, target) in targets.iter().enumerate() {
        match &states[i] {
            TargetState::Pending { .. } => {
                // Phase 1: detect (read-only)
                let detect_result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    target.driver.detect(bus_path, target.address)
                }));

                match detect_result {
                    Ok(Ok(identity)) => {
                        // detect succeeded — immediately try init (same cycle)
                        let init_result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                            target.driver.init(bus_path, target.address)
                        }));
                        match init_result {
                            Ok(Ok(())) => {
                                let key = device_key_for(target.address, &target.key_suffix);
                                outcomes.push(PollOutcome::Discovered {
                                    target_index: i,
                                    key,
                                    identity,
                                });
                            }
                            Ok(Err(msg)) => {
                                outcomes.push(PollOutcome::InitFailed {
                                    target_index: i,
                                    identity,
                                    message: msg,
                                    is_panic: false,
                                });
                            }
                            Err(panic_val) => {
                                let msg = panic_message(&panic_val);
                                tracing::error!(
                                    address = format_args!("0x{:02x}", target.address),
                                    bus_path,
                                    "driver panicked during init: {msg}",
                                );
                                outcomes.push(PollOutcome::InitFailed {
                                    target_index: i,
                                    identity,
                                    message: format!(
                                        "driver panic during init 0x{:02x}@{}: {}",
                                        target.address, bus_path, msg,
                                    ),
                                    is_panic: true,
                                });
                            }
                        }
                    }
                    Ok(Err(msg)) => {
                        outcomes.push(PollOutcome::DetectFailed {
                            target_index: i,
                            message: msg,
                            is_panic: false,
                        });
                    }
                    Err(panic_val) => {
                        let msg = panic_message(&panic_val);
                        tracing::error!(
                            address = format_args!("0x{:02x}", target.address),
                            bus_path,
                            "driver panicked during detect: {msg}",
                        );
                        outcomes.push(PollOutcome::DetectFailed {
                            target_index: i,
                            message: format!(
                                "driver panic during detect 0x{:02x}@{}: {}",
                                target.address, bus_path, msg,
                            ),
                            is_panic: true,
                        });
                    }
                }
            }
            TargetState::Detected { identity, .. } => {
                // Phase 2: init retry (from previous failed init)
                let init_result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    target.driver.init(bus_path, target.address)
                }));
                match init_result {
                    Ok(Ok(())) => {
                        let key = device_key_for(target.address, &target.key_suffix);
                        outcomes.push(PollOutcome::Discovered {
                            target_index: i,
                            key,
                            identity: identity.clone(),
                        });
                    }
                    Ok(Err(msg)) => {
                        outcomes.push(PollOutcome::InitFailed {
                            target_index: i,
                            identity: identity.clone(),
                            message: msg,
                            is_panic: false,
                        });
                    }
                    Err(panic_val) => {
                        let msg = panic_message(&panic_val);
                        tracing::error!(
                            address = format_args!("0x{:02x}", target.address),
                            bus_path,
                            "driver panicked during init: {msg}",
                        );
                        outcomes.push(PollOutcome::InitFailed {
                            target_index: i,
                            identity: identity.clone(),
                            message: format!(
                                "driver panic during init 0x{:02x}@{}: {}",
                                target.address, bus_path, msg,
                            ),
                            is_panic: true,
                        });
                    }
                }
            }
            TargetState::Active { key, .. } => {
                // Phase 3: read (unchanged logic)
                match panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    target.driver.read(bus_path, target.address)
                })) {
                    Ok(Ok(reading)) => {
                        outcomes.push(PollOutcome::Reading {
                            key: key.clone(),
                            reading,
                            observed_at: std::time::SystemTime::now(),
                        });
                    }
                    Ok(Err(msg)) => {
                        outcomes.push(PollOutcome::ReadError {
                            target_index: i,
                            key: key.clone(),
                            message: msg,
                            is_panic: false,
                        });
                    }
                    Err(panic_val) => {
                        let msg = panic_message(&panic_val);
                        tracing::error!(
                            address = format_args!("0x{:02x}", target.address),
                            bus_path,
                            "driver panicked during read: {msg}",
                        );
                        outcomes.push(PollOutcome::ReadError {
                            target_index: i,
                            key: key.clone(),
                            message: format!(
                                "driver panic during read 0x{:02x}@{}: {}",
                                target.address, bus_path, msg,
                            ),
                            is_panic: true,
                        });
                    }
                }
            }
        }
    }
    outcomes
}
```

- [ ] **Step 5: Update apply_outcomes**

Replace the entire `apply_outcomes` function. The key changes: handle `InitFailed` and `DetectFailed`, update counter field names, add `Detected` state transitions:

```rust
pub(crate) fn apply_outcomes(
    outcomes: Vec<PollOutcome>,
    states: &mut [TargetState],
    targets: &[TargetRuntime],
) -> Vec<AdapterEvent> {
    let mut events = Vec::new();

    for outcome in outcomes {
        match outcome {
            PollOutcome::Discovered {
                target_index,
                key,
                identity,
            } => {
                states[target_index] = TargetState::Active {
                    key: key.clone(),
                    consecutive_read_failures: 0,
                };
                events.push(AdapterEvent::DeviceDiscovered {
                    device_key: key,
                    identity,
                });
            }

            PollOutcome::InitFailed {
                target_index,
                identity,
                message,
                ..
            } => {
                let new_failures = match &states[target_index] {
                    TargetState::Detected { consecutive_init_failures, .. } => {
                        consecutive_init_failures + 1
                    }
                    _ => 1, // First init failure (from Pending same-cycle)
                };

                if new_failures >= MAX_INIT_FAILURES {
                    // Too many init failures — return to Pending for fresh detect
                    states[target_index] = TargetState::new_pending();
                } else {
                    states[target_index] = TargetState::Detected {
                        identity,
                        consecutive_init_failures: new_failures,
                    };
                }

                events.push(AdapterEvent::AdapterError {
                    device_key: None,
                    error: format!(
                        "init failed ({}/{}): {}",
                        new_failures, MAX_INIT_FAILURES, message,
                    ),
                });
            }

            PollOutcome::Reading { key, reading, observed_at } => {
                for state in states.iter_mut() {
                    if let TargetState::Active { key: k, consecutive_read_failures } = state
                        && k.as_str() == key.as_str()
                    {
                        *consecutive_read_failures = 0;
                        break;
                    }
                }
                events.push(AdapterEvent::SensorData {
                    device_key: key,
                    reading,
                    rssi: None,
                    battery_pct: None,
                    ingested_at: observed_at,
                });
            }

            PollOutcome::ReadError {
                target_index,
                key,
                message,
                ..
            } => {
                if let TargetState::Active {
                    ref mut consecutive_read_failures,
                    ..
                } = states[target_index]
                {
                    *consecutive_read_failures += 1;
                    let n = *consecutive_read_failures;

                    events.push(AdapterEvent::AdapterError {
                        device_key: Some(key.clone()),
                        error: message.clone(),
                    });

                    if n >= MAX_READ_FAILURES {
                        let reason = format!(
                            "consecutive read failures ({n}): {message}"
                        );
                        tracing::info!(
                            device_key = key.as_str(),
                            "device lost: {reason}"
                        );
                        events.push(AdapterEvent::DeviceLost {
                            device_key: key,
                            reason,
                        });
                        states[target_index] = TargetState::new_pending();
                    }
                }
            }

            PollOutcome::DetectFailed {
                target_index,
                message,
                is_panic,
            } => {
                if let TargetState::Pending {
                    ref mut consecutive_detect_failures,
                    ref mut escalation_emitted,
                } = states[target_index]
                {
                    *consecutive_detect_failures += 1;
                    let n = *consecutive_detect_failures;

                    if is_panic {
                        // Panic: emit immediately but do NOT consume escalation_emitted,
                        // so the normal threshold path still fires later if needed.
                        // (Preserves current probe panic behavior.)
                        let addr = targets[target_index].address;
                        events.push(AdapterEvent::AdapterError {
                            device_key: None,
                            error: format!(
                                "target 0x{addr:02x} detect failed (driver panic): {message}"
                            ),
                        });
                    } else if n >= MAX_DETECT_FAILURES && !*escalation_emitted {
                        let addr = targets[target_index].address;
                        events.push(AdapterEvent::AdapterError {
                            device_key: None,
                            error: format!(
                                "target 0x{addr:02x} detect failed {n} consecutive times: {message}"
                            ),
                        });
                        *escalation_emitted = true;
                    } else {
                        tracing::warn!(
                            target_index,
                            "detect failed: {message}"
                        );
                    }
                }
            }
        }
    }

    events
}
```

#### Phase E: Update polling_loop main loop and startup

**File:** `iotkit-polling-adapter-runtime/src/polling_loop.rs` (continued)

**Note:** `SensorIdentity`, `ConnectionInfo`, and all other `core/types` structs already derive `Clone`. No changes to `core/types/src/lib.rs` are needed.

- [ ] **Step 1: Update startup probe section**

In the `polling_loop` async function, replace the startup probe section. The key changes: use `TargetState::clone()` instead of manual match, and update `all_failed` to check both `DetectFailed` and `InitFailed`:

Replace the state snapshot and `all_failed` check:

```rust
    // ── Startup probe ────────────────────────────────────
    if !targets.is_empty() {
        let t = Arc::clone(&targets);
        let bp = bus_path.clone();
        let s_snap: Vec<TargetState> = states.iter().cloned().collect();

        let outcomes = match tokio::task::spawn_blocking(move || poll_cycle(&t, &s_snap, &bp)).await {
            Ok(outcomes) => outcomes,
            Err(e) => {
                tracing::error!("startup probe spawn_blocking failed: {e}");
                let event = AdapterEvent::AdapterError {
                    device_key: None,
                    error: format!("fatal: startup probe task failed: {e}"),
                };
                if event_tx.send(event).await.is_err() {
                    tracing::warn!("event channel closed while sending fatal startup error");
                }
                return;
            }
        };

        let all_failed = !outcomes.is_empty()
            && outcomes.iter().all(|o| {
                matches!(o, PollOutcome::DetectFailed { .. } | PollOutcome::InitFailed { .. })
            });

        let events = apply_outcomes(outcomes, &mut states, &targets);

        for event in events {
            if event_tx.send(event).await.is_err() {
                tracing::warn!("event channel closed during startup probe");
                return;
            }
        }

        if all_failed {
            let addrs: Vec<String> = targets.iter().map(|t| format!("0x{:02x}", t.address)).collect();
            let event = AdapterEvent::AdapterError {
                device_key: None,
                error: format!(
                    "all targets failed startup probe on bus {}: [{}]",
                    bus_path, addrs.join(", "),
                ),
            };
            if event_tx.send(event).await.is_err() {
                tracing::warn!("event channel closed during startup probe");
                return;
            }
        }
    }
```

- [ ] **Step 2: Confirm TargetState derives Clone**

Phase D already added `#[derive(Clone)]` to `TargetState`. Verify this is present — it enables `.iter().cloned().collect()` in startup and main loop, replacing the verbose manual `match` snapshot. No action needed if Task 4 was applied correctly.

- [ ] **Step 3: Update main loop ticker section**

Replace the state snapshot in the main loop's ticker branch:

```rust
            _ = ticker.tick() => {
                let t = Arc::clone(&targets);
                let bp = bus_path.clone();
                let s_snap: Vec<TargetState> = states.iter().cloned().collect();

                // ... rest unchanged
```

- [ ] **Step 4: Verify compilation passes**

Run: `cargo check --workspace`
Expected: All production code compiles. Tests will fail (still reference `probe`), but `cargo check` (which does not run tests) should pass.

- [ ] **Step 5: Commit**

```bash
git add iotkit-polling-adapter-runtime/src/lib.rs iotkit-polling-adapter-runtime/src/polling_loop.rs rpi-local-adapter/src/drivers/mcp9600.rs rpi-local-adapter/src/drivers/opt3001.rs
git commit -m "refactor(polling-adapter-runtime): replace probe() with detect() + init() across trait, drivers, and state machine"
```

---

### Task 2: Update all existing tests

**Depends on:** Task 1

**Files:**
- Modify: `iotkit-polling-adapter-runtime/src/polling_loop.rs`

- [ ] **Step 1: Update all existing tests**

The following tests and test infrastructure in `polling_loop.rs` must be updated:

**Test drivers:**
- `MockDriver`: rename `probe_results` field to `detect_results`, add `init_results: Mutex<VecDeque<Result<(), String>>>`. Update `SensorDriver` impl: `probe()` → `detect()` + `init()`.
- `StubDriver` (in polling_loop tests): `probe()` → `detect()` + `init()`. `detect()` returns `make_identity()`, `init()` returns `Ok(())`.
- `PanickingDriver`: rename `panic_on_probe` to `panic_on_detect`, add `panic_on_init`. Update `SensorDriver` impl: `probe()` → `detect()` + `init()`.
- `make_mock_target` and `make_sensor_target`: add `init_results` parameter.

**Test renames and updates (15 tests affected):**
1. `pending_target_probe_success` → `pending_target_detect_init_success`: now expects `Discovered` from detect+init.
2. `pending_target_probe_failure` → `pending_target_detect_failure`: `ProbeFailed` → `DetectFailed`.
3. `probe_success_transitions_to_active_and_emits_discovered` → `detect_init_success_transitions_to_active`: unchanged logic.
4. `probe_failure_below_threshold_emits_no_event` → `detect_failure_below_threshold_emits_no_event`: `ProbeFailed` → `DetectFailed`, `consecutive_probe_failures` → `consecutive_detect_failures`.
5. `probe_failure_at_threshold_emits_one_error` → `detect_failure_at_threshold_emits_one_error`: `ProbeFailed` → `DetectFailed`, `MAX_PROBE_FAILURES` → `MAX_DETECT_FAILURES`, `consecutive_probe_failures` → `consecutive_detect_failures`. Update expected error message: "target 0x40 probe failed" → update to match new `apply_outcomes` format.
6. `probe_success_after_escalation_resets_counter_and_flag` → `detect_init_success_after_escalation_resets`: unchanged logic (Discovered still transitions to Active).
7. `probe_panic_becomes_probe_failed` → `detect_panic_becomes_detect_failed`: `panic_on_probe` → `panic_on_detect`, `ProbeFailed` → `DetectFailed`.
8. `read_failures_at_threshold_emits_lost_and_transitions_to_pending`: update `consecutive_probe_failures` → `consecutive_detect_failures` in Pending state assertion.
9. `multiple_targets_independent`: `ProbeFailed` → `DetectFailed`, `consecutive_probe_failures` → `consecutive_detect_failures`.
10. `mock_probe_discovery_then_read`: update to provide both `detect_results` and `init_results` to MockDriver.
11. `all_targets_fail_startup_emits_immediate_error`: detect failure still triggers same error message.

12. `read_panic_becomes_read_error`: update `PanickingDriver` constructor (`panic_on_probe` → `panic_on_detect`, add `panic_on_init: false`). Logic unchanged.
13. `panic_in_one_target_does_not_affect_sibling`: update `PanickingDriver` constructor, `ProbeFailed` assertion → `DetectFailed`. Second target now goes through detect+init, so outcome changes from `Discovered` (probe) to `Discovered` (detect+init) — verify the outcome is still `Discovered`.
14. `probe_panic_emits_immediate_adapter_error` → `detect_panic_emits_immediate_adapter_error`: `ProbeFailed` → `DetectFailed`. **IMPORTANT:** The plan's `apply_outcomes` for `DetectFailed` must preserve immediate emit on panic — see semantic note below.
15. `panic_does_not_consume_escalation_emitted` → `detect_panic_does_not_consume_escalation`: `ProbeFailed` → `DetectFailed`, `MAX_PROBE_FAILURES` → `MAX_DETECT_FAILURES`. Same behavior preservation needed as test 14.

**Tests unchanged (no probe references):**
- `shutdown_command_stops_loop`
- `command_channel_drop_stops_loop`
- `event_channel_close_detected_with_events`
- `event_channel_close_detected_without_events`
- `device_command_rejection_preserves_device_key`
- `empty_targets_no_startup_error`
- `read_success_emits_sensor_data_and_resets_counter`
- `read_failure_emits_error_and_increments_counter`
- `active_target_read_success`
- `active_target_read_failure`
- `discovered_only_no_same_cycle_read`

- [ ] **Step 2: Verify existing tests pass**

Run: `cargo test -p iotkit-polling-adapter-runtime`
Expected: All existing tests pass (with renamed probe→detect references).

- [ ] **Step 3: Commit**

```bash
git add iotkit-polling-adapter-runtime/src/polling_loop.rs
git commit -m "refactor(polling-adapter-runtime): update existing tests for detect/init split"
```

---

### Task 3: Add new tests for init failure and Detected state paths

**Depends on:** Task 2

**Files:**
- Modify: `iotkit-polling-adapter-runtime/src/polling_loop.rs`

- [ ] **Step 1: Add new tests for init failure and Detected state paths**

Add tests to the `tests` module in `polling_loop.rs`. Implement a `DetectOnlyDriver` that succeeds `detect()` but fails `init()`, and use the updated `PanickingDriver` (with `panic_on_init` field) for panic tests:

```rust
#[test]
fn detect_success_init_failure_enters_detected_state() {
    // Create a DetectOnlyDriver where detect succeeds but init returns Err
    // Run poll_cycle on Pending target
    // Verify: single PollOutcome::InitFailed with identity
    // Run apply_outcomes
    // Verify: state is Detected { identity, consecutive_init_failures: 1 }
    // Verify: AdapterError event emitted with "init failed (1/5)" message
}

#[test]
fn detected_state_retries_init_and_succeeds() {
    // Start in Detected { identity, consecutive_init_failures: 1 }
    // Use MockDriver with init_results: [Ok(())]
    // Run poll_cycle
    // Verify: PollOutcome::Discovered (not DetectFailed — detect is NOT called)
    // Run apply_outcomes
    // Verify: state is Active with consecutive_read_failures: 0
    // Verify: DeviceDiscovered event emitted
}

#[test]
fn max_init_failures_returns_to_pending() {
    // Start in Detected { identity, consecutive_init_failures: MAX_INIT_FAILURES - 1 }
    // apply_outcomes with InitFailed
    // Verify: state returns to Pending { consecutive_detect_failures: 0, escalation_emitted: false }
}

#[test]
fn same_cycle_detect_init_success_emits_discovered() {
    // Pending target, MockDriver with detect: [Ok(identity)], init: [Ok(())]
    // Run poll_cycle
    // Verify: exactly 1 outcome: PollOutcome::Discovered (not two separate outcomes)
    // Verify: single DeviceDiscovered event after apply_outcomes
}

#[test]
fn init_panic_becomes_init_failed() {
    // Use PanickingDriver { panic_on_detect: false, panic_on_init: true, panic_on_read: false }
    // Start in Detected { identity, consecutive_init_failures: 0 }
    // Run poll_cycle
    // Verify: PollOutcome::InitFailed { is_panic: true, message contains "panic" }
    // Run apply_outcomes
    // Verify: state is Detected with consecutive_init_failures: 1
    // Verify: AdapterError event emitted
}

#[test]
fn init_panic_from_pending_same_cycle() {
    // Use PanickingDriver { panic_on_detect: false, panic_on_init: true, panic_on_read: false }
    // Start in Pending
    // Run poll_cycle: detect succeeds, init panics
    // Verify: PollOutcome::InitFailed { is_panic: true }
    // Run apply_outcomes
    // Verify: state is Detected (not Pending, not Active)
}

#[tokio::test]
async fn all_targets_init_fail_startup_emits_aggregate_error() {
    // Use DetectOnlyDriver (detect=Ok, init=Err) for all targets
    // Run polling_loop startup
    // Verify: per-target AdapterError for each init failure
    // Verify: aggregate "all targets failed startup probe" error emitted
    // (InitFailed counts toward the all_failed condition)
}
```

- [ ] **Step 2: Verify all tests pass**

Run: `cargo test -p iotkit-polling-adapter-runtime`
Expected: All tests pass (existing + new)

Run: `cargo test --workspace`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add iotkit-polling-adapter-runtime/src/polling_loop.rs
git commit -m "test(polling-adapter-runtime): add init failure and Detected state tests"
```

---

## Self-Review

**Spec coverage:**
- ✅ SensorDriver trait: detect() + init() + read() — Task 1 Phase A
- ✅ detect() contract (read-only, error strings) — Task 1 Phases B, C
- ✅ init() contract (idempotent, error strings) — Task 1 Phases B, C
- ✅ MCP9600 split — Task 1 Phase B
- ✅ OPT3001 split — Task 1 Phase C
- ✅ TargetState with Detected — Task 1 Phase D
- ✅ PollOutcome with InitFailed, DetectFailed, Discovered — Task 1 Phase D
- ✅ Same-cycle detect+init — Task 1 Phase D (poll_cycle)
- ✅ Counter reset rules — Task 1 Phase D (apply_outcomes)
- ✅ Init retry without re-detect — Task 1 Phase D (poll_cycle Detected branch)
- ✅ MAX_DETECT_FAILURES, MAX_INIT_FAILURES — Task 1 Phase D
- ✅ Startup behavior — Task 1 Phase E
- ✅ State snapshot Clone — Task 1 Phase E Step 2
- ✅ Panic safety (catch_unwind for all 3 methods) — Task 1 Phase D
- ✅ Detect panic immediate emit — Task 1 Phase D (apply_outcomes, is_panic branch)
- ✅ Init panic test — Task 3 Step 1 (`init_panic_becomes_init_failed`, `init_panic_from_pending_same_cycle`)
- ✅ All tests updated — Task 2 Step 1 (15 tests enumerated)
- ✅ detect() read-only verification — Task 1 Phases B, C (grep-based)
- ✅ No changes to core/types — verified (`SensorIdentity` already derives `Clone`)
- ✅ No changes to bravepi adapter — verified

**Placeholder scan:** No TBD/TODO. Task 3 Step 1 has test skeletons — Dev subagent must implement full test bodies.

**Type consistency:** `TargetState`, `PollOutcome`, `SensorDriver` trait — consistent across all tasks.

**Green-state checkpoints:**
- Task 1 end: `cargo check --workspace` passes (all production code compiles)
- Task 2 end: `cargo test -p iotkit-polling-adapter-runtime` passes (existing tests updated)
- Task 3 end: `cargo test --workspace` passes (all tests including new init-failure tests)

**Related integration tests (not modified, hardware-only):** `rpi-local-adapter/tests/integration.rs` contains `#[ignore]` tests `real_i2c_discovers_and_reads_mcp9600` and `real_i2c_discovers_and_reads_opt3001`. These verify end-to-end startup discovery on real hardware. They do not need code changes but should be run on hardware after merge: `cargo test -p rpi-local-adapter --test integration -- --ignored`.
