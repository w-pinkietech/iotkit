# Phase 1 Hardening: Adapter-Core Boundary Improvements

Date: 2026-03-26
Status: Approved

## Background

Phase 1 PoC (channel-based adapter-core boundary) is working on real hardware.
Two rounds of code review identified remaining issues to fix before Phase 2.
This spec covers the 4 items selected for immediate action.

## Scope

4 fixes (approach A selected for all):

1. Serial error auto-retry inside adapter
2. event_loop async integration tests
3. DeviceDiscovered event emission
4. JoinHandle retention in AdapterHandle

Out of scope (deferred to Phase 2):
- DifferentialPressure / Acceleration test coverage
- Codec buffer drain on error
- rssi type unification (i8 vs i16)

## Fix 1: Serial Error Auto-Retry

### Location
`bravepi-adapter/src/task.rs` — `serial_reader_thread()`

### Design
Add retry loop inside the reader thread. On fatal serial read error:

1. Close the failed transport (drop)
2. Increment `retry_count`
3. If `retry_count > MAX_RETRIES (10)`: send `Err(msg)` to bytes channel, return
4. Sleep with exponential backoff: `min(2^retry_count, 30)` seconds
5. Check `bytes_tx.is_closed()` before retrying (respond to Shutdown)
6. Attempt to reopen transport with same port_path and serial_config()
7. On success: reset `retry_count = 0`, continue read loop
8. On failure: go to step 3 (retry_count already incremented)

### Rationale
- Core should not know about serial reconnection (adapter internal concern)
- Reader thread already owns the transport, so retry fits naturally here
- MAX_RETRIES and backoff are hardcoded for PoC; configurable in Phase 2

## Fix 2: event_loop Async Integration Tests

### Location
`bravepi-adapter/src/task.rs` — make `event_loop` pub
`bravepi-adapter/tests/event_loop_test.rs` — new file

### Design
Make `event_loop` pub so tests can inject channels directly.

4 test scenarios:
1. **Shutdown command**: send `AdapterCommand::Shutdown` via command_tx, verify event_loop exits (event_rx closes)
2. **Bytes channel Err**: send `Err("test error")` via bytes_tx, verify `AdapterEvent::AdapterError` arrives on event_rx with the error message
3. **Bytes channel close**: drop bytes_tx, verify `AdapterEvent::AdapterError` arrives with "exited unexpectedly" message
4. **Normal data flow**: send raw frame bytes (reuse codec test data) via bytes_tx, verify `AdapterEvent::SensorData` arrives on event_rx with correct values

### Test structure
```rust
#[tokio::test]
async fn test_scenario() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);

    // Note: event_loop signature includes port_path for DeviceDiscovered (Fix 3)
    let handle = tokio::spawn(event_loop("test".into(), bytes_rx, event_tx, command_rx));

    // ... test actions ...
    // For data flow test, first event will be DeviceDiscovered, then SensorData

    // verify event_loop has exited
    handle.await.unwrap();
}
```

## Fix 3: DeviceDiscovered Event Emission

### Location
`bravepi-adapter/src/task.rs` — `event_loop()` and `frame_to_event()`

### Design

**frame_to_event signature change:**
```rust
pub fn frame_to_event(frame: BravePiFrame) -> Option<(AdapterEvent, Option<SensorIdentity>)>
```

Returns a tuple: the event itself + optionally a SensorIdentity (for SensorData frames only).
For DecodeError/Config frames, identity is None.

**event_loop state:**
```rust
let mut seen_devices: HashSet<DeviceKey> = HashSet::new();
```

**Logic:**
When frame_to_event returns `Some((event, Some(identity)))`:
1. Extract device_key from the event
2. If `seen_devices.insert(device_key.clone())` returns true (first time):
   - Send `AdapterEvent::DeviceDiscovered { device_key, identity }` first
3. Send the original SensorData event

**frame_to_event remains a pure function** — no state. State lives in event_loop.

### Identity construction
Each sensor module already has an `identity(ConnectionInfo)` function.
`frame_to_event` will construct identity using:
- `sensor_type_from_bravepi_raw()` to determine sensor type
- The corresponding sensor module's `identity()` with a placeholder `ConnectionInfo` (UART, port from adapter context)

Since `frame_to_event` needs the port_path for ConnectionInfo, it will take an additional parameter:
```rust
pub fn frame_to_event(frame: BravePiFrame, port_path: &str) -> Option<(AdapterEvent, Option<SensorIdentity>)>
```

### Test impact
Existing `frame_to_event` tests need updating for the new return type.
New tests for DeviceDiscovered in event_loop integration tests.

## Fix 4: JoinHandle Retention

### Location
`bravepi-adapter/src/task.rs` — `AdapterHandle` and `start()`

### Design

**AdapterHandle change:**
```rust
pub struct AdapterHandle {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub command_tx: mpsc::Sender<AdapterCommand>,
    reader_thread: Option<std::thread::JoinHandle<()>>,
}
```

**New method:**
```rust
impl AdapterHandle {
    /// Send Shutdown command and wait for the reader thread to exit.
    pub async fn shutdown(mut self) -> Result<(), String> {
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;
        if let Some(handle) = self.reader_thread.take() {
            handle.join().map_err(|_| "Reader thread panicked".to_string())?;
        }
        Ok(())
    }
}
```

- No `Drop` implementation (blocking join in Drop is dangerous)
- Explicit `shutdown()` is the recommended way to stop the adapter
- `reader_thread` is private — only accessible via `shutdown()`

## Dependency Changes

None. All fixes use existing dependencies (tokio, std::thread, std::collections::HashSet).

## Test Summary

| Test file | New tests |
|-----------|-----------|
| `tests/frame_to_event_test.rs` | Update 12 existing tests for new return type |
| `tests/event_loop_test.rs` | 4 new async tests (shutdown, error, close, data flow) |
