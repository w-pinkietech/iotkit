# Sensor Registry / Dispatch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate scattered sensor dispatch into a two-layer registry so adding a new sensor only requires one file in sensors/, one registry entry, and one core SensorType variant.

**Architecture:** sensors crate defines `SensorHandler` (descriptor with fn pointers for identity + decode_uart). Each sensor module exports `pub const HANDLER`. adapter crate owns BravePI raw code → handler mapping in `registry.rs`. `convert.rs` becomes thin lookup + event assembly.

**Tech Stack:** Rust, bravepi-sensors crate, bravepi-adapter crate, iotkit-core-types

---

## File Structure

### New files
- `bravepi-adapter/sensors/src/contact.rs` — ContactInput/ContactOutput handler, decode, identity
- `bravepi-adapter/src/registry.rs` — BravePI raw code → SensorHandler lookup table

### Modified files
- `bravepi-adapter/sensors/src/lib.rs` — Add `UartSample`, `SensorHandler`, `pub mod contact`
- `bravepi-adapter/sensors/src/mcp9600.rs` — Add `decode_uart` wrapper + `HANDLER` const
- `bravepi-adapter/sensors/src/opt3001.rs` — Add `decode_uart` wrapper + `HANDLER` const
- `bravepi-adapter/sensors/src/mcp3427.rs` — Add `decode_uart` wrapper + `HANDLER` const
- `bravepi-adapter/sensors/src/vl53l1x.rs` — Add `decode_uart` wrapper + `HANDLER` const
- `bravepi-adapter/sensors/src/sdp810.rs` — Add `decode_uart` wrapper + `HANDLER` const
- `bravepi-adapter/sensors/src/lis2duxs12.rs` — Add `decode_uart` wrapper + `HANDLER` const
- `bravepi-adapter/src/lib.rs` — Add `pub(crate) mod registry`, remove `sensor_type_from_bravepi_raw()`
- `bravepi-adapter/src/task/convert.rs` — Replace match dispatch with registry lookup

### Unchanged files
- `bravepi-adapter/src/task/event_loop.rs`
- `bravepi-adapter/src/task/event_loop_test.rs`
- `bravepi-adapter/src/task/serial_source.rs`
- `bravepi-adapter/src/task/handle.rs`
- `bravepi-adapter/codec/` (all)
- `core/types/src/lib.rs`

---

### Task 1: Define SensorHandler and UartSample in sensors crate

**Files:**
- Modify: `bravepi-adapter/sensors/src/lib.rs`

- [ ] **Step 1: Add UartSample and SensorHandler to sensors/src/lib.rs**

Add the two new types at the top of `lib.rs`, below the existing doc comment and above the module declarations:

```rust
// bravepi-adapter/sensors/src/lib.rs

//! iotkit-sensors: sensor IC ごとの変換ドライバー
//!
//! 入力ソース（I2C 生値 / UART BravePI フレーム）を問わず、
//! 同じセンサー IC なら同じ SensorReading を返す。

use iotkit_core_types::{ConnectionInfo, SensorIdentity, SensorReading, SensorType};

/// UART デコードの入力。payload + data_count を含む。
pub struct UartSample<'a> {
    pub payload: &'a [u8],
    pub data_count: u16,
}

/// センサー/endpoint の decode と identity 生成をまとめた descriptor。
/// 各センサーモジュールが `pub const HANDLER: SensorHandler` として公開する。
pub struct SensorHandler {
    pub sensor_type: SensorType,
    pub key_suffix: &'static str,
    pub identity: fn(ConnectionInfo) -> SensorIdentity,
    pub decode_uart: fn(UartSample<'_>) -> SensorReading,
}

pub mod contact;
pub mod opt3001;
pub mod mcp9600;
pub mod mcp3427;
pub mod vl53l1x;
pub mod sdp810;
pub mod lis2duxs12;
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p bravepi-sensors`

Expected: Compilation error because `contact` module doesn't exist yet. That's expected — we'll create it in Task 2. For now, comment out `pub mod contact;` temporarily to verify the rest compiles.

Actually, to avoid partial state, add a minimal placeholder for contact:

Create `bravepi-adapter/sensors/src/contact.rs`:

```rust
//! ContactInput / ContactOutput endpoint decoder。
//! IC ドライバーではなく、module-level の endpoint として扱う。
```

