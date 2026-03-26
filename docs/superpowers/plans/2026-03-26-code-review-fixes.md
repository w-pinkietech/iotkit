# Code Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix 8 issues (Critical 3 + Important 5) identified in the architecture code review.

**Architecture:** Bottom-up approach — start with core types changes, then codec changes, then adapter/sensor updates that depend on them. Each task is self-contained and produces a compiling, test-passing codebase.

**Tech Stack:** Rust, tokio, cargo test

**Spec:** `docs/superpowers/specs/2026-03-26-code-review-fixes-design.md`

---

## File Structure

| File | Responsibility | Tasks |
|------|---------------|-------|
| `core/types/src/lib.rs` | Domain types (SensorReading, AdapterId, DeviceKey) | 1, 7 |
| `bravepi-adapter/codec/src/codec.rs` | BravePiCodec, frame types, encode/decode | 2, 3, 4, 5 |
| `bravepi-adapter/codec/src/lib.rs` | Codec crate re-exports | 6 |
| `bravepi-adapter/codec/tests/codec_test.rs` | Codec unit tests | 2, 3, 4 |
| `bravepi-adapter/sensors/src/mcp9600.rs` | Temperature sensor driver | 1 |
| `bravepi-adapter/sensors/src/opt3001.rs` | Illuminance sensor driver | 1 |
| `bravepi-adapter/sensors/src/mcp3427.rs` | ADC sensor driver | 1 |
| `bravepi-adapter/sensors/src/vl53l1x.rs` | Ranging sensor driver | 1 |
| `bravepi-adapter/sensors/src/sdp810.rs` | Differential pressure sensor driver | 1 |
| `bravepi-adapter/sensors/src/lis2duxs12.rs` | Acceleration sensor driver | 1 |
| `bravepi-adapter/src/task/convert.rs` | Frame-to-event conversion | 1, 7 |
| `bravepi-adapter/src/task/handle.rs` | Adapter lifecycle | 7, 8 |
| `bravepi-adapter/src/task/reader.rs` | Serial reader thread | 8 |
| `bravepi-adapter/src/task/event_loop.rs` | Async event loop | 6 |
| `bravepi-adapter/tests/frame_to_event_test.rs` | Integration tests for frame→event | 1, 7 |
| `bravepi-adapter/tests/event_loop_test.rs` | Integration tests for event loop | 7 |

---

### Task 1: SensorReading に labels フィールド追加 (Critical)

**Files:**
- Modify: `core/types/src/lib.rs:89-103`
- Modify: `bravepi-adapter/sensors/src/mcp9600.rs`
- Modify: `bravepi-adapter/sensors/src/opt3001.rs`
- Modify: `bravepi-adapter/sensors/src/mcp3427.rs`
- Modify: `bravepi-adapter/sensors/src/vl53l1x.rs`
- Modify: `bravepi-adapter/sensors/src/sdp810.rs`
- Modify: `bravepi-adapter/sensors/src/lis2duxs12.rs`
- Modify: `bravepi-adapter/src/task/convert.rs:56-64`
- Modify: `bravepi-adapter/tests/frame_to_event_test.rs`

- [ ] **Step 1: Update SensorReading struct and constructors in core/types**

In `core/types/src/lib.rs`, change:

```rust
/// センサーの値（毎回変わる）。
#[derive(Debug, Clone, PartialEq)]
pub struct SensorReading {
    pub sensor_type: SensorType,
    pub values: Vec<f64>,
    pub labels: Vec<&'static str>,
}

impl SensorReading {
    pub fn new(sensor_type: SensorType, values: Vec<f64>, labels: Vec<&'static str>) -> Self {
        Self { sensor_type, values, labels }
    }

    pub fn empty(sensor_type: SensorType) -> Self {
        Self { sensor_type, values: vec![], labels: vec![] }
    }
}
```

- [ ] **Step 2: Run cargo check to see all compilation errors**

