# SensorDriver Redesign: detect/init Split (#34)

## Goal

Split `SensorDriver::probe()` into read-only `detect()` + separate `init()` so that I2C bus scanning can safely identify devices without writing to hardware. This is a prerequisite for #35 auto-detection.

## Architecture

The `SensorDriver` trait gains two methods (`detect`, `init`) replacing `probe`. The polling loop state machine gains a `Detected` intermediate state between `Pending` and `Active`. Drivers are always fully configured — the `NeedsConfig` concept is deferred to #35 auto-detection. No changes to `core/types`, `core/engine`, or `AdapterEvent`.

## SensorDriver Trait

### Before (current)

```rust
pub trait SensorDriver: Send + Sync {
    fn probe(&self, bus_path: &str, address: u8) -> Result<SensorIdentity, String>;
    fn read(&self, bus_path: &str, address: u8) -> Result<SensorReading, String>;
    fn ic_name(&self) -> &'static str;
    fn validate(&self, poll_interval_ms: u64) -> Result<(), String> { Ok(()) }
}
```

### After

```rust
pub trait SensorDriver: Send + Sync {
    /// Read-only detection. Must NOT write to hardware.
    /// Reads device ID registers and returns identity on match.
    fn detect(&self, bus_path: &str, address: u8) -> Result<SensorIdentity, String>;

    /// Initialize hardware by writing config registers.
    /// Called only after detect() succeeds. Must be idempotent.
    fn init(&self, bus_path: &str, address: u8) -> Result<(), String>;

    /// Read sensor value. Called only after init() succeeds.
    fn read(&self, bus_path: &str, address: u8) -> Result<SensorReading, String>;

    /// Return the IC part name (e.g. "opt3001", "mcp9600").
    fn ic_name(&self) -> &'static str;

    /// Validate poll interval. Default: accept any.
    fn validate(&self, poll_interval_ms: u64) -> Result<(), String> {
        let _ = poll_interval_ms;
        Ok(())
    }
}
```

### Contract