Run: `cargo check -p bravepi-sensors`

Expected: PASS (no errors)

- [ ] **Step 3: Commit**

```bash
git add bravepi-adapter/sensors/src/lib.rs bravepi-adapter/sensors/src/contact.rs
git commit -m "feat(sensors): add SensorHandler and UartSample types"
```

---

### Task 2: Add HANDLER const to all 6 IC sensor modules

**Files:**
- Modify: `bravepi-adapter/sensors/src/mcp9600.rs`
- Modify: `bravepi-adapter/sensors/src/opt3001.rs`
- Modify: `bravepi-adapter/sensors/src/mcp3427.rs`
- Modify: `bravepi-adapter/sensors/src/vl53l1x.rs`
- Modify: `bravepi-adapter/sensors/src/sdp810.rs`
- Modify: `bravepi-adapter/sensors/src/lis2duxs12.rs`

Each module gets a named `decode_uart` wrapper function and a `pub const HANDLER`. The wrapper delegates to the existing `from_uart_payload`, passing only `sample.payload`. Existing public API is unchanged.

- [ ] **Step 1: Add HANDLER to mcp9600.rs**

Add the import for `UartSample` and the two new items after the existing `from_uart_payload` function, before the `#[cfg(test)]` block:

```rust
use crate::UartSample;

fn decode_uart(sample: UartSample<'_>) -> SensorReading {
    from_uart_payload(sample.payload)
}

pub const HANDLER: crate::SensorHandler = crate::SensorHandler {
    sensor_type: SensorType::Temperature,
    key_suffix: "temperature",
    identity: identity,
    decode_uart: decode_uart,
};
```

Note: `SensorType::Temperature` is usable in const context because `SensorType` derives `Clone` but the field just needs to be a valid const expression. Since `SensorType::Temperature` is a unit variant, it works in const.

- [ ] **Step 2: Add HANDLER to opt3001.rs**

```rust
use crate::UartSample;

fn decode_uart(sample: UartSample<'_>) -> SensorReading {
    from_uart_payload(sample.payload)
}

pub const HANDLER: crate::SensorHandler = crate::SensorHandler {
    sensor_type: SensorType::Illuminance,
    key_suffix: "illuminance",
    identity: identity,
    decode_uart: decode_uart,
};
```

- [ ] **Step 3: Add HANDLER to mcp3427.rs**

```rust
use crate::UartSample;

fn decode_uart(sample: UartSample<'_>) -> SensorReading {
    from_uart_payload(sample.payload)
}

pub const HANDLER: crate::SensorHandler = crate::SensorHandler {
    sensor_type: SensorType::Adc,
    key_suffix: "adc",
    identity: identity,
    decode_uart: decode_uart,
};
```

- [ ] **Step 4: Add HANDLER to vl53l1x.rs**

```rust
use crate::UartSample;

fn decode_uart(sample: UartSample<'_>) -> SensorReading {
    from_uart_payload(sample.payload)
}

pub const HANDLER: crate::SensorHandler = crate::SensorHandler {
    sensor_type: SensorType::Ranging,
    key_suffix: "ranging",
    identity: identity,
    decode_uart: decode_uart,
};
```

- [ ] **Step 5: Add HANDLER to sdp810.rs**

```rust
use crate::UartSample;

fn decode_uart(sample: UartSample<'_>) -> SensorReading {
    from_uart_payload(sample.payload)
}

pub const HANDLER: crate::SensorHandler = crate::SensorHandler {
    sensor_type: SensorType::DifferentialPressure,
    key_suffix: "differential_pressure",
    identity: identity,
    decode_uart: decode_uart,
};
```

- [ ] **Step 6: Add HANDLER to lis2duxs12.rs**

```rust
use crate::UartSample;

fn decode_uart(sample: UartSample<'_>) -> SensorReading {
    from_uart_payload(sample.payload)
}

pub const HANDLER: crate::SensorHandler = crate::SensorHandler {
    sensor_type: SensorType::Acceleration,
    key_suffix: "acceleration",
    identity: identity,
    decode_uart: decode_uart,
};
```

- [ ] **Step 7: Verify all modules compile and existing tests still pass**

Run: `cargo test -p bravepi-sensors`

Expected: All existing tests pass (i2c, uart, both_sources, etc.). No new tests needed — HANDLER is a const struct wiring existing functions.