Run: `cargo check 2>&1 | head -50`
Expected: Compilation errors in all sensor modules and convert.rs where `SensorReading::new()` is called with 2 args instead of 3.

- [ ] **Step 3: Update mcp9600 (Temperature) — add labels `["celsius"]`**

In `bravepi-adapter/sensors/src/mcp9600.rs`:

`from_i2c_raw`:
```rust
pub fn from_i2c_raw(data: &[u8; 2]) -> SensorReading {
    let raw = i16::from_be_bytes(*data);
    let temp = raw as f64 * 0.0625;
    SensorReading::new(sensor_type(), vec![temp], vec!["celsius"])
}
```

`from_uart_payload`:
```rust
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 4 {
        return SensorReading::empty(sensor_type());
    }
    let temp = f32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64;
    SensorReading::new(sensor_type(), vec![temp], vec!["celsius"])
}
```

- [ ] **Step 4: Update opt3001 (Illuminance) — add labels `["lux"]`**

In `bravepi-adapter/sensors/src/opt3001.rs`:

`from_i2c_raw`:
```rust
pub fn from_i2c_raw(raw: u16) -> SensorReading {
    let exponent = (raw & 0x00F0) >> 4;
    let fractional = ((raw & 0xFF00) >> 8) + ((raw & 0x000F) << 8);
    let lux = (1u32 << exponent) as f64 * fractional as f64 * 0.01;
    SensorReading::new(sensor_type(), vec![lux], vec!["lux"])
}
```

`from_uart_payload`:
```rust
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 4 {
        return SensorReading::empty(sensor_type());
    }
    let lux = f32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64;
    SensorReading::new(sensor_type(), vec![lux], vec!["lux"])
}
```

- [ ] **Step 5: Update mcp3427 (ADC) — add labels `["ch1_volt", "ch2_volt"]`**

In `bravepi-adapter/sensors/src/mcp3427.rs`:

`from_i2c_volts`:
```rust
pub fn from_i2c_volts(ch1_volt: f64, ch2_volt: f64) -> SensorReading {
    SensorReading::new(sensor_type(), vec![ch1_volt * 1000.0, ch2_volt * 1000.0], vec!["ch1_volt", "ch2_volt"])
}
```

`from_uart_payload`:
```rust
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 4 {
        return SensorReading::empty(sensor_type());
    }
    let ch1 = i16::from_le_bytes([data[0], data[1]]) as f64;
    let ch2 = i16::from_le_bytes([data[2], data[3]]) as f64;
    SensorReading::new(sensor_type(), vec![ch1, ch2], vec!["ch1_volt", "ch2_volt"])
}
```

- [ ] **Step 6: Update vl53l1x (Ranging) — add labels `["distance_mm"]`**

In `bravepi-adapter/sensors/src/vl53l1x.rs`:

`from_i2c_distance`:
```rust
pub fn from_i2c_distance(distance_mm: u16) -> SensorReading {
    if distance_mm == 0 {
        return SensorReading::empty(sensor_type());
    }
    let capped = distance_mm.min(MAX_DISTANCE_MM);
    SensorReading::new(sensor_type(), vec![capped as f64], vec!["distance_mm"])
}
```

`from_uart_payload`:
```rust
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 2 {
        return SensorReading::empty(sensor_type());
    }
    let mm = u16::from_le_bytes([data[0], data[1]]);
    if mm == 0 {
        return SensorReading::empty(sensor_type());
    }
    SensorReading::new(sensor_type(), vec![mm.min(MAX_DISTANCE_MM) as f64], vec!["distance_mm"])
}
```

- [ ] **Step 7: Update sdp810 (DifferentialPressure) — add labels `["pascal"]`**

In `bravepi-adapter/sensors/src/sdp810.rs`:

`from_i2c_raw`:
```rust
pub fn from_i2c_raw(data: &[u8; 9]) -> SensorReading {
    if crc8(&data[0..2]) != data[2] {
        return SensorReading::empty(sensor_type());
    }
    if crc8(&data[6..8]) != data[8] {
        return SensorReading::empty(sensor_type());
    }

    let dp = i16::from_be_bytes([data[0], data[1]]) as f64;
    let scale_factor = i16::from_be_bytes([data[6], data[7]]) as f64;

    if scale_factor == 0.0 {
        return SensorReading::empty(sensor_type());
    }

    let pressure = dp / scale_factor;
    SensorReading::new(sensor_type(), vec![pressure], vec!["pascal"])
}
```

`from_uart_payload`:
```rust
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 4 {
        return SensorReading::empty(sensor_type());
    }
    let pa = f32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64;
    SensorReading::new(sensor_type(), vec![pa], vec!["pascal"])
}
```

- [ ] **Step 8: Update lis2duxs12 (Acceleration) — add labels `["x_g", "y_g", "z_g", "magnitude_g"]`**

In `bravepi-adapter/sensors/src/lis2duxs12.rs`:

`from_i2c_raw`:
```rust
pub fn from_i2c_raw(data: &[u8; 6]) -> SensorReading {
    let x = i16::from_le_bytes([data[0], data[1]]) as f64 * MG_SCALE;
    let y = i16::from_le_bytes([data[2], data[3]]) as f64 * MG_SCALE;
    let z = i16::from_le_bytes([data[4], data[5]]) as f64 * MG_SCALE;
    let mag = magnitude(x, y, z);
    SensorReading::new(sensor_type(), vec![x / 1000.0, y / 1000.0, z / 1000.0, mag], vec!["x_g", "y_g", "z_g", "magnitude_g"])
}
```

`from_uart_payload`:
```rust
pub fn from_uart_payload(data: &[u8]) -> SensorReading {
    if data.len() < 12 {
        return SensorReading::empty(sensor_type());
    }
    let x = f32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64;
    let y = f32::from_le_bytes([data[4], data[5], data[6], data[7]]) as f64;
    let z = f32::from_le_bytes([data[8], data[9], data[10], data[11]]) as f64;
    let mag = magnitude(x, y, z);
    SensorReading::new(sensor_type(), vec![x / 1000.0, y / 1000.0, z / 1000.0, mag], vec!["x_g", "y_g", "z_g", "magnitude_g"])
}
```

- [ ] **Step 9: Update ContactInput / ContactOutput in convert.rs — labels は空**

In `bravepi-adapter/src/task/convert.rs`, change the ContactInput/ContactOutput arm:

```rust
SensorType::ContactInput | SensorType::ContactOutput => {
    let values: Vec<f64> = s
        .value_data
        .iter()
        .take(s.data_count as usize)
        .map(|&b| if b != 0 { 1.0 } else { 0.0 })
        .collect();
    (SensorReading::new(sensor_type.clone(), values, vec![]), None)
}
```

- [ ] **Step 10: Run all tests**

Run: `cargo test --workspace 2>&1`
Expected: All tests pass. Sensor module tests use `reading.values[N]` comparisons which still work. The `labels` field is not asserted in existing tests — that's fine, no existing test needs to change.

- [ ] **Step 11: Commit**

```bash
git add core/types/src/lib.rs bravepi-adapter/sensors/src/*.rs bravepi-adapter/src/task/convert.rs
git commit -m "feat(types): add labels field to SensorReading

Each sensor module now provides static labels describing its value channels
(e.g. [\"celsius\"], [\"x_g\", \"y_g\", \"z_g\", \"magnitude_g\"]).
ContactInput/ContactOutput use empty labels as their data_count is dynamic."
```

---

### Task 2: Codec フレームサイズ上限 (Critical)

**Files:**
- Modify: `bravepi-adapter/codec/src/codec.rs:83-95`
- Modify: `bravepi-adapter/codec/tests/codec_test.rs`