- **`detect()`**: Read-only. Must not write any registers. Must not change hardware state. Returns `SensorIdentity` on success (device ID matches expected value). Returns `Err` if device not found, wrong ID, or I2C communication error.
- **`init()`**: Writes configuration registers. Must be idempotent — safe to call multiple times (e.g., after a panic recovery). Returns `Ok(())` when hardware is ready for `read()`. Returns `Err` on I2C communication error.
- **`read()`**: Reads sensor value. Called repeatedly in the poll loop. Unchanged from current behavior.
- **Error strings**: All `Err` return values from `detect()`, `init()`, and `read()` must include the bus path and address for field debugging, e.g. `format!("MCP9600 0x{:02x}@{}: ...", address, bus_path, ...)`. This carries forward the existing contract from the current `probe()` and `read()` doc comments.
- **Drivers are always fully configured.** The `NeedsConfig` concept (e.g., MCP9600 without thermocouple type) is handled outside the trait at the discovery/scanning layer (#35). A `SensorDriver` instance always has all config it needs to `detect`, `init`, and `read`.

### Enforcement

The `detect()` no-write contract is enforced by documentation and code review, not by the type system. A `ReadOnlyTransport` wrapper is not needed at this project scale (~2 drivers, single team). If the driver set grows beyond ~5 drivers, consider a read-only transport wrapper as a hardening step.

**Testability**: Driver tests should verify `detect()` does not call `write_register`. Since both MCP9600 and OPT3001 open `I2cTransport` internally, tests on real hardware can verify by checking that the device's config registers are unchanged after `detect()`. Unit tests without hardware can verify by inspection (the detect() method body contains no `write_register` calls).

### Error Semantics

`detect()` returns `Err(String)` for both "device not found / wrong ID" and "I2C transport error". This conflation is acceptable for #34's scope (configured targets with known addresses). For #35 auto-detection (scanning unknown addresses), distinguishing "no device" from "transport fault" becomes important. #35 should either refine the error type (e.g., `Result<Option<SensorIdentity>, TransportError>`) or add a separate scanning interface. This spec does not pre-optimize for that.

### Relationship to #35 Auto-Detection

`detect()` requires a fully constructed `SensorDriver` instance — you must know the IC type and have the driver configured to call `detect()`. This means `detect()` alone is not a general-purpose "scan an unknown address" primitive. For #35, the scanning layer must iterate known driver types per address (e.g., try `Mcp9600Driver::detect()`, then `Opt3001Driver::detect()`) or use a separate address-level probing mechanism. This is a #35 design concern, not a #34 limitation.

## Driver Changes

### MCP9600

**detect()**: Read device ID register (`REG_DEVICE_ID`). Verify `id_buf[0] == mcp9600::DEVICE_ID`. Return `SensorIdentity`. No register writes.

**init()**: Write thermocouple type to `REG_SENSOR_CONFIGURATION`. The `thermocouple_type` field remains `ThermocoupleType` (not `Option`) — the driver is always constructed with a concrete type.

**read()**: Unchanged.

### OPT3001

**detect()**: Read device ID register (`REG_DEVICE_ID`). Verify `device_id == opt3001::DEVICE_ID`. Return `SensorIdentity`. No register writes.

**init()**: Write `INIT_CONFIG` to `REG_CONFIG`.

**read()**: Unchanged.

## Polling Loop State Machine

### Before

```
Pending → probe() success → Active (DeviceDiscovered) → read() loop
```

### After

```
Pending → detect()+init() success (same cycle) → Active (DeviceDiscovered) → read() loop
Pending → detect() success + init() failure → Detected → init() retry → Active (DeviceDiscovered) → read() loop
```

### TargetState

```rust
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
```

### Transitions

| From | Event | To | Side Effect |
|------|-------|----|-------------|
| Pending | detect() success + init() success (same cycle) | Active | Emit `DeviceDiscovered` |
| Pending | detect() success + init() failure (same cycle) | Detected | Emit `AdapterError` (see below) |
| Pending | detect() failure | Pending | Increment `consecutive_detect_failures` |
| Detected | init() success | Active | Emit `DeviceDiscovered` |
| Detected | init() failure | Detected | Increment `consecutive_init_failures`, emit `AdapterError` (see below) |
| Active | read() success | Active | Emit `SensorData`, reset `consecutive_read_failures` |
| Active | read() failure (< MAX) | Active | Increment `consecutive_read_failures`, emit `AdapterError` |
| Active | read() failure (>= MAX) | Pending | Emit `DeviceLost` |

Note: `poll_cycle()` performs same-cycle detect+init for `Pending` targets (see "Same-Cycle Detect+Init" above). A `Pending` target that detects successfully is immediately init'd in the same cycle. The `Detected` state is only entered when init fails after a successful detect.

### Failure Thresholds

- `MAX_DETECT_FAILURES`: 10 (same as current `MAX_PROBE_FAILURES`)
- `MAX_INIT_FAILURES`: 5 (new — init is expected to succeed after detect)
- `MAX_READ_FAILURES`: 5 (unchanged)

After `MAX_DETECT_FAILURES` consecutive detect failures with escalation not yet emitted, emit `AdapterError` escalation (same as current probe escalation behavior).

After `MAX_INIT_FAILURES` consecutive init failures, transition back to `Pending` and re-detect (device may have been disconnected or hot-swapped). On each init failure (including panics), emit `AdapterError` with `device_key: None` (the device has not yet been discovered so there is no `DeviceKey`). The error message must include the target address, bus path, and failure count for field debugging.

### Counter Reset Rules

Counters are reset on state transitions to prevent stale values:

| Transition | Counter Resets |
|------------|---------------|
| Pending → Detected | `consecutive_detect_failures` → 0, `escalation_emitted` → false |
| Detected → Active | `consecutive_init_failures` → 0 |
| Active → Pending (DeviceLost) | All counters reset (fresh `Pending` state) |
| Detected → Pending (MAX_INIT_FAILURES) | All counters reset (fresh `Pending` state) |

### Init Retry Without Re-Detect

When a target is in `Detected` state (init failed), subsequent ticks retry `init()` directly without re-running `detect()`. This is acceptable because:
- The time between retries is one poll interval (typically 1s) — unlikely for hardware to be hot-swapped
- `init()` writes are idempotent — writing config to a different device is detectable by subsequent `read()` failures, which lead back to `Pending` via `DeviceLost`
- Re-detecting on every init retry would double I2C bus traffic for no practical benefit

After `MAX_INIT_FAILURES`, the target returns to `Pending` where `detect()` runs again, providing a fresh identity check.

### PollOutcome

```rust
pub(crate) enum PollOutcome {
    /// detect() succeeded, init() succeeded (same cycle or from Detected state).
    Discovered {
        target_index: usize,
        key: DeviceKey,
        identity: SensorIdentity,
    },
    /// detect() succeeded but init() failed (same cycle or from Detected state).
    /// Includes identity so apply_outcomes can enter/maintain Detected state.
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

### Same-Cycle Detect+Init

To avoid introducing extra-tick latency (both at startup and during steady-state recovery from `DeviceLost`), `poll_cycle()` must advance a target through **both** `detect()` and `init()` in a single call when both succeed. Specifically: when a target in `Pending` state passes `detect()`, `poll_cycle()` immediately calls `init()` on that target within the same cycle. If `init()` also succeeds, the outcome is `Discovered` (not a separate `Detected` outcome followed by `Discovered` on the next tick). If `init()` fails, the outcome is `InitFailed` (which carries the identity so `apply_outcomes` can enter `Detected` state), and the target retries `init()` on subsequent ticks.

This preserves the current behavior where a successful `probe()` yields `DeviceDiscovered` in the same cycle, for both startup and steady-state recovery paths.

### Startup Behavior

The startup probe runs one `poll_cycle()` before entering the ticker loop, same as today. Because `poll_cycle()` now does same-cycle detect+init (see above), targets that both detect and initialize successfully will emit `DeviceDiscovered` before the ticker starts, matching current behavior.

The existing startup all-targets-failed `AdapterError` is preserved: if all targets produce `DetectFailed` or `InitFailed` outcomes on the startup cycle, emit the same "all targets failed startup probe" error with the bus path and address list.

### State Snapshot

The current state-snapshot clone pattern (manual `match` to copy `TargetState` fields) is duplicated for startup and main-loop `poll_cycle()` calls. With the addition of `Detected`, derive `Clone` on `TargetState` to eliminate this duplication and prevent drift.

### Panic Safety

All three methods (`detect`, `init`, `read`) are wrapped in `catch_unwind` in `poll_cycle()`. After a panic:
- `detect()` panic → stays in `Pending`, increments failure counter (same as current probe panic)
- `init()` panic → stays in `Detected`, increments failure counter. Hardware may be partially configured; next `init()` call overwrites (idempotent contract).
- `read()` panic → stays in `Active`, increments failure counter (same as current)

## Changes NOT Made

- **`core/types`**: `AdapterEvent::DeviceDiscovered` is unchanged. No new event types.
- **`core/engine`**: No changes. State handling is unaffected.
- **`bravepi-mainboard-adapter`**: Does not implement `SensorDriver`. Unaffected.
- **`iotkit-gateway/src/main.rs`**: No changes.
- **`rpi-local-adapter/src/lib.rs`**: No changes to `RpiLocalConfig`, `RpiLocalTarget`, `start()`, or `current_hardcoded_targets()`. The adapter constructs drivers the same way; only the driver internals change.
- **`NeedsConfig` / `Readiness` / `DriverCapability`**: Deferred to #35 auto-detection. This spec only splits probe() into detect()+init().

## Blast Radius

| File | Change |
|------|--------|
| `iotkit-polling-adapter-runtime/src/lib.rs` | Trait: `probe()` → `detect()` + `init()`. Test drivers updated. |
| `iotkit-polling-adapter-runtime/src/polling_loop.rs` | State machine: `Detected` state added. `poll_cycle()` updated for 3-phase flow. `PollOutcome` gains `InitFailed` variant, keeps `Discovered` (renamed from probe-based semantics). `ProbeFailed` renamed to `DetectFailed`. `catch_unwind` added for `init()`. |
| `rpi-local-adapter/src/drivers/mcp9600.rs` | `probe()` split into `detect()` + `init()`. |
| `rpi-local-adapter/src/drivers/opt3001.rs` | `probe()` split into `detect()` + `init()`. |

**4 files total.** No changes outside polling-adapter-runtime and rpi-local-adapter.

## Testing Strategy

### Unit tests in `iotkit-polling-adapter-runtime`

- **Trait compliance**: StubDriver implements `detect()`, `init()`, `read()` correctly.
- **State machine transitions**: Test all 7 transitions in the state table above.
- **Detect failure → retry**: Verify `consecutive_detect_failures` increments and escalation emits.
- **Init failure → retry**: Verify `consecutive_init_failures` increments. Verify transition back to `Pending` after `MAX_INIT_FAILURES`.
- **Same-cycle detect+init**: Verify that a `Pending` target with successful `detect()` and `init()` transitions to `Active` and emits `DeviceDiscovered` in a single `poll_cycle()` call — no extra tick delay.
- **Same-cycle detect success + init failure**: Verify that a `Pending` target with successful `detect()` but failed `init()` transitions to `Detected` and emits `AdapterError` in a single `poll_cycle()` call.
- **Init after detect (from Detected state)**: Verify `Detected` → `Active` transition emits `DeviceDiscovered`.
- **Panic recovery**: Verify `catch_unwind` handles panics in all three methods.
- **Existing tests**: All current `validate_config` tests remain unchanged. `probe` references in test drivers updated to `detect` + `init`.

### Unit tests in `rpi-local-adapter`

- **MCP9600 detect()**: Verify no register writes (would require mock transport or careful test design).
- **OPT3001 detect()**: Same — verify read-only.
- **Existing tests**: `start_without_runtime_returns_error`, `start_with_invalid_config_returns_config_error`, `opt3001_rejects_short_poll_interval` — unchanged (they test config validation, not probe/detect).

## Dependencies

- No new crate dependencies.
- This spec blocks #35 (auto-detection) which will use `detect()` for safe bus scanning.