- [ ] **Step 8: Commit**

```bash
git add bravepi-adapter/sensors/src/mcp9600.rs bravepi-adapter/sensors/src/opt3001.rs bravepi-adapter/sensors/src/mcp3427.rs bravepi-adapter/sensors/src/vl53l1x.rs bravepi-adapter/sensors/src/sdp810.rs bravepi-adapter/sensors/src/lis2duxs12.rs
git commit -m "feat(sensors): add HANDLER const to all 6 IC sensor modules"
```

---

### Task 3: Implement contact.rs with handlers and tests

**Files:**
- Modify: `bravepi-adapter/sensors/src/contact.rs`

This task moves the inline ContactInput/ContactOutput decode logic and identity from `convert.rs` into the sensors crate as a proper handler module.

- [ ] **Step 1: Write tests for contact decode and identity**

Replace the placeholder `contact.rs` with full implementation including tests:

```rust
//! ContactInput / ContactOutput endpoint decoder。
//! IC ドライバーではなく、module-level の endpoint として扱う。

use iotkit_core_types::{ConnectionInfo, SensorIdentity, SensorReading, SensorType};

use crate::{SensorHandler, UartSample};

fn decode_contact(sample: UartSample<'_>) -> SensorReading {
    let values: Vec<f64> = sample
        .payload
        .iter()
        .take(sample.data_count as usize)
        .map(|&b| if b != 0 { 1.0 } else { 0.0 })
        .collect();
    // ContactInput と ContactOutput は同じ decode ロジック。
    // sensor_type は HANDLER 側で決まるため、ここでは ContactInput を使う。
    // SensorReading の sensor_type は呼び出し元が HANDLER.sensor_type で上書きするわけではなく
    // そのまま使われるが、decode_contact は両方の HANDLER から呼ばれるため、
    // 各 HANDLER の sensor_type と一致させる必要がある。
    // → 分離: contact_input 用と contact_output 用の decode を個別に用意する。
    SensorReading::new(SensorType::ContactInput, values, vec![])
}

fn decode_contact_input(sample: UartSample<'_>) -> SensorReading {
    let values: Vec<f64> = sample
        .payload
        .iter()
        .take(sample.data_count as usize)
        .map(|&b| if b != 0 { 1.0 } else { 0.0 })
        .collect();
    SensorReading::new(SensorType::ContactInput, values, vec![])
}

fn decode_contact_output(sample: UartSample<'_>) -> SensorReading {
    let values: Vec<f64> = sample
        .payload
        .iter()
        .take(sample.data_count as usize)
        .map(|&b| if b != 0 { 1.0 } else { 0.0 })
        .collect();
    SensorReading::new(SensorType::ContactOutput, values, vec![])
}

fn contact_input_identity(connection: ConnectionInfo) -> SensorIdentity {
    SensorIdentity {
        manufacturer: "Braveridge".to_string(),
        ic_part_number: "Contact Input Module".to_string(),
        sensor_type: SensorType::ContactInput,
        connection,
    }
}

fn contact_output_identity(connection: ConnectionInfo) -> SensorIdentity {
    SensorIdentity {
        manufacturer: "Braveridge".to_string(),
        ic_part_number: "Contact Output Module".to_string(),
        sensor_type: SensorType::ContactOutput,
        connection,
    }
}

pub const CONTACT_INPUT: SensorHandler = SensorHandler {
    sensor_type: SensorType::ContactInput,
    key_suffix: "contact_input",
    identity: contact_input_identity,
    decode_uart: decode_contact_input,
};

pub const CONTACT_OUTPUT: SensorHandler = SensorHandler {
    sensor_type: SensorType::ContactOutput,
    key_suffix: "contact_output",
    identity: contact_output_identity,
    decode_uart: decode_contact_output,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use iotkit_core_types::ConnectionKind;

    fn test_conn() -> ConnectionInfo {
        ConnectionInfo {
            kind: ConnectionKind::Uart,
            parameters: BTreeMap::from([
                ("port".into(), "/dev/test".into()),
                ("transmitter_id".into(), "test123".into()),
            ]),
        }
    }

    #[test]
    fn contact_input_decode_maps_bytes_to_float() {
        let sample = UartSample {
            payload: &[0x01, 0x00, 0x01, 0xFF],
            data_count: 3,
        };
        let reading = decode_contact_input(sample);
        assert_eq!(reading.sensor_type, SensorType::ContactInput);
        assert_eq!(reading.values, vec![1.0, 0.0, 1.0]);
    }

    #[test]
    fn contact_output_decode_maps_bytes_to_float() {
        let sample = UartSample {
            payload: &[0x00, 0x01],
            data_count: 2,
        };
        let reading = decode_contact_output(sample);
        assert_eq!(reading.sensor_type, SensorType::ContactOutput);
        assert_eq!(reading.values, vec![0.0, 1.0]);
    }

    #[test]
    fn data_count_limits_values() {
        let sample = UartSample {
            payload: &[0x01, 0x00, 0x01],
            data_count: 2,
        };
        let reading = decode_contact_input(sample);
        assert_eq!(reading.values.len(), 2);
    }

    #[test]
    fn data_count_exceeds_payload_does_not_panic() {
        let sample = UartSample {
            payload: &[0x01, 0x00],
            data_count: 100,
        };
        let reading = decode_contact_input(sample);
        assert_eq!(reading.values.len(), 2);
    }

    #[test]
    fn contact_input_identity_is_correct() {
        let id = contact_input_identity(test_conn());
        assert_eq!(id.manufacturer, "Braveridge");
        assert_eq!(id.ic_part_number, "Contact Input Module");
        assert_eq!(id.sensor_type, SensorType::ContactInput);
        assert_eq!(id.connection.kind, ConnectionKind::Uart);
    }

    #[test]
    fn contact_output_identity_is_correct() {
        let id = contact_output_identity(test_conn());
        assert_eq!(id.manufacturer, "Braveridge");
        assert_eq!(id.ic_part_number, "Contact Output Module");
        assert_eq!(id.sensor_type, SensorType::ContactOutput);
    }
}
```