- [ ] **Step 1: Write the failing test**

In `bravepi-adapter/codec/tests/codec_test.rs`, add:

```rust
#[test]
fn decode_rejects_oversized_frame() {
    let mut codec = BravePiCodec::new();
    // payload_len = 5000 (exceeds MAX_FRAME_SIZE of 4096)
    // frame_len = 2 + 12 + 5000 = 5014
    let payload_len: u16 = 5000;
    let mut frame = Vec::new();
    frame.extend_from_slice(&payload_len.to_le_bytes());
    // Fill enough bytes for the codec to read the header
    frame.extend(vec![0u8; 12 + 5000]);
    codec.feed(&frame);
    match codec.decode() {
        Some(BravePiFrame::DecodeError { reason, .. }) => {
            assert!(reason.contains("frame size exceeds maximum"), "reason was: {}", reason);
        }
        other => panic!("expected DecodeError, got {:?}", other),
    }
    // Buffer should be cleared — next decode returns None
    assert!(codec.decode().is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p bravepi-codec --test codec_test decode_rejects_oversized_frame 2>&1`
Expected: FAIL — currently the codec will try to decode the oversized frame as a normal sensor frame.

- [ ] **Step 3: Implement MAX_FRAME_SIZE check in decode()**

In `bravepi-adapter/codec/src/codec.rs`, add the constant after `HEADER_SIZE`:

```rust
const MAX_FRAME_SIZE: usize = 4096;
```

In the `decode()` method, after computing `frame_len` (line 90), before the `if self.buf.len() < frame_len` check, add:

```rust
    pub fn decode(&mut self) -> Option<BravePiFrame> {
        loop {
            if self.buf.len() < 2 {
                return None;
            }

            let payload_len = u16::from_le_bytes([self.buf[0], self.buf[1]]) as usize;
            let frame_len = 2 + POST_LENGTH_HEADER + payload_len;

            // フレームサイズ上限チェック
            if frame_len > MAX_FRAME_SIZE {
                self.buf.clear();
                self.continuation = None;
                return Some(BravePiFrame::DecodeError {
                    device_number: "unknown".to_string(),
                    sensor_type_raw: 0,
                    reason: format!(
                        "frame size exceeds maximum: {} > {}",
                        frame_len, MAX_FRAME_SIZE
                    ),
                });
            }

            if self.buf.len() < frame_len {
                return None;
            }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p bravepi-codec --test codec_test decode_rejects_oversized_frame 2>&1`
Expected: PASS

- [ ] **Step 5: Run all codec tests**

Run: `cargo test -p bravepi-codec 2>&1`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add bravepi-adapter/codec/src/codec.rs bravepi-adapter/codec/tests/codec_test.rs
git commit -m "fix(codec): add MAX_FRAME_SIZE (4096) limit to prevent unbounded buffer growth

