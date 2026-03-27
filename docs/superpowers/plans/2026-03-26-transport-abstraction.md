# Sub-project A: Transport Abstraction — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Separate port-open / reader-thread / reconnect responsibilities from the adapter's event loop, making the channel boundary the explicit adapter input contract.

**Architecture:** Extract `serial_source` module that owns SerialTransport + reader thread + reconnect, returning a `BytesReceiver` channel. `event_loop` consumes `BytesReceiver` (typed with `TransportError` instead of bare `String`). `handle.rs` becomes thin wiring. Visibility narrows to `pub(crate)` for internal modules; integration tests move to crate-internal unit tests.

**Tech Stack:** Rust 2024 edition, tokio (sync/rt/macros), rpi4b-transport, bravepi-codec, tracing

**Spec:** `docs/superpowers/specs/2026-03-26-transport-abstraction-design.md`

---

## File Structure

| Action | Path | Responsibility |
|--------|------|---------------|
| Create | `bravepi-adapter/src/transport.rs` | `TransportError` struct, `BytesReceiver` type alias |
| Create | `bravepi-adapter/src/task/serial_source.rs` | `SerialSource`, `SerialSourceHandle`, `start()`, `serial_reader_thread()` |
| Create | `bravepi-adapter/src/task/event_loop_test.rs` | event_loop crate-internal unit tests (moved from `tests/`) |
| Create | `bravepi-adapter/src/task/convert_test.rs` | frame_to_event crate-internal unit tests (moved from `tests/`) |
| Modify | `bravepi-adapter/src/lib.rs` | Add `pub(crate) mod transport;` |
| Modify | `bravepi-adapter/src/task/mod.rs` | Replace `mod reader` with `mod serial_source`, add test modules, narrow re-exports |
| Modify | `bravepi-adapter/src/task/event_loop.rs` | `pub` → `pub(crate)`, `Receiver<Result<Vec<u8>, String>>` → `BytesReceiver` |
| Modify | `bravepi-adapter/src/task/convert.rs` | `pub fn` → `pub(crate) fn` |
| Modify | `bravepi-adapter/src/task/handle.rs` | Use `serial_source::start()`, new `AdapterHandle` fields, new shutdown sequence |
| Delete | `bravepi-adapter/src/task/reader.rs` | Content moved to `serial_source.rs` |
| Delete | `bravepi-adapter/tests/event_loop_test.rs` | Moved to crate-internal |
| Delete | `bravepi-adapter/tests/frame_to_event_test.rs` | Moved to crate-internal |

---

### Task 1: Create TransportError type and BytesReceiver alias

**Files:**
- Create: `bravepi-adapter/src/transport.rs`
- Modify: `bravepi-adapter/src/lib.rs`

- [ ] **Step 1: Create `transport.rs` with type definitions**

```rust
// bravepi-adapter/src/transport.rs

//! Transport 層の型定義。adapter crate 内部でのみ使用する。

use std::fmt;
use tokio::sync::mpsc;

/// Transport source が回復不能な障害で停止した理由。
#[derive(Debug, Clone)]
pub(crate) struct TransportError {
    pub message: String,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// event_loop が受け取る byte stream の型。
pub(crate) type BytesReceiver = mpsc::Receiver<Result<Vec<u8>, TransportError>>;
```

- [ ] **Step 2: Wire `transport` module into `lib.rs`**

In `bravepi-adapter/src/lib.rs`, add `pub(crate) mod transport;` after the existing `pub mod task;` line:

```rust
pub mod task;
pub(crate) mod transport;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p bravepi-adapter`
Expected: compiles with no errors (transport.rs is unused but that's OK — it will be used in subsequent tasks)

- [ ] **Step 4: Commit**

```bash
git add bravepi-adapter/src/transport.rs bravepi-adapter/src/lib.rs
git commit -m "feat(bravepi): add TransportError type and BytesReceiver alias"
```

---

### Task 2: Migrate event_loop to pub(crate) and use TransportError

**Files:**
- Modify: `bravepi-adapter/src/task/event_loop.rs`
- Modify: `bravepi-adapter/src/task/mod.rs`
- Create: `bravepi-adapter/src/task/event_loop_test.rs` (moved from `bravepi-adapter/tests/event_loop_test.rs`)
- Delete: `bravepi-adapter/tests/event_loop_test.rs`

- [ ] **Step 1: Update `event_loop.rs` — visibility and type**

Change the function signature from `pub async fn` to `pub(crate) async fn`, and replace `Result<Vec<u8>, String>` with `BytesReceiver`:

Replace the imports and signature in `bravepi-adapter/src/task/event_loop.rs`:

```rust
//! async task: raw bytes → codec → AdapterEvent。
//! デバイスのライフサイクル追跡もここで行う。

use std::collections::HashSet;

use bravepi_codec::BravePiCodec;
use iotkit_core_types::{AdapterCommand, AdapterEvent, DeviceKey};
use tokio::sync::mpsc;

use crate::transport::BytesReceiver;
use super::convert::frame_to_event;

pub(crate) async fn event_loop(
    port_path: String,
    mut bytes_rx: BytesReceiver,
    event_tx: mpsc::Sender<AdapterEvent>,
    mut command_rx: mpsc::Receiver<AdapterCommand>,
) {
```

The body remains the same except the `Some(Err(error))` arm. Change:

```rust
                    Some(Err(error)) => {
                        tracing::error!(%error, "Serial reader reported error");
                        let _ = event_tx.send(AdapterEvent::AdapterError {
                            device_key: None,
                            error: error.to_string(),
                        }).await;
                        return;
                    }
```

The only change is `error` → `error.to_string()` on the `AdapterEvent::AdapterError` line, because `error` is now `TransportError` and the `AdapterError` field expects `String`.

- [ ] **Step 2: Update `mod.rs` — remove public re-export of `event_loop`, add test module**

Replace the full content of `bravepi-adapter/src/task/mod.rs`:

```rust
//! BravePI adapter async task。
//!
//! シリアルポートからフレームを読み、AdapterEvent に変換して channel に送信する。
//! blocking serial I/O は専用スレッドで実行し、async 側と bytes channel で接続する。

mod convert;
pub(crate) mod event_loop;
mod handle;
mod reader;

pub use convert::frame_to_event;
pub use handle::{start, AdapterHandle};

#[cfg(test)]
mod event_loop_test;
```

Note: `event_loop` changes from `mod event_loop;` (private) + `pub use event_loop::event_loop;` (re-export) to `pub(crate) mod event_loop;` (no re-export). `convert::frame_to_event` stays `pub use` for now (Task 3 changes it).

- [ ] **Step 3: Create `event_loop_test.rs` as crate-internal test**

Create `bravepi-adapter/src/task/event_loop_test.rs` with the following content. Key changes from the old integration test:
- No `use bravepi_adapter::task::event_loop;` — use `super::event_loop::event_loop;` instead
- `Err(String)` → `Err(TransportError { message: ... })`

```rust
use iotkit_core_types::{AdapterCommand, AdapterEvent, SensorType};
use tokio::sync::mpsc;

use crate::transport::TransportError;
use super::event_loop::event_loop;

/// Build raw frame bytes for the BravePI codec.
/// Format: [payload_len:u16 LE][device_number:u64 LE][sensor_type:u16 LE][rssi:i8][flag:u8][payload...]
fn build_sensor_frame_bytes(device_number: u64, sensor_type: u16, rssi: i8, battery: u8, count: u16, values: &[u8]) -> Vec<u8> {
    let mut payload = vec![battery];
    payload.extend_from_slice(&count.to_le_bytes());
    payload.extend_from_slice(values);

    let payload_len = payload.len() as u16;
    let mut frame = Vec::new();
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&device_number.to_le_bytes());
    frame.extend_from_slice(&sensor_type.to_le_bytes());
    frame.push(rssi as u8);
    frame.push(0x00); // flag = no continuation
    frame.extend_from_slice(&payload);
    frame
}

#[tokio::test]
async fn shutdown_command_exits_event_loop() {
    let (_bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);

    let handle = tokio::spawn(event_loop("test".into(), bytes_rx, event_tx, command_rx));

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();

    // event_loop exits → event_tx dropped → recv returns None
    assert!(event_rx.recv().await.is_none());
}

#[tokio::test]
async fn bytes_channel_error_produces_adapter_error() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(16);

    let handle = tokio::spawn(event_loop("test".into(), bytes_rx, event_tx, command_rx));

    bytes_tx.send(Err(TransportError { message: "serial port disconnected".to_string() })).await.unwrap();
    handle.await.unwrap();

    match event_rx.recv().await.expect("should receive event") {
        AdapterEvent::AdapterError { device_key, error } => {
            assert!(device_key.is_none());
            assert!(error.contains("serial port disconnected"));
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }
}

#[tokio::test]
async fn bytes_channel_close_produces_adapter_error() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(16);

    let handle = tokio::spawn(event_loop("test".into(), bytes_rx, event_tx, command_rx));

    drop(bytes_tx); // simulate reader thread death
    handle.await.unwrap();

    match event_rx.recv().await.expect("should receive event") {
        AdapterEvent::AdapterError { error, .. } => {
            assert!(error.contains("exited unexpectedly"));
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }
}

#[tokio::test]
async fn normal_data_flow_produces_device_discovered_then_sensor_data() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);

    let handle = tokio::spawn(event_loop("/dev/test".into(), bytes_rx, event_tx, command_rx));

    // Temperature frame (sensor_type=261), mcp9600 uart payload: Float32LE
    // 22.4375°C = 0x41b38000 in big-endian, so LE bytes are [0x00, 0x80, 0xb3, 0x41]
    let device: u64 = 0x246880020140018b;
    let frame_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();

    // First event: DeviceDiscovered (first time seeing this device)
    match event_rx.recv().await.expect("should receive DeviceDiscovered") {
        AdapterEvent::DeviceDiscovered { device_key, identity } => {
            assert_eq!(device_key.as_str(), "246880020140018b");
            assert_eq!(identity.manufacturer, "Braveridge");
            assert_eq!(identity.ic_part_number, "MCP9600");
        }
        other => panic!("expected DeviceDiscovered, got {:?}", other),
    }

    // Second event: SensorData
    match event_rx.recv().await.expect("should receive SensorData") {
        AdapterEvent::SensorData { device_key, reading, .. } => {
            assert_eq!(device_key.as_str(), "246880020140018b");
            assert_eq!(reading.sensor_type, SensorType::Temperature);
            assert!((reading.values[0] - 22.4375).abs() < 0.01);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }

    // Same device again → only SensorData (no DeviceDiscovered)
    let frame_bytes2 = build_sensor_frame_bytes(device, 261, -55, 90, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(frame_bytes2)).await.unwrap();

    match event_rx.recv().await.expect("should receive SensorData") {
        AdapterEvent::SensorData { .. } => {} // OK - no DeviceDiscovered before this
        other => panic!("expected SensorData (no DeviceDiscovered), got {:?}", other),
    }

    // Clean shutdown
    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}
```

- [ ] **Step 4: Delete old integration test**

```bash
rm bravepi-adapter/tests/event_loop_test.rs
```

- [ ] **Step 5: Run tests to verify**

Run: `cargo test -p bravepi-adapter`
Expected: All 4 event_loop tests pass from the new location. The `frame_to_event_test.rs` integration tests also still pass (not yet moved).

- [ ] **Step 6: Commit**

```bash
git add bravepi-adapter/src/task/event_loop.rs bravepi-adapter/src/task/mod.rs bravepi-adapter/src/task/event_loop_test.rs
git rm bravepi-adapter/tests/event_loop_test.rs
git commit -m "refactor(bravepi): migrate event_loop to pub(crate), use TransportError, move tests to crate-internal"
```

---

### Task 3: Migrate frame_to_event to pub(crate) and move tests

**Files:**
- Modify: `bravepi-adapter/src/task/convert.rs`
- Modify: `bravepi-adapter/src/task/mod.rs`
- Create: `bravepi-adapter/src/task/convert_test.rs` (moved from `bravepi-adapter/tests/frame_to_event_test.rs`)
- Delete: `bravepi-adapter/tests/frame_to_event_test.rs`

- [ ] **Step 1: Change `convert.rs` visibility**

In `bravepi-adapter/src/task/convert.rs`, change line 15:

From:
```rust
pub fn frame_to_event(
```
To:
```rust
pub(crate) fn frame_to_event(
```

- [ ] **Step 2: Update `mod.rs` — remove public re-export, add test module**

Replace the full content of `bravepi-adapter/src/task/mod.rs`:

```rust
//! BravePI adapter async task。
//!
//! シリアルポートからフレームを読み、AdapterEvent に変換して channel に送信する。
//! blocking serial I/O は専用スレッドで実行し、async 側と bytes channel で接続する。

mod convert;
pub(crate) mod event_loop;
mod handle;
mod reader;

pub use handle::{start, AdapterHandle};

#[cfg(test)]
mod event_loop_test;
#[cfg(test)]
mod convert_test;
```

Note: `pub use convert::frame_to_event;` is removed entirely. `frame_to_event` is now only accessible within the crate via `super::convert::frame_to_event` (used by `event_loop.rs`).

- [ ] **Step 3: Create `convert_test.rs` as crate-internal test**

Create `bravepi-adapter/src/task/convert_test.rs`. Key changes from the old integration test:
- `use bravepi_adapter::task::frame_to_event;` → `use super::convert::frame_to_event;`
- `use iotkit_core_types::...` stays the same (crate dependency)

```rust
use bravepi_codec::{BravePiFrame, ConfigFrame, SensorFrame};
use iotkit_core_types::{AdapterEvent, SensorType};

use super::convert::frame_to_event;

// ── ヘルパー ──────────────────────────────────────

fn make_sensor_frame(sensor_type_raw: u16, value_data: Vec<u8>) -> SensorFrame {
    SensorFrame {
        device_number: "246880020140018b".to_string(),
        sensor_type_raw,
        rssi: -60,
        battery: 95,
        data_count: 1,
        value_data,
    }
}

// ── Temperature (261, mcp9600) ──────────────────

#[test]
fn temperature_frame_produces_sensor_data() {
    let frame = BravePiFrame::Sensor(make_sensor_frame(261, vec![0x00, 0x80, 0xb3, 0x41]));
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData {
            device_key,
            reading,
            rssi,
            battery_pct,
        } => {
            assert_eq!(device_key.as_str(), "246880020140018b");
            assert_eq!(reading.sensor_type, SensorType::Temperature);
            assert_eq!(reading.values.len(), 1);
            assert!((reading.values[0] - 22.4375).abs() < 0.01);
            assert_eq!(rssi, Some(-60));
            assert_eq!(battery_pct, Some(95));
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── Illuminance (264, opt3001) ──────────────────

#[test]
fn illuminance_frame_produces_sensor_data() {
    let lux_bytes = 500.0f32.to_le_bytes().to_vec();
    let frame = BravePiFrame::Sensor(make_sensor_frame(264, lux_bytes));
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData { reading, .. } => {
            assert_eq!(reading.sensor_type, SensorType::Illuminance);
            assert_eq!(reading.values.len(), 1);
            assert!((reading.values[0] - 500.0).abs() < 0.1);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── ContactInput (257) ──────────────────────────

#[test]
fn contact_input_frame_maps_bytes_to_float() {
    let frame = BravePiFrame::Sensor(SensorFrame {
        device_number: "aabbccdd00112233".to_string(),
        sensor_type_raw: 257,
        rssi: -50,
        battery: 80,
        data_count: 3,
        value_data: vec![0x01, 0x00, 0x01, 0xff],
    });
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData { reading, .. } => {
            assert_eq!(reading.sensor_type, SensorType::ContactInput);
            assert_eq!(reading.values, vec![1.0, 0.0, 1.0]);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── ContactOutput (258) ─────────────────────────

#[test]
fn contact_output_frame_produces_sensor_data() {
    let frame = BravePiFrame::Sensor(SensorFrame {
        device_number: "1234567890abcdef".to_string(),
        sensor_type_raw: 258,
        rssi: -70,
        battery: 100,
        data_count: 2,
        value_data: vec![0x00, 0x01],
    });
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData { reading, .. } => {
            assert_eq!(reading.sensor_type, SensorType::ContactOutput);
            assert_eq!(reading.values, vec![0.0, 1.0]);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── Unknown sensor type → None ──────────────────

#[test]
fn unknown_sensor_type_returns_none() {
    let frame = BravePiFrame::Sensor(make_sensor_frame(9999, vec![0x01, 0x02]));
    assert!(frame_to_event(frame, "/dev/test").is_none());
}

// ── ConfigFrame → None (PoC) ────────────────────

#[test]
fn config_frame_returns_none() {
    let frame = BravePiFrame::Config(ConfigFrame {
        device_number: "246880020140018b".to_string(),
        rssi: -55,
        true_sensor_type: 261,
        firmware_version: "1.2.3".to_string(),
        timezone: 9,
        ble_mode: 1,
        tx_power: 4,
        advertise_interval: 1000,
        uplink_interval: 60,
    });
    assert!(frame_to_event(frame, "/dev/test").is_none());
}

// ── DecodeError → AdapterError ──────────────────

#[test]
fn decode_error_produces_adapter_error() {
    let frame = BravePiFrame::DecodeError {
        device_number: "bad_device".to_string(),
        sensor_type_raw: 261,
        reason: "payload too short".to_string(),
    };
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::AdapterError { device_key, error } => {
            assert_eq!(device_key.unwrap().as_str(), "bad_device");
            assert!(error.contains("Decode error"));
            assert!(error.contains("payload too short"));
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }
}

// ── Ranging (260, vl53l1x) ──────────────────────

#[test]
fn ranging_frame_produces_sensor_data() {
    let frame = BravePiFrame::Sensor(make_sensor_frame(260, vec![0xe8, 0x03]));
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData { reading, .. } => {
            assert_eq!(reading.sensor_type, SensorType::Ranging);
            assert!(!reading.values.is_empty());
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── ADC (259, mcp3427) ──────────────────────────

#[test]
fn adc_frame_produces_sensor_data() {
    let frame = BravePiFrame::Sensor(make_sensor_frame(259, vec![0x00, 0x00, 0x80, 0x3f]));
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData { reading, .. } => {
            assert_eq!(reading.sensor_type, SensorType::Adc);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── rssi / battery が正しく伝搬される ───────────

#[test]
fn rssi_and_battery_are_propagated() {
    let frame = BravePiFrame::Sensor(SensorFrame {
        device_number: "test_device".to_string(),
        sensor_type_raw: 257,
        rssi: -128,
        battery: 0,
        data_count: 1,
        value_data: vec![0x01],
    });
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData {
            rssi, battery_pct, ..
        } => {
            assert_eq!(rssi, Some(-128));
            assert_eq!(battery_pct, Some(0));
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── 空の value_data でもパニックしない ────────────

#[test]
fn empty_value_data_does_not_panic() {
    let frame = BravePiFrame::Sensor(make_sensor_frame(261, vec![]));
    let event = frame_to_event(frame, "/dev/test");
    assert!(event.is_some());
}

// ── ContactInput で data_count > value_data.len() ─

#[test]
fn contact_input_data_count_exceeds_data_does_not_panic() {
    let frame = BravePiFrame::Sensor(SensorFrame {
        device_number: "test".to_string(),
        sensor_type_raw: 257,
        rssi: -50,
        battery: 50,
        data_count: 100,
        value_data: vec![0x01, 0x00],
    });
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::SensorData { reading, .. } => {
            assert_eq!(reading.values.len(), 2);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }
}

// ── SensorIdentity tests ────────────────────────

#[test]
fn temperature_frame_returns_identity() {
    let frame = BravePiFrame::Sensor(make_sensor_frame(261, vec![0x00, 0x80, 0xb3, 0x41]));
    let (_event, identity) = frame_to_event(frame, "/dev/ttyAMA0").expect("should produce event");

    let identity = identity.expect("temperature should have identity");
    assert_eq!(identity.manufacturer, "Braveridge");
    assert_eq!(identity.ic_part_number, "MCP9600");
    assert_eq!(identity.sensor_type, SensorType::Temperature);
    assert_eq!(identity.connection.kind, iotkit_core_types::ConnectionKind::Uart);
    assert_eq!(identity.connection.parameters.get("port").unwrap(), "/dev/ttyAMA0");
    assert_eq!(identity.connection.parameters.get("transmitter_id").unwrap(), "246880020140018b");
}

#[test]
fn contact_input_has_no_identity() {
    let frame = BravePiFrame::Sensor(SensorFrame {
        device_number: "test".to_string(),
        sensor_type_raw: 257,
        rssi: -50,
        battery: 80,
        data_count: 1,
        value_data: vec![0x01],
    });
    let (_event, identity) = frame_to_event(frame, "/dev/test").expect("should produce event");
    assert!(identity.is_none());
}

// ── DecodeError "unknown" → device_key: None ────

#[test]
fn decode_error_unknown_device_produces_none_key() {
    let frame = BravePiFrame::DecodeError {
        device_number: "unknown".to_string(),
        sensor_type_raw: 0,
        reason: "frame size exceeds maximum".to_string(),
    };
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::AdapterError { device_key, error } => {
            assert!(device_key.is_none(), "unknown device should produce None key");
            assert!(error.contains("frame size exceeds maximum"));
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }
}
```

- [ ] **Step 4: Delete old integration test**

```bash
rm bravepi-adapter/tests/frame_to_event_test.rs
```

- [ ] **Step 5: Run tests to verify**

Run: `cargo test -p bravepi-adapter`
Expected: All 17 tests pass (4 event_loop + 13 convert). No integration tests remain.

- [ ] **Step 6: Commit**

```bash
git add bravepi-adapter/src/task/convert.rs bravepi-adapter/src/task/mod.rs bravepi-adapter/src/task/convert_test.rs
git rm bravepi-adapter/tests/frame_to_event_test.rs
git commit -m "refactor(bravepi): migrate frame_to_event to pub(crate), move tests to crate-internal"
```

---

### Task 4: Create serial_source module (extract from reader.rs)

**Files:**
- Create: `bravepi-adapter/src/task/serial_source.rs`
- Modify: `bravepi-adapter/src/task/mod.rs`
- Delete: `bravepi-adapter/src/task/reader.rs`

- [ ] **Step 1: Create `serial_source.rs`**

This file combines the `SerialSource`/`SerialSourceHandle` types from the spec with the `serial_reader_thread` logic moved from `reader.rs`. The key changes from `reader.rs`:
- `bytes_tx` type changes from `mpsc::Sender<Result<Vec<u8>, String>>` to `mpsc::Sender<Result<Vec<u8>, TransportError>>`
- Error sends construct `TransportError { message: msg }` instead of bare `String`
- `start()` function encapsulates port open + thread spawn

```rust
// bravepi-adapter/src/task/serial_source.rs

//! Serial port source: port open + reader thread + reconnect。
//! event_loop に bytes channel を提供する。

use std::time::Duration;

use rpi4b_transport::SerialTransport;
use tokio::sync::mpsc;

use crate::serial_config;
use crate::transport::{BytesReceiver, TransportError};

pub(crate) struct SerialSource {
    pub bytes_rx: BytesReceiver,
    pub handle: SerialSourceHandle,
}

pub(crate) struct SerialSourceHandle {
    thread_handle: std::thread::JoinHandle<()>,
}

impl SerialSourceHandle {
    pub async fn join(self) -> Result<(), String> {
        tokio::task::spawn_blocking(|| self.thread_handle.join())
            .await
            .map_err(|_| "spawn_blocking failed".to_string())?
            .map_err(|_| "Reader thread panicked".to_string())
    }
}

const MAX_RETRIES: u32 = 10;
const MAX_BACKOFF_SECS: u64 = 30;

/// SerialTransport を開き、reader thread を起動する。
/// reconnect ロジックもこの中に閉じる。
pub(crate) fn start(port_path: &str) -> Result<SerialSource, std::io::Error> {
    let config = serial_config();
    let transport = SerialTransport::open(port_path, &config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let (bytes_tx, bytes_rx) = mpsc::channel(64);
    let owned_path = port_path.to_string();
    let thread_handle = std::thread::Builder::new()
        .name(format!("bravepi-serial-{}", port_path))
        .spawn(move || serial_reader_thread(owned_path, transport, bytes_tx))?;
    Ok(SerialSource {
        bytes_rx,
        handle: SerialSourceHandle { thread_handle },
    })
}

fn serial_reader_thread(
    port_path: String,
    mut transport: SerialTransport,
    bytes_tx: mpsc::Sender<Result<Vec<u8>, TransportError>>,
) {
    tracing::info!(port = %port_path, "Serial reader thread started");
    let mut buf = [0u8; 4096];
    let timeout = Duration::from_millis(500);
    let mut retry_count: u32 = 0;

    loop {
        if bytes_tx.is_closed() {
            tracing::info!("Bytes channel closed, reader thread exiting");
            return;
        }

        match transport.read(&mut buf, timeout) {
            Ok(0) => continue,
            Ok(n) => {
                retry_count = 0;
                if bytes_tx.blocking_send(Ok(buf[..n].to_vec())).is_err() {
                    tracing::info!("Bytes channel closed, reader thread exiting");
                    return;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => {
                tracing::error!(error = %e, port = %port_path, "Serial read error");
                drop(transport);

                loop {
                    retry_count += 1;
                    if retry_count > MAX_RETRIES {
                        let msg = format!(
                            "Serial read error on {}: {} (max retries {} exceeded)",
                            port_path, e, MAX_RETRIES
                        );
                        tracing::error!("{}", msg);
                        let _ = bytes_tx.blocking_send(Err(TransportError { message: msg }));
                        return;
                    }

                    if bytes_tx.is_closed() {
                        tracing::info!("Bytes channel closed during retry, exiting");
                        return;
                    }

                    let backoff_secs = (1u64 << retry_count.min(5)).min(MAX_BACKOFF_SECS);
                    tracing::warn!(
                        port = %port_path,
                        retry = retry_count,
                        backoff_secs = backoff_secs,
                        "Attempting serial reconnect"
                    );
                    for _ in 0..backoff_secs {
                        if bytes_tx.is_closed() {
                            tracing::info!("Bytes channel closed during retry, exiting");
                            return;
                        }
                        std::thread::sleep(Duration::from_secs(1));
                    }

                    let config = serial_config();
                    match SerialTransport::open(&port_path, &config) {
                        Ok(new_transport) => {
                            tracing::info!(port = %port_path, "Serial reconnected");
                            transport = new_transport;
                            retry_count = 0;
                            break;
                        }
                        Err(open_err) => {
                            tracing::warn!(
                                error = %open_err,
                                port = %port_path,
                                "Reconnect failed"
                            );
                        }
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Update `mod.rs` — replace `reader` with `serial_source`**

Replace the full content of `bravepi-adapter/src/task/mod.rs`:

```rust
//! BravePI adapter async task。
//!
//! シリアルポートからフレームを読み、AdapterEvent に変換して channel に送信する。
//! blocking serial I/O は専用スレッドで実行し、async 側と bytes channel で接続する。

mod convert;
pub(crate) mod event_loop;
mod handle;
mod serial_source;

pub use handle::{start, AdapterHandle};

#[cfg(test)]
mod event_loop_test;
#[cfg(test)]
mod convert_test;
```

- [ ] **Step 3: Delete `reader.rs`**

```bash
rm bravepi-adapter/src/task/reader.rs
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p bravepi-adapter`
Expected: Compiles. `handle.rs` still references `super::reader::serial_reader_thread` which no longer exists — this will cause a compile error. Proceed to Step 5.

Actually, `handle.rs` imports `super::reader::serial_reader_thread`. After deleting `reader.rs` and before updating `handle.rs`, compilation will fail. This is expected — Task 5 fixes `handle.rs`. To keep the build green within this task, we need to update `handle.rs` imports minimally.

- [ ] **Step 4 (revised): Temporarily update `handle.rs` imports**

In `bravepi-adapter/src/task/handle.rs`, change line 9:

From:
```rust
use super::reader::serial_reader_thread;
```
To:
```rust
use super::serial_source;
```

And change the `start()` function to use `serial_source::start()`. This is actually the full handle.rs refactor, so we combine this with Task 5. **Instead, do not delete `reader.rs` yet — leave it and proceed to Task 5 which does the full handle.rs refactor and reader.rs deletion together.**

**Revised approach for this task:** Create `serial_source.rs` and update `mod.rs` to include it, but keep `reader.rs` alive (it will be unused but still compiles). Task 5 will switch `handle.rs` to use `serial_source` and delete `reader.rs`.

Update `mod.rs` to keep both modules temporarily:

```rust
//! BravePI adapter async task。
//!
//! シリアルポートからフレームを読み、AdapterEvent に変換して channel に送信する。
//! blocking serial I/O は専用スレッドで実行し、async 側と bytes channel で接続する。

mod convert;
pub(crate) mod event_loop;
mod handle;
mod reader;
mod serial_source;

pub use handle::{start, AdapterHandle};

#[cfg(test)]
mod event_loop_test;
#[cfg(test)]
mod convert_test;
```

- [ ] **Step 5: Verify compilation and tests**

Run: `cargo test -p bravepi-adapter`
Expected: All 17 tests pass. `serial_source` compiles but is unused (dead_code warning is OK).

- [ ] **Step 6: Commit**

```bash
git add bravepi-adapter/src/task/serial_source.rs bravepi-adapter/src/task/mod.rs
git commit -m "feat(bravepi): add serial_source module with SerialSource, SerialSourceHandle, and reader thread"
```

---

### Task 5: Refactor handle.rs to use serial_source, new shutdown, delete reader.rs

**Files:**
- Modify: `bravepi-adapter/src/task/handle.rs`
- Modify: `bravepi-adapter/src/task/mod.rs`
- Delete: `bravepi-adapter/src/task/reader.rs`

- [ ] **Step 1: Rewrite `handle.rs`**

Replace the full content of `bravepi-adapter/src/task/handle.rs`:

```rust
//! AdapterHandle: adapter の起動とライフサイクル管理。

use iotkit_core_types::{AdapterCommand, AdapterEvent, AdapterId};
use tokio::sync::mpsc;

use super::event_loop::event_loop;
use super::serial_source::{self, SerialSourceHandle};

/// adapter 起動結果。core はこの handle を使って adapter と通信する。
pub struct AdapterHandle {
    pub id: AdapterId,
    pub event_rx: mpsc::Receiver<AdapterEvent>,
    pub command_tx: mpsc::Sender<AdapterCommand>,
    source_handle: Option<SerialSourceHandle>,
    event_loop_handle: Option<tokio::task::JoinHandle<()>>,
}

impl AdapterHandle {
    /// シャットダウン: event_rx close → Shutdown cmd → event_loop join → reader thread join。
    pub async fn shutdown(mut self) -> Result<(), String> {
        // 1. event_rx を close → event_loop の send() が Err で抜ける (buffer 詰まり対策)
        self.event_rx.close();

        // 2. Shutdown コマンド送信 → event_loop が select で観測して return
        let _ = self.command_tx.send(AdapterCommand::Shutdown).await;

        // 3. event_loop の完了を待つ
        if let Some(handle) = self.event_loop_handle.take() {
            handle.await.map_err(|e| format!("event_loop panicked: {}", e))?;
        }

        // 4. reader thread の join
        //    event_loop 終了 → bytes_rx drop → bytes_tx.is_closed() = true
        //    → reader thread が次の is_closed() チェックで終了
        if let Some(source) = self.source_handle.take() {
            source.join().await?;
        }

        Ok(())
    }
}

/// BravePI adapter を起動する。
///
/// 戻り値の `AdapterHandle` 経由で event を受信し、command を送信する。
/// serial read は専用スレッド、フレーム処理は tokio task で動作する。
pub fn start(port_path: String) -> Result<AdapterHandle, std::io::Error> {
    let source = serial_source::start(&port_path)?;

    let (event_tx, event_rx) = mpsc::channel::<AdapterEvent>(256);
    let (command_tx, command_rx) = mpsc::channel::<AdapterCommand>(32);
    let id = AdapterId::new(format!("bravepi:{}", port_path));

    let event_loop_handle = tokio::spawn(
        event_loop(port_path, source.bytes_rx, event_tx, command_rx)
    );

    Ok(AdapterHandle {
        id,
        event_rx,
        command_tx,
        source_handle: Some(source.handle),
        event_loop_handle: Some(event_loop_handle),
    })
}
```

- [ ] **Step 2: Remove `reader` from `mod.rs`**

Replace the full content of `bravepi-adapter/src/task/mod.rs`:

```rust
//! BravePI adapter async task。
//!
//! シリアルポートからフレームを読み、AdapterEvent に変換して channel に送信する。
//! blocking serial I/O は専用スレッドで実行し、async 側と bytes channel で接続する。

mod convert;
pub(crate) mod event_loop;
mod handle;
mod serial_source;

pub use handle::{start, AdapterHandle};

#[cfg(test)]
mod event_loop_test;
#[cfg(test)]
mod convert_test;
```

- [ ] **Step 3: Delete `reader.rs`**

```bash
rm bravepi-adapter/src/task/reader.rs
```

- [ ] **Step 4: Run all tests**

Run: `cargo test -p bravepi-adapter`
Expected: All 17 tests pass. No compilation errors.

Run: `cargo test -p bravepi-codec`
Expected: All codec tests pass (unchanged).

- [ ] **Step 5: Check the PoC binary compiles**

Run: `cargo check -p bravepi-poc`
Expected: Compiles. The PoC uses `bravepi_adapter::task::{start, AdapterHandle}` which is still public.

- [ ] **Step 6: Commit**

```bash
git add bravepi-adapter/src/task/handle.rs bravepi-adapter/src/task/mod.rs
git rm bravepi-adapter/src/task/reader.rs
git commit -m "refactor(bravepi): wire serial_source into handle.rs, add event_rx.close() shutdown, delete reader.rs"
```

---

## Post-Implementation Checklist

After all 5 tasks are complete:

1. **Full test suite:** `cargo test --workspace` — all tests pass
2. **No warnings check:** `cargo check --workspace 2>&1 | grep warning` — only expected dead_code warnings (if any)
3. **Verify deleted files:** `reader.rs` is gone, `tests/event_loop_test.rs` and `tests/frame_to_event_test.rs` are gone
4. **Verify new files:** `transport.rs`, `serial_source.rs`, `event_loop_test.rs`, `convert_test.rs` exist in correct locations
5. **Verify `tests/` directory:** should be empty (or not exist) — can be deleted if empty