Wait — the above has a dead `decode_contact` function. Let me clean that up. The actual file should be:

```rust
//! ContactInput / ContactOutput endpoint decoder。
//! IC ドライバーではなく、module-level の endpoint として扱う。

use iotkit_core_types::{ConnectionInfo, SensorIdentity, SensorReading, SensorType};

use crate::{SensorHandler, UartSample};

fn decode_values(sample: &UartSample<'_>) -> Vec<f64> {
    sample
        .payload
        .iter()
        .take(sample.data_count as usize)
        .map(|&b| if b != 0 { 1.0 } else { 0.0 })
        .collect()
}

fn decode_contact_input(sample: UartSample<'_>) -> SensorReading {
    SensorReading::new(SensorType::ContactInput, decode_values(&sample), vec![])
}

fn decode_contact_output(sample: UartSample<'_>) -> SensorReading {
    SensorReading::new(SensorType::ContactOutput, decode_values(&sample), vec![])
}

fn contact_input_identity(connection: ConnectionInfo) -> SensorIdentity {
    SensorIdentity {
        manufacturer: "Braveridge".to_string(),
        ic_part_number: "Contact Input Module".to_string(),
        sensor_type: SensorType::ContactInput,
        connection,
    }
}

fn contact_output_identity(connection: ConnectionInfo) -> SensorIdentity {
    SensorIdentity {
        manufacturer: "Braveridge".to_string(),
        ic_part_number: "Contact Output Module".to_string(),
        sensor_type: SensorType::ContactOutput,
        connection,
    }
}

pub const CONTACT_INPUT: SensorHandler = SensorHandler {
    sensor_type: SensorType::ContactInput,
    key_suffix: "contact_input",
    identity: contact_input_identity,
    decode_uart: decode_contact_input,
};

pub const CONTACT_OUTPUT: SensorHandler = SensorHandler {
    sensor_type: SensorType::ContactOutput,
    key_suffix: "contact_output",
    identity: contact_output_identity,
    decode_uart: decode_contact_output,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use iotkit_core_types::ConnectionKind;

    fn test_conn() -> ConnectionInfo {
        ConnectionInfo {
            kind: ConnectionKind::Uart,
            parameters: BTreeMap::from([
                ("port".into(), "/dev/test".into()),
                ("transmitter_id".into(), "test123".into()),
            ]),
        }
    }

    #[test]
    fn contact_input_decode_maps_bytes_to_float() {
        let sample = UartSample {
            payload: &[0x01, 0x00, 0x01, 0xFF],
            data_count: 3,
        };
        let reading = decode_contact_input(sample);
        assert_eq!(reading.sensor_type, SensorType::ContactInput);
        assert_eq!(reading.values, vec![1.0, 0.0, 1.0]);
    }

    #[test]
    fn contact_output_decode_maps_bytes_to_float() {
        let sample = UartSample {
            payload: &[0x00, 0x01],
            data_count: 2,
        };
        let reading = decode_contact_output(sample);
        assert_eq!(reading.sensor_type, SensorType::ContactOutput);
        assert_eq!(reading.values, vec![0.0, 1.0]);
    }

    #[test]
    fn data_count_limits_values() {
        let sample = UartSample {
            payload: &[0x01, 0x00, 0x01],
            data_count: 2,
        };
        let reading = decode_contact_input(sample);
        assert_eq!(reading.values.len(), 2);
    }

    #[test]
    fn data_count_exceeds_payload_does_not_panic() {
        let sample = UartSample {
            payload: &[0x01, 0x00],
            data_count: 100,
        };
        let reading = decode_contact_input(sample);
        assert_eq!(reading.values.len(), 2);
    }

    #[test]
    fn contact_input_identity_is_correct() {
        let id = contact_input_identity(test_conn());
        assert_eq!(id.manufacturer, "Braveridge");
        assert_eq!(id.ic_part_number, "Contact Input Module");
        assert_eq!(id.sensor_type, SensorType::ContactInput);
        assert_eq!(id.connection.kind, ConnectionKind::Uart);
    }

    #[test]
    fn contact_output_identity_is_correct() {
        let id = contact_output_identity(test_conn());
        assert_eq!(id.manufacturer, "Braveridge");
        assert_eq!(id.ic_part_number, "Contact Output Module");
        assert_eq!(id.sensor_type, SensorType::ContactOutput);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p bravepi-sensors`