Oversized frames now return DecodeError and clear the buffer/continuation state."
```

---

### Task 3: hex_to_device_bytes エラーハンドリング (Critical)

**Files:**
- Modify: `bravepi-adapter/codec/src/codec.rs:141-174`
- Modify: `bravepi-adapter/codec/tests/codec_test.rs`

- [ ] **Step 1: Write the failing test**

In `bravepi-adapter/codec/tests/codec_test.rs`, add:

```rust
#[test]
fn encode_downlink_invalid_hex_returns_error() {
    let result = BravePiCodec::encode_downlink(
        "not_valid_hex",
        &DownlinkCommand::ParameterGet,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid device number hex"));
}

#[test]
fn encode_downlink_valid_hex_returns_ok() {
    let result = BravePiCodec::encode_downlink(
        "246880020140018b",
        &DownlinkCommand::ParameterGet,
    );
    assert!(result.is_ok());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p bravepi-codec --test codec_test encode_downlink_invalid 2>&1`
Expected: FAIL — `encode_downlink` currently returns `Vec<u8>`, not `Result`.

- [ ] **Step 3: Change hex_to_device_bytes to return Result**

In `bravepi-adapter/codec/src/codec.rs`, change `hex_to_device_bytes`:

```rust
    fn hex_to_device_bytes(hex: &str) -> Result<[u8; 8], String> {
        let val = u64::from_str_radix(hex, 16)
            .map_err(|e| format!("Invalid device number hex '{}': {}", hex, e))?;
        let le = val.to_le_bytes();
        Ok([le[7], le[6], le[5], le[4], le[3], le[2], le[1], le[0]])
    }
```

- [ ] **Step 4: Change encode_downlink to return Result**

In `bravepi-adapter/codec/src/codec.rs`, change `encode_downlink`:

```rust
    pub fn encode_downlink(device_number_hex: &str, cmd: &DownlinkCommand) -> Result<Vec<u8>, String> {
        let device_bytes = Self::hex_to_device_bytes(device_number_hex)?;

        let (opcode, cmd_data, sensor_type_bytes) = match cmd {
            DownlinkCommand::ImmediateUplink { sensor_type } => {
                (0x00u8, vec![], sensor_type.to_le_bytes())
            }
            DownlinkCommand::ParameterGet => {
                (0x0D, vec![0x00], [0x00, 0x00])
            }
            DownlinkCommand::ContactOutput { signal_mode, signal_out_time } => {
                let mut data = vec![*signal_mode];
                data.extend_from_slice(&signal_out_time.to_le_bytes());
                (0x11, data, [0x00, 0x00])
            }
        };

        let length = (12 + cmd_data.len()) as u16;
        let mut frame = Vec::new();
        frame.push(0x00);
        frame.extend_from_slice(&length.to_le_bytes());
        frame.extend_from_slice(&device_bytes);
        frame.extend_from_slice(&sensor_type_bytes);
        frame.push(opcode);
        frame.push(0x00);
        frame.extend_from_slice(&cmd_data);
        Ok(frame)
    }
```

- [ ] **Step 5: Update existing encode tests to unwrap Result**

In `bravepi-adapter/codec/tests/codec_test.rs`, update the 3 existing encode tests:

`encode_immediate_uplink`:
```rust
#[test]
fn encode_immediate_uplink() {
    let f = BravePiCodec::encode_downlink("246880020140018b", &DownlinkCommand::ImmediateUplink { sensor_type: 261 }).unwrap();
    assert_eq!(f[0], 0x00);
    assert_eq!(f[13], 0x00);
    assert_eq!(u16::from_le_bytes([f[11], f[12]]), 261);
}
```

`encode_parameter_get`:
```rust
#[test]
fn encode_parameter_get() {
    let f = BravePiCodec::encode_downlink("246880020140018b", &DownlinkCommand::ParameterGet).unwrap();
    assert_eq!(f[13], 0x0D);
}
```

`encode_contact_output`:
```rust
#[test]
fn encode_contact_output() {
    let f = BravePiCodec::encode_downlink("246880020140018b", &DownlinkCommand::ContactOutput { signal_mode: 1, signal_out_time: 5000 }).unwrap();
    assert_eq!(f[13], 0x11);
    assert_eq!(f[15], 1);
    assert_eq!(u16::from_le_bytes([f[16], f[17]]), 5000);
}
```

- [ ] **Step 6: Run all codec tests**

Run: `cargo test -p bravepi-codec 2>&1`
Expected: All tests pass (including new error-handling tests).

- [ ] **Step 7: Commit**

```bash
git add bravepi-adapter/codec/src/codec.rs bravepi-adapter/codec/tests/codec_test.rs
git commit -m "fix(codec): return Result from encode_downlink for proper error handling

hex_to_device_bytes now returns Result instead of silently using unwrap_or(0)."
```

---

### Task 4: BravePiFrame 等の derive 追加 (Important)

**Files:**
- Modify: `bravepi-adapter/codec/src/codec.rs:5-50`

- [ ] **Step 1: Write the test**

In `bravepi-adapter/codec/tests/codec_test.rs`, add:

```rust
#[test]
fn sensor_frame_clone_and_eq() {
    let frame = SensorFrame {
        device_number: "test".to_string(),
        sensor_type_raw: 261,
        rssi: -60,
        battery: 95,
        data_count: 1,
        value_data: vec![0x00],
    };
    let cloned = frame.clone();
    assert_eq!(frame, cloned);
}

#[test]
fn bravepi_frame_clone_and_eq() {
    let frame = BravePiFrame::DecodeError {
        device_number: "test".to_string(),
        sensor_type_raw: 0,
        reason: "test".to_string(),
    };
    let cloned = frame.clone();
    assert_eq!(frame, cloned);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p bravepi-codec --test codec_test sensor_frame_clone 2>&1`
Expected: FAIL — `Clone` and `PartialEq` not derived on `SensorFrame`.

- [ ] **Step 3: Add Clone + PartialEq derives to all codec types**

In `bravepi-adapter/codec/src/codec.rs`, change the derives:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum BravePiFrame {
```

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct SensorFrame {
```

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigFrame {
```

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum DownlinkCommand {
```

- [ ] **Step 4: Run all tests**

Run: `cargo test --workspace 2>&1`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add bravepi-adapter/codec/src/codec.rs bravepi-adapter/codec/tests/codec_test.rs
git commit -m "feat(codec): add Clone + PartialEq derives to all public codec types"
```

---

### Task 5: BravePiCodec に Default 追加 (Important)

**Files:**
- Modify: `bravepi-adapter/codec/src/codec.rs`
- Modify: `bravepi-adapter/codec/tests/codec_test.rs`

- [ ] **Step 1: Write the test**

In `bravepi-adapter/codec/tests/codec_test.rs`, add:

```rust
#[test]
fn codec_default_works() {
    let codec = BravePiCodec::default();
    // Default should behave identically to new()
    let mut codec = codec;
    codec.feed(&[]);
    assert!(codec.decode().is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p bravepi-codec --test codec_test codec_default_works 2>&1`
Expected: FAIL — `Default` not implemented for `BravePiCodec`.

- [ ] **Step 3: Add Default impl**

In `bravepi-adapter/codec/src/codec.rs`, after the `impl BravePiCodec` block (after the closing `}`), add:

```rust
impl Default for BravePiCodec {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: Run all tests**

Run: `cargo test -p bravepi-codec 2>&1`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add bravepi-adapter/codec/src/codec.rs bravepi-adapter/codec/tests/codec_test.rs
git commit -m "feat(codec): add Default impl for BravePiCodec"
```

---

### Task 6: codec モジュールパスの冗長性解消 (Important)

**Files:**
- Modify: `bravepi-adapter/codec/src/lib.rs`
- Modify: `bravepi-adapter/src/task/event_loop.rs:6`
- Modify: `bravepi-adapter/src/task/convert.rs:7`
- Modify: `bravepi-adapter/tests/frame_to_event_test.rs:2`
- Modify: `bravepi-adapter/codec/tests/codec_test.rs:1`

- [ ] **Step 1: Add re-exports to codec lib.rs**

In `bravepi-adapter/codec/src/lib.rs`, replace:

```rust
pub mod codec;
```

with:

```rust
pub mod codec;

pub use codec::{BravePiCodec, BravePiFrame, SensorFrame, ConfigFrame, DownlinkCommand};
```

- [ ] **Step 2: Update imports in event_loop.rs**

In `bravepi-adapter/src/task/event_loop.rs`, change:

```rust
use bravepi_codec::codec::BravePiCodec;
```

to:

```rust
use bravepi_codec::BravePiCodec;
```

- [ ] **Step 3: Update imports in convert.rs**

In `bravepi-adapter/src/task/convert.rs`, change:

```rust
use bravepi_codec::codec::BravePiFrame;
```

to:

```rust
use bravepi_codec::BravePiFrame;
```

- [ ] **Step 4: Update imports in frame_to_event_test.rs**

In `bravepi-adapter/tests/frame_to_event_test.rs`, change:

```rust
use bravepi_codec::codec::{BravePiFrame, ConfigFrame, SensorFrame};
```

to:

```rust
use bravepi_codec::{BravePiFrame, ConfigFrame, SensorFrame};
```

- [ ] **Step 5: Update imports in codec_test.rs**

In `bravepi-adapter/codec/tests/codec_test.rs`, change:

```rust
use bravepi_codec::codec::*;
```

to:

```rust
use bravepi_codec::*;
```

- [ ] **Step 6: Run all tests**

Run: `cargo test --workspace 2>&1`
Expected: All tests pass. Both `bravepi_codec::BravePiCodec` and `bravepi_codec::codec::BravePiCodec` paths work, but all imports now use the shorter form.

- [ ] **Step 7: Commit**

```bash
git add bravepi-adapter/codec/src/lib.rs bravepi-adapter/src/task/event_loop.rs bravepi-adapter/src/task/convert.rs bravepi-adapter/tests/frame_to_event_test.rs bravepi-adapter/codec/tests/codec_test.rs
git commit -m "refactor(codec): re-export public types from crate root to eliminate stuttering paths

bravepi_codec::codec::BravePiCodec → bravepi_codec::BravePiCodec"
```

---

### Task 7: AdapterId / DeviceKey の newtype 強化 (Important)

**Files:**
- Modify: `core/types/src/lib.rs:108-125`
- Modify: `bravepi-adapter/src/task/handle.rs:53`
- Modify: `bravepi-adapter/src/task/convert.rs:22,95`
- Modify: `bravepi-adapter/tests/frame_to_event_test.rs:34,150`
- Modify: `bravepi-adapter/tests/event_loop_test.rs:94,104`

- [ ] **Step 1: Write the test — verify .0 field is not accessible**

This is a compile-time guarantee. Instead, write a test that uses the new API:

In `bravepi-adapter/tests/frame_to_event_test.rs`, the tests currently use `device_key.0` — we'll update them in Step 5. First, change the core types.

- [ ] **Step 2: Make AdapterId and DeviceKey fields private, add constructors**

In `core/types/src/lib.rs`, replace the AdapterId and DeviceKey definitions:

```rust
/// adapter の一意識別子。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AdapterId(String);

impl AdapterId {
    pub fn new(id: impl Into<String>) -> Self { Self(id.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
}

/// デバイスの一意キー。adapter 内で一意であればよい。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceKey(String);

impl DeviceKey {
    pub fn new(key: impl Into<String>) -> Self { Self(key.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
}
```

Keep the existing `Display` impls unchanged (they already use `self.0` internally).

- [ ] **Step 3: Run cargo check to see all compilation errors**

Run: `cargo check --workspace 2>&1 | head -50`
Expected: Errors in handle.rs, convert.rs, frame_to_event_test.rs, event_loop_test.rs where `AdapterId(...)` and `DeviceKey(...)` tuple constructors or `.0` field access are used.

- [ ] **Step 4: Update handle.rs — use AdapterId::new()**

In `bravepi-adapter/src/task/handle.rs`, change:

```rust
    let id = AdapterId(format!("bravepi:{}", port_path));
```

to:

```rust
    let id = AdapterId::new(format!("bravepi:{}", port_path));
```

- [ ] **Step 5: Update convert.rs — use DeviceKey::new()**

In `bravepi-adapter/src/task/convert.rs`, change line 22:

```rust
            let device_key = DeviceKey(s.device_number.clone());
```

to:

```rust
            let device_key = DeviceKey::new(s.device_number.clone());
```

And line 95:

```rust
                device_key: Some(DeviceKey(device_number)),
```

to:

```rust
                device_key: Some(DeviceKey::new(device_number)),
```

- [ ] **Step 6: Update frame_to_event_test.rs — use .as_str()**

In `bravepi-adapter/tests/frame_to_event_test.rs`, change:

Line 34: `assert_eq!(device_key.0, "246880020140018b");` → `assert_eq!(device_key.as_str(), "246880020140018b");`

Line 150: `assert_eq!(device_key.unwrap().0, "bad_device");` → `assert_eq!(device_key.unwrap().as_str(), "bad_device");`

- [ ] **Step 7: Update event_loop_test.rs — use .as_str()**

In `bravepi-adapter/tests/event_loop_test.rs`, change:

Line 94: `assert_eq!(device_key.0, "246880020140018b");` → `assert_eq!(device_key.as_str(), "246880020140018b");`

Line 104: `assert_eq!(device_key.0, "246880020140018b");` → `assert_eq!(device_key.as_str(), "246880020140018b");`

- [ ] **Step 8: Run all tests**

Run: `cargo test --workspace 2>&1`
Expected: All tests pass.

- [ ] **Step 9: Commit**

```bash
git add core/types/src/lib.rs bravepi-adapter/src/task/handle.rs bravepi-adapter/src/task/convert.rs bravepi-adapter/tests/frame_to_event_test.rs bravepi-adapter/tests/event_loop_test.rs
git commit -m "refactor(types): make AdapterId/DeviceKey fields private with new()/as_str() accessors

Prevents accidental construction from raw strings without going through the newtype API."
```

---

### Task 8: doc comment の言語統一 (Important)

**Files:**
- Modify: `bravepi-adapter/src/task/handle.rs:20`
- Modify: `bravepi-adapter/src/task/reader.rs:1-2`

- [ ] **Step 1: Update handle.rs doc comment**

In `bravepi-adapter/src/task/handle.rs`, change line 20:

```rust
    /// Send Shutdown command and wait for the reader thread to exit.
```

to:

```rust
    /// シャットダウンコマンドを送信し、reader スレッドの終了を待つ。
```

- [ ] **Step 2: Update reader.rs module doc**

In `bravepi-adapter/src/task/reader.rs`, change lines 1-2:

```rust
//! 専用スレッド: serial port から読んで bytes channel に送る。
//! エラー時は exponential backoff で再接続を試みる。
```

These are already in Japanese. Check for any remaining English comments in the adapter task modules.

- [ ] **Step 3: Scan for remaining English comments in adapter task modules**

Run: `cargo doc -p bravepi-adapter --no-deps 2>&1 | tail -5`
Expected: No warnings. Doc generation succeeds.

- [ ] **Step 4: Run all tests**

Run: `cargo test --workspace 2>&1`
Expected: All tests pass (doc changes are non-functional).

- [ ] **Step 5: Commit**

```bash
git add bravepi-adapter/src/task/handle.rs
git commit -m "docs(adapter): unify doc comments to Japanese in adapter task modules"
```

---

## Execution Order

Tasks can be executed in this order (each builds on the previous):

1. **Task 1** (SensorReading labels) — changes core type signature, all sensors
2. **Task 2** (MAX_FRAME_SIZE) — codec only, independent
3. **Task 3** (hex_to_device_bytes Result) — codec only, independent
4. **Task 4** (derive additions) — codec only, independent
5. **Task 5** (Default impl) — codec only, independent
6. **Task 6** (re-exports) — changes imports across adapter
7. **Task 7** (newtype strengthening) — changes core types + all usage sites
8. **Task 8** (doc comments) — non-functional, last

Tasks 2-5 are all codec-only and independent of each other, but should be done sequentially to avoid merge conflicts in `codec.rs`.

## Verification

After all tasks are complete:

```bash
cargo test --workspace 2>&1
cargo clippy --workspace 2>&1
```

All 50+ tests should pass with no warnings.
