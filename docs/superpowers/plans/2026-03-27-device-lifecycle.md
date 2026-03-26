# Device Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the DeviceDiscovered contract — all BravePI devices emit DeviceDiscovered with composite DeviceKey and required SensorIdentity.

**Architecture:** Two files change: `convert.rs` gets composite key generation and contact identity helpers; `event_loop.rs` replaces `HashSet<DeviceKey>` with `HashMap<DeviceKey, DeviceState>` and enforces "no DeviceDiscovered = no SensorData" invariant. Core types unchanged.

**Tech Stack:** Rust, tokio, iotkit-core-types

**Spec:** `docs/superpowers/specs/2026-03-27-device-lifecycle-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `bravepi-adapter/src/task/convert.rs` | Modify | Add `device_key_suffix()`, `contact_identity()` helpers; composite key; contact identity |
| `bravepi-adapter/src/task/event_loop.rs` | Modify | `HashMap<DeviceKey, DeviceState>`; insert ordering; identity=None guard |
| `bravepi-adapter/src/task/convert_test.rs` | Modify | Update key assertions; flip contact identity test; add DecodeError composite test |
| `bravepi-adapter/src/task/event_loop_test.rs` | Modify | Update key assertions; add contact + multi-sensor-type tests |

No new files. No changes to core/types, codec, sensors, transport, serial_source, or handle.

---

### Task 1: Composite key in convert.rs

Add `device_key_suffix()` helper and change `frame_to_event()` to generate composite keys (`bravepi:{transmitter_id}:{suffix}`). Unknown sensor type handling moves from the inner match to the suffix check (same behavior, cleaner flow).

**Files:**
- Modify: `bravepi-adapter/src/task/convert.rs`
- Test: `bravepi-adapter/src/task/convert_test.rs`

- [ ] **Step 1: Update test to expect composite key**

In `bravepi-adapter/src/task/convert_test.rs`, change the assertion in `temperature_frame_produces_sensor_data`:

```rust
// Was:
assert_eq!(device_key.as_str(), "246880020140018b");
// Change to:
assert_eq!(device_key.as_str(), "bravepi:246880020140018b:temperature");
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p bravepi-adapter temperature_frame_produces_sensor_data`

Expected: FAIL — actual key is `"246880020140018b"`, expected `"bravepi:246880020140018b:temperature"`.

- [ ] **Step 3: Add `device_key_suffix()` and change key generation**

In `bravepi-adapter/src/task/convert.rs`, add at the bottom of the file:

```rust
fn device_key_suffix(sensor_type: &SensorType) -> Option<&'static str> {
    match sensor_type {
        SensorType::ContactInput => Some("contact_input"),
        SensorType::ContactOutput => Some("contact_output"),
        SensorType::Adc => Some("adc"),
        SensorType::Ranging => Some("ranging"),
        SensorType::Temperature => Some("temperature"),
        SensorType::Acceleration => Some("acceleration"),
        SensorType::DifferentialPressure => Some("differential_pressure"),
        SensorType::Illuminance => Some("illuminance"),
        SensorType::Unknown(_) => None,
    }
}
```

Then in `frame_to_event()`, replace the beginning of the `BravePiFrame::Sensor(s)` arm. The current code is:

```rust
BravePiFrame::Sensor(s) => {
    let sensor_type = sensor_type_from_bravepi_raw(s.sensor_type_raw);
    let device_key = DeviceKey::new(s.device_number.clone());

    let conn_info = BravepiConnection::Uart {
        port: port_path.to_string(),
        transmitter_id: s.device_number.clone(),
    }
    .to_connection_info();

    // reading と identity を同じ match で生成。...
    let (reading, identity) = match sensor_type {
        // ...
        SensorType::Unknown(_) => {
            tracing::warn!(raw = s.sensor_type_raw, "Unknown sensor type, skipping");
            return None;
        }
    };
```

Replace with:

```rust
BravePiFrame::Sensor(s) => {
    let sensor_type = sensor_type_from_bravepi_raw(s.sensor_type_raw);

    let suffix = match device_key_suffix(&sensor_type) {
        Some(suffix) => suffix,
        None => {
            tracing::warn!(raw = s.sensor_type_raw, "Unknown sensor type, skipping");
            return None;
        }
    };

    let transmitter_id = s.device_number.clone();
    let device_key = DeviceKey::new(format!("bravepi:{}:{}", transmitter_id, suffix));

    let conn_info = BravepiConnection::Uart {
        port: port_path.to_string(),
        transmitter_id,
    }
    .to_connection_info();

    let (reading, identity) = match sensor_type {
        // ... (all existing arms EXCEPT Unknown) ...
        SensorType::Unknown(_) => unreachable!("Unknown filtered by device_key_suffix"),
    };
```

Note: `transmitter_id` is borrowed by `format!()` then moved into `BravepiConnection`. This compiles because `format!()` only borrows.

- [ ] **Step 4: Run the updated test to verify it passes**

Run: `cargo test -p bravepi-adapter temperature_frame_produces_sensor_data`

Expected: PASS

- [ ] **Step 5: Run all convert tests to verify nothing else broke**

Run: `cargo test -p bravepi-adapter convert_test`

Expected: ALL PASS. The `unknown_sensor_type_returns_none` test still passes because `device_key_suffix` returns `None` for `Unknown`, triggering the same early return. Other tests don't assert on `device_key` so they pass unchanged.

- [ ] **Step 6: Commit**

```bash
git add bravepi-adapter/src/task/convert.rs bravepi-adapter/src/task/convert_test.rs
git commit -m "feat(bravepi): composite device key (bravepi:{transmitter}:{suffix})"
```

---

### Task 2: Contact identity in convert.rs

Add `contact_identity()` helper so ContactInput/ContactOutput return `Some(SensorIdentity)` instead of `None`.

**Files:**
- Modify: `bravepi-adapter/src/task/convert.rs`
- Test: `bravepi-adapter/src/task/convert_test.rs`

- [ ] **Step 1: Flip the contact identity test and add assertions**

In `bravepi-adapter/src/task/convert_test.rs`, replace the `contact_input_has_no_identity` test:

```rust
// Was:
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

// Replace with:
#[test]
fn contact_input_has_module_identity() {
    let frame = BravePiFrame::Sensor(SensorFrame {
        device_number: "test".to_string(),
        sensor_type_raw: 257,
        rssi: -50,
        battery: 80,
        data_count: 1,
        value_data: vec![0x01],
    });
    let (_event, identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    let identity = identity.expect("contact_input should have identity");
    assert_eq!(identity.manufacturer, "Braveridge");
    assert_eq!(identity.ic_part_number, "Contact Input Module");
    assert_eq!(identity.sensor_type, SensorType::ContactInput);
    assert_eq!(identity.connection.kind, iotkit_core_types::ConnectionKind::Uart);
    assert_eq!(identity.connection.parameters.get("transmitter_id").unwrap(), "test");
}
```

- [ ] **Step 2: Add a ContactOutput identity test**

Add to `bravepi-adapter/src/task/convert_test.rs`:

```rust
#[test]
fn contact_output_has_module_identity() {
    let frame = BravePiFrame::Sensor(SensorFrame {
        device_number: "1234567890abcdef".to_string(),
        sensor_type_raw: 258,
        rssi: -70,
        battery: 100,
        data_count: 1,
        value_data: vec![0x01],
    });
    let (_event, identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    let identity = identity.expect("contact_output should have identity");
    assert_eq!(identity.manufacturer, "Braveridge");
    assert_eq!(identity.ic_part_number, "Contact Output Module");
    assert_eq!(identity.sensor_type, SensorType::ContactOutput);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p bravepi-adapter contact_input_has_module_identity contact_output_has_module_identity`

Expected: FAIL — `identity.expect(...)` panics because identity is currently `None`.

- [ ] **Step 4: Add `contact_identity()` and update the contact match arm**

In `bravepi-adapter/src/task/convert.rs`, add the import at the top (if not already there):

```rust
use iotkit_core_types::{
    AdapterEvent, ConnectionInfo, DeviceKey, SensorIdentity, SensorReading, SensorType,
};
```

Add `contact_identity()` at the bottom of the file (next to `device_key_suffix()`):

```rust
fn contact_identity(sensor_type: &SensorType, conn_info: ConnectionInfo) -> SensorIdentity {
    SensorIdentity {
        manufacturer: "Braveridge".to_string(),
        ic_part_number: match sensor_type {
            SensorType::ContactInput => "Contact Input Module".to_string(),
            SensorType::ContactOutput => "Contact Output Module".to_string(),
            _ => unreachable!("contact_identity called with non-contact sensor type"),
        },
        sensor_type: sensor_type.clone(),
        connection: conn_info,
    }
}
```

Then replace the `ContactInput | ContactOutput` arm in the match:

```rust
// Was:
SensorType::ContactInput | SensorType::ContactOutput => {
    let values: Vec<f64> = s
        .value_data
        .iter()
        .take(s.data_count as usize)
        .map(|&b| if b != 0 { 1.0 } else { 0.0 })
        .collect();
    (SensorReading::new(sensor_type.clone(), values, vec![]), None)
}

// Replace with:
SensorType::ContactInput | SensorType::ContactOutput => {
    let values: Vec<f64> = s
        .value_data
        .iter()
        .take(s.data_count as usize)
        .map(|&b| if b != 0 { 1.0 } else { 0.0 })
        .collect();
    (
        SensorReading::new(sensor_type.clone(), values, vec![]),
        Some(contact_identity(&sensor_type, conn_info)),
    )
}
```

Note: `conn_info` is moved into `contact_identity()`. This is fine because the other arms also move `conn_info` into their identity functions.

Also add `ConnectionInfo` to the import from `iotkit_core_types` (needed for the `contact_identity` signature):

```rust
use iotkit_core_types::{
    AdapterEvent, ConnectionInfo, DeviceKey, SensorIdentity, SensorReading, SensorType,
};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p bravepi-adapter convert_test`

Expected: ALL PASS

- [ ] **Step 6: Commit**

```bash
git add bravepi-adapter/src/task/convert.rs bravepi-adapter/src/task/convert_test.rs
git commit -m "feat(bravepi): contact sensors return module-level identity"
```

---

### Task 3: DecodeError composite key

Update the `DecodeError` branch to use composite key when the sensor type suffix is known.

**Files:**
- Modify: `bravepi-adapter/src/task/convert.rs`
- Test: `bravepi-adapter/src/task/convert_test.rs`

- [ ] **Step 1: Update the DecodeError test to expect composite key**

In `bravepi-adapter/src/task/convert_test.rs`, change the assertion in `decode_error_produces_adapter_error`:

```rust
// Was:
assert_eq!(device_key.unwrap().as_str(), "bad_device");
// Change to:
assert_eq!(device_key.unwrap().as_str(), "bravepi:bad_device:temperature");
```

This test uses `sensor_type_raw: 261` (Temperature), so the suffix is "temperature".

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p bravepi-adapter decode_error_produces_adapter_error`

Expected: FAIL — actual key is `"bad_device"`, expected `"bravepi:bad_device:temperature"`.

- [ ] **Step 3: Update the DecodeError branch**

In `bravepi-adapter/src/task/convert.rs`, replace the `BravePiFrame::DecodeError` arm:

```rust
// Was:
BravePiFrame::DecodeError {
    device_number,
    sensor_type_raw,
    reason,
} => {
    let device_key = if device_number == "unknown" {
        None
    } else {
        Some(DeviceKey::new(device_number))
    };
    Some((
        AdapterEvent::AdapterError {
            device_key,
            error: format!("Decode error (type={}): {}", sensor_type_raw, reason),
        },
        None,
    ))
}

// Replace with:
BravePiFrame::DecodeError {
    device_number,
    sensor_type_raw,
    reason,
} => {
    let device_key = if device_number == "unknown" {
        None
    } else {
        let sensor_type = sensor_type_from_bravepi_raw(sensor_type_raw);
        Some(match device_key_suffix(&sensor_type) {
            Some(suffix) => {
                DeviceKey::new(format!("bravepi:{}:{}", device_number, suffix))
            }
            None => DeviceKey::new(device_number),
        })
    };
    Some((
        AdapterEvent::AdapterError {
            device_key,
            error: format!("Decode error (type={}): {}", sensor_type_raw, reason),
        },
        None,
    ))
}
```

- [ ] **Step 4: Add DecodeError with unknown sensor type test (fallback to raw key)**

Add to `bravepi-adapter/src/task/convert_test.rs`:

```rust
#[test]
fn decode_error_unknown_sensor_type_falls_back_to_raw_key() {
    let frame = BravePiFrame::DecodeError {
        device_number: "bad_device".to_string(),
        sensor_type_raw: 9999,
        reason: "bad payload".to_string(),
    };
    let (event, _identity) = frame_to_event(frame, "/dev/test").expect("should produce event");

    match event {
        AdapterEvent::AdapterError { device_key, .. } => {
            assert_eq!(device_key.unwrap().as_str(), "bad_device");
        }
        other => panic!("expected AdapterError, got {:?}", other),
    }
}
```

This tests the fallback path: sensor type 9999 is Unknown, so `device_key_suffix` returns None, and the raw device_number is used as the key.

- [ ] **Step 5: Run all convert tests**

Run: `cargo test -p bravepi-adapter convert_test`

Expected: ALL PASS. The `decode_error_unknown_device_produces_none_key` test is also unchanged (device_number == "unknown" → None key).

- [ ] **Step 6: Commit**

```bash
git add bravepi-adapter/src/task/convert.rs bravepi-adapter/src/task/convert_test.rs
git commit -m "feat(bravepi): DecodeError uses composite key when suffix is known"
```

---

### Task 4: DeviceState and event_loop lifecycle changes

Replace `HashSet<DeviceKey>` with `HashMap<DeviceKey, DeviceState>`. Insert only after DeviceDiscovered succeeds. Guard against identity=None on new devices (warn + skip SensorData).

**Files:**
- Modify: `bravepi-adapter/src/task/event_loop.rs`
- Test: `bravepi-adapter/src/task/event_loop_test.rs`

- [ ] **Step 1: Update existing test to expect composite key**

In `bravepi-adapter/src/task/event_loop_test.rs`, update assertions in `normal_data_flow_produces_device_discovered_then_sensor_data`:

```rust
// In the DeviceDiscovered match (around line 92):
// Was:
assert_eq!(device_key.as_str(), "246880020140018b");
// Change to:
assert_eq!(device_key.as_str(), "bravepi:246880020140018b:temperature");

// In the first SensorData match (around line 101):
// Was:
assert_eq!(device_key.as_str(), "246880020140018b");
// Change to:
assert_eq!(device_key.as_str(), "bravepi:246880020140018b:temperature");
```

- [ ] **Step 2: Run test to verify it passes (composite key comes from convert.rs, already changed)**

Run: `cargo test -p bravepi-adapter normal_data_flow`

Expected: PASS — convert.rs already produces composite keys from Task 1.

- [ ] **Step 3: Replace HashSet with HashMap + DeviceState**

In `bravepi-adapter/src/task/event_loop.rs`, make these changes:

Change the import:

```rust
// Was:
use std::collections::HashSet;
// Change to:
use std::collections::HashMap;
```

Add the `DeviceState` struct after the imports:

```rust
struct DeviceState {
    last_seen: tokio::time::Instant,
}
```

Change the `seen_devices` declaration:

```rust
// Was:
let mut seen_devices: HashSet<DeviceKey> = HashSet::new();
// Change to:
let mut devices: HashMap<DeviceKey, DeviceState> = HashMap::new();
```

Replace the lifecycle tracking block inside the `while let Some(frame) = codec.decode()` loop. The current code is:

```rust
if let Some((event, identity)) = frame_to_event(frame, &port_path) {
    // 初回デバイスは DeviceDiscovered を先に送信
    if let AdapterEvent::SensorData { ref device_key, .. } = event {
        // seen_devices は identity の有無に関わらず記録する。
        // adapter 再起動時にリセットされるため、DeviceDiscovered は再送信される（意図通り）。
        if seen_devices.insert(device_key.clone()) {
            if let Some(identity) = identity {
                let discovered = AdapterEvent::DeviceDiscovered {
                    device_key: device_key.clone(),
                    identity,
                };
                if event_tx.send(discovered).await.is_err() {
                    tracing::warn!("Event channel closed, shutting down");
                    return;
                }
            }
        }
    }

    if event_tx.send(event).await.is_err() {
        tracing::warn!("Event channel closed, shutting down");
        return;
    }
}
```

Replace with:

```rust
if let Some((event, identity)) = frame_to_event(frame, &port_path) {
    if let AdapterEvent::SensorData { ref device_key, .. } = event {
        if !devices.contains_key(device_key) {
            match identity {
                Some(identity) => {
                    let discovered = AdapterEvent::DeviceDiscovered {
                        device_key: device_key.clone(),
                        identity,
                    };
                    if event_tx.send(discovered).await.is_err() {
                        tracing::warn!("Event channel closed, shutting down");
                        return;
                    }
                    devices.insert(
                        device_key.clone(),
                        DeviceState { last_seen: tokio::time::Instant::now() },
                    );
                }
                None => {
                    tracing::warn!(
                        device_key = %device_key,
                        "New device without identity, skipping"
                    );
                    continue;
                }
            }
        } else {
            devices.get_mut(device_key).unwrap().last_seen =
                tokio::time::Instant::now();
        }
    }

    if event_tx.send(event).await.is_err() {
        tracing::warn!("Event channel closed, shutting down");
        return;
    }
}
```

Key changes:
- `contains_key` instead of `insert` (insert happens after DeviceDiscovered)
- `identity=None` for new device → warn + `continue` (skips SensorData send)
- Known device → `last_seen` update
- `devices.insert()` only after DeviceDiscovered send succeeds

- [ ] **Step 4: Run all event_loop tests**

Run: `cargo test -p bravepi-adapter event_loop_test`

Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add bravepi-adapter/src/task/event_loop.rs bravepi-adapter/src/task/event_loop_test.rs
git commit -m "refactor(bravepi): HashSet→HashMap<DeviceKey,DeviceState>, enforce identity-required invariant"
```

---

### Task 5: New event_loop integration tests

Add two integration tests: (1) contact device emits DeviceDiscovered, (2) same transmitter with different sensor types produces two separate logical devices.

**Files:**
- Test: `bravepi-adapter/src/task/event_loop_test.rs`

- [ ] **Step 1: Add contact DeviceDiscovered test**

Add to `bravepi-adapter/src/task/event_loop_test.rs`:

```rust
#[tokio::test]
async fn contact_input_produces_device_discovered() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);

    let handle = tokio::spawn(event_loop("/dev/test".into(), bytes_rx, event_tx, command_rx));

    let device: u64 = 0xaabbccdd00112233;
    let frame_bytes = build_sensor_frame_bytes(device, 257, -50, 80, 1, &[0x01]);
    bytes_tx.send(Ok(frame_bytes)).await.unwrap();

    match event_rx.recv().await.expect("should receive DeviceDiscovered") {
        AdapterEvent::DeviceDiscovered { device_key, identity } => {
            assert_eq!(device_key.as_str(), "bravepi:aabbccdd00112233:contact_input");
            assert_eq!(identity.manufacturer, "Braveridge");
            assert_eq!(identity.ic_part_number, "Contact Input Module");
            assert_eq!(identity.sensor_type, SensorType::ContactInput);
        }
        other => panic!("expected DeviceDiscovered, got {:?}", other),
    }

    match event_rx.recv().await.expect("should receive SensorData") {
        AdapterEvent::SensorData { device_key, reading, .. } => {
            assert_eq!(device_key.as_str(), "bravepi:aabbccdd00112233:contact_input");
            assert_eq!(reading.sensor_type, SensorType::ContactInput);
        }
        other => panic!("expected SensorData, got {:?}", other),
    }

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}
```

- [ ] **Step 2: Add same-transmitter-different-sensor-type test**

Add to `bravepi-adapter/src/task/event_loop_test.rs`:

```rust
#[tokio::test]
async fn same_transmitter_different_sensor_type_produces_two_discoveries() {
    let (bytes_tx, bytes_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(16);

    let handle = tokio::spawn(event_loop("/dev/test".into(), bytes_rx, event_tx, command_rx));

    let device: u64 = 0x246880020140018b;

    // --- Temperature frame ---
    let temp_bytes = build_sensor_frame_bytes(device, 261, -60, 95, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(temp_bytes)).await.unwrap();

    // DeviceDiscovered for temperature
    match event_rx.recv().await.expect("should receive DeviceDiscovered #1") {
        AdapterEvent::DeviceDiscovered { device_key, .. } => {
            assert_eq!(device_key.as_str(), "bravepi:246880020140018b:temperature");
        }
        other => panic!("expected DeviceDiscovered, got {:?}", other),
    }
    // SensorData for temperature
    match event_rx.recv().await.expect("should receive SensorData #1") {
        AdapterEvent::SensorData { device_key, .. } => {
            assert_eq!(device_key.as_str(), "bravepi:246880020140018b:temperature");
        }
        other => panic!("expected SensorData, got {:?}", other),
    }

    // --- ContactInput frame (same transmitter, different sensor type) ---
    let contact_bytes = build_sensor_frame_bytes(device, 257, -55, 90, 1, &[0x01]);
    bytes_tx.send(Ok(contact_bytes)).await.unwrap();

    // DeviceDiscovered for contact_input (different logical device)
    match event_rx.recv().await.expect("should receive DeviceDiscovered #2") {
        AdapterEvent::DeviceDiscovered { device_key, .. } => {
            assert_eq!(device_key.as_str(), "bravepi:246880020140018b:contact_input");
        }
        other => panic!("expected DeviceDiscovered, got {:?}", other),
    }
    // SensorData for contact_input
    match event_rx.recv().await.expect("should receive SensorData #2") {
        AdapterEvent::SensorData { device_key, .. } => {
            assert_eq!(device_key.as_str(), "bravepi:246880020140018b:contact_input");
        }
        other => panic!("expected SensorData, got {:?}", other),
    }

    // --- Repeat: temperature again (no new DeviceDiscovered) ---
    let temp_bytes2 = build_sensor_frame_bytes(device, 261, -58, 92, 1, &[0x00, 0x80, 0xb3, 0x41]);
    bytes_tx.send(Ok(temp_bytes2)).await.unwrap();

    match event_rx.recv().await.expect("should receive SensorData only") {
        AdapterEvent::SensorData { device_key, .. } => {
            assert_eq!(device_key.as_str(), "bravepi:246880020140018b:temperature");
        }
        other => panic!("expected SensorData (no re-discover), got {:?}", other),
    }

    // --- Repeat: contact again (no new DeviceDiscovered) ---
    let contact_bytes2 = build_sensor_frame_bytes(device, 257, -52, 88, 1, &[0x00]);
    bytes_tx.send(Ok(contact_bytes2)).await.unwrap();

    match event_rx.recv().await.expect("should receive SensorData only") {
        AdapterEvent::SensorData { device_key, .. } => {
            assert_eq!(device_key.as_str(), "bravepi:246880020140018b:contact_input");
        }
        other => panic!("expected SensorData (no re-discover), got {:?}", other),
    }

    command_tx.send(AdapterCommand::Shutdown).await.unwrap();
    handle.await.unwrap();
}
```

- [ ] **Step 3: Run all new tests**

Run: `cargo test -p bravepi-adapter event_loop_test`

Expected: ALL PASS (6 tests total: 4 existing + 2 new)

- [ ] **Step 4: Run full test suite**

Run: `cargo test -p bravepi-adapter`

Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add bravepi-adapter/src/task/event_loop_test.rs
git commit -m "test(bravepi): contact DeviceDiscovered + same-transmitter-different-type tests"
```