Expected: All tests pass (existing 6 modules + new contact tests)

- [ ] **Step 3: Commit**

```bash
git add bravepi-adapter/sensors/src/contact.rs
git commit -m "feat(sensors): add contact.rs with ContactInput/ContactOutput handlers"
```

---

### Task 4: Create registry.rs in adapter crate

**Files:**
- Create: `bravepi-adapter/src/registry.rs`
- Modify: `bravepi-adapter/src/lib.rs`

- [ ] **Step 1: Write registry.rs with lookup test**

Create `bravepi-adapter/src/registry.rs`:

```rust
//! BravePI raw sensor_type → SensorHandler の対応表。
//! BravePI プロトコル固有の番号体系はこのモジュールに閉じる。

use bravepi_sensors::SensorHandler;

struct RegistryEntry {
    raw_sensor_type: u16,
    handler: &'static SensorHandler,
}

static REGISTRY: &[RegistryEntry] = &[
    RegistryEntry { raw_sensor_type: 257, handler: &bravepi_sensors::contact::CONTACT_INPUT },
    RegistryEntry { raw_sensor_type: 258, handler: &bravepi_sensors::contact::CONTACT_OUTPUT },
    RegistryEntry { raw_sensor_type: 259, handler: &bravepi_sensors::mcp3427::HANDLER },
    RegistryEntry { raw_sensor_type: 260, handler: &bravepi_sensors::vl53l1x::HANDLER },
    RegistryEntry { raw_sensor_type: 261, handler: &bravepi_sensors::mcp9600::HANDLER },
    RegistryEntry { raw_sensor_type: 262, handler: &bravepi_sensors::lis2duxs12::HANDLER },
    RegistryEntry { raw_sensor_type: 263, handler: &bravepi_sensors::sdp810::HANDLER },
    RegistryEntry { raw_sensor_type: 264, handler: &bravepi_sensors::opt3001::HANDLER },
];

pub(crate) fn lookup_handler(raw: u16) -> Option<&'static SensorHandler> {
    REGISTRY.iter()
        .find(|e| e.raw_sensor_type == raw)
        .map(|e| e.handler)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_types::SensorType;

    #[test]
    fn all_known_raw_codes_resolve() {
        let expected = [
            (257, "contact_input"),
            (258, "contact_output"),
            (259, "adc"),
            (260, "ranging"),
            (261, "temperature"),
            (262, "acceleration"),
            (263, "differential_pressure"),
            (264, "illuminance"),
        ];
        for (raw, suffix) in expected {
            let handler = lookup_handler(raw)
                .unwrap_or_else(|| panic!("raw {} should resolve", raw));
            assert_eq!(handler.key_suffix, suffix, "raw {} suffix mismatch", raw);
        }
    }

    #[test]
    fn unknown_raw_code_returns_none() {
        assert!(lookup_handler(0).is_none());
        assert!(lookup_handler(9999).is_none());
    }

    #[test]
    fn handler_sensor_types_are_correct() {
        assert_eq!(lookup_handler(261).unwrap().sensor_type, SensorType::Temperature);
        assert_eq!(lookup_handler(257).unwrap().sensor_type, SensorType::ContactInput);
        assert_eq!(lookup_handler(258).unwrap().sensor_type, SensorType::ContactOutput);
    }
}
```

- [ ] **Step 2: Wire registry module in lib.rs and remove sensor_type_from_bravepi_raw**

In `bravepi-adapter/src/lib.rs`:

1. Add `pub(crate) mod registry;` after the existing module declarations
2. Remove the `sensor_type_from_bravepi_raw()` function (lines 67-80) and its doc comment (line 67)
3. Remove `SensorType` from the `use iotkit_core_types` import since it's no longer used in lib.rs

The file should become:

```rust
//! BravePI adapter — BravePI プロトコル固有の処理。
//! rpi4b-driver の transport / sensors を使い、BravePI 特有のマッピングを行う。
//!
//! `task` モジュールで async task として起動し、AdapterEvent channel で core と通信する。

pub mod task;
pub(crate) mod transport;
pub(crate) mod registry;

use std::collections::BTreeMap;
use iotkit_core_types::{ConnectionInfo, ConnectionKind};
use rpi4b_transport::{DataBits, Parity, SerialConfig, StopBits};

/// BravePI adapter 内部の型安全な接続表現。
#[derive(Debug, Clone, PartialEq)]
pub enum BravepiConnection {
    Uart {
        port: String,
        transmitter_id: String,
    },
    I2c {
        bus: String,
        address: u8,
    },
    Gpio {
        pin: u8,
    },
}

impl BravepiConnection {
    /// adapter 固有の型 → core の汎用型に変換。
    pub fn to_connection_info(&self) -> ConnectionInfo {
        match self {
            Self::Uart { port, transmitter_id } => ConnectionInfo {
                kind: ConnectionKind::Uart,
                parameters: BTreeMap::from([
                    ("port".into(), port.clone()),
                    ("transmitter_id".into(), transmitter_id.clone()),
                ]),
            },
            Self::I2c { bus, address } => ConnectionInfo {
                kind: ConnectionKind::I2c,
                parameters: BTreeMap::from([
                    ("bus".into(), bus.clone()),
                    ("address".into(), format!("0x{:02x}", address)),
                ]),
            },
            Self::Gpio { pin } => ConnectionInfo {
                kind: ConnectionKind::Gpio,
                parameters: BTreeMap::from([
                    ("pin".into(), format!("BCM{}", pin)),
                ]),
            },
        }
    }
}

/// BravePI UART 標準設定: 38400 8N1
pub fn serial_config() -> SerialConfig {
    SerialConfig {
        baud_rate: 38400,
        data_bits: DataBits::Eight,
        parity: Parity::None,
        stop_bits: StopBits::One,
    }
}
```

- [ ] **Step 3: Check that convert.rs still compiles**

`convert.rs` currently uses `sensor_type_from_bravepi_raw` — it will fail to compile. That's expected. We'll fix it in Task 5. For now, verify registry.rs tests pass in isolation:

Run: `cargo test -p bravepi-adapter -- registry`

Expected: 3 registry tests pass. Other tests may fail due to missing `sensor_type_from_bravepi_raw` — that's OK.

- [ ] **Step 4: Commit**

```bash
git add bravepi-adapter/src/registry.rs bravepi-adapter/src/lib.rs
git commit -m "feat(bravepi): add registry.rs with BravePI raw code lookup table"
```

---

### Task 5: Rewrite convert.rs to use registry lookup

**Files:**
- Modify: `bravepi-adapter/src/task/convert.rs`
- Test: `bravepi-adapter/src/task/convert_test.rs` (existing — no changes needed, all tests must still pass)

- [ ] **Step 1: Rewrite convert.rs**

Replace the entire contents of `bravepi-adapter/src/task/convert.rs` with:

```rust
//! BravePiFrame → AdapterEvent 変換。純粋関数、状態なし。

use iotkit_core_types::{AdapterEvent, DeviceKey, SensorIdentity};

use bravepi_codec::BravePiFrame;
use bravepi_sensors::UartSample;

use crate::registry::lookup_handler;
use crate::BravepiConnection;

/// BravePiFrame を AdapterEvent に変換する。
/// SensorData フレームの場合は SensorIdentity も返す (DeviceDiscovered 用)。
/// None を返す場合、そのフレームは core に通知する必要がない。
pub(crate) fn frame_to_event(
    frame: BravePiFrame,
    port_path: &str,
) -> Option<(AdapterEvent, Option<SensorIdentity>)> {
    match frame {
        BravePiFrame::Sensor(s) => {
            let handler = lookup_handler(s.sensor_type_raw).or_else(|| {
                tracing::warn!(raw = s.sensor_type_raw, "Unknown sensor type, skipping");
                None
            })?;

            let transmitter_id = s.device_number.clone();
            let device_key = DeviceKey::new(
                format!("bravepi:{}:{}", transmitter_id, handler.key_suffix),
            );

            let conn_info = BravepiConnection::Uart {
                port: port_path.to_string(),
                transmitter_id,
            }
            .to_connection_info();

            let sample = UartSample {
                payload: &s.value_data,
                data_count: s.data_count,
            };
            let reading = (handler.decode_uart)(sample);
            let identity = (handler.identity)(conn_info);

            let event = AdapterEvent::SensorData {
                device_key,
                reading,
                rssi: Some(s.rssi as i16),
                battery_pct: Some(s.battery),
            };

            Some((event, Some(identity)))
        }
        BravePiFrame::Config(cfg) => {
            tracing::info!(
                device = %cfg.device_number,
                sensor_type = cfg.true_sensor_type,
                firmware = %cfg.firmware_version,
                "Config frame received"
            );
            None
        }
        BravePiFrame::DecodeError {
            device_number,
            sensor_type_raw,
            reason,
        } => {
            let device_key = if device_number == "unknown" {
                None
            } else {
                lookup_handler(sensor_type_raw).map(|h| {
                    DeviceKey::new(format!("bravepi:{}:{}", device_number, h.key_suffix))
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
    }
}
```

- [ ] **Step 2: Run all bravepi-adapter tests to verify no regressions**

Run: `cargo test -p bravepi-adapter`

Expected: All 23 existing tests pass. The convert_test.rs and event_loop_test.rs tests verify the same inputs produce the same outputs, even though the dispatch path has changed.

- [ ] **Step 3: Also run sensors crate tests**

Run: `cargo test -p bravepi-sensors`

Expected: All tests pass (existing IC module tests + new contact tests)

- [ ] **Step 4: Commit**

```bash
git add bravepi-adapter/src/task/convert.rs
git commit -m "refactor(bravepi): replace convert.rs match dispatch with registry lookup"
```

---

### Task 6: Clean up — remove unused imports and verify final state

**Files:**
- Modify: `bravepi-adapter/src/task/mod.rs` (if it re-exports `sensor_type_from_bravepi_raw`)
- Verify: no remaining references to removed functions

- [ ] **Step 1: Check for remaining references to removed functions**

Search for any remaining usages of:
- `sensor_type_from_bravepi_raw`
- `device_key_suffix`
- `contact_identity` (in convert.rs context)

Run grep across the codebase. If any remain (e.g., in `poc/src/main.rs`), update them to use registry lookup.

- [ ] **Step 2: Run full test suite**

Run: `cargo test -p bravepi-sensors && cargo test -p bravepi-adapter`

Expected: All tests pass across both crates.

- [ ] **Step 3: Verify the compile with no warnings**

Run: `cargo check -p bravepi-adapter 2>&1`

Expected: No warnings about unused imports or dead code (except the pre-existing `#[allow(dead_code)]` on `DeviceState.last_seen`).

- [ ] **Step 4: Commit if any cleanup was needed**

```bash
git add -u
git commit -m "chore(bravepi): remove unused imports after registry migration"
```

If no changes were needed, skip this commit.
