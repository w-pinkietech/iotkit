# Phase 1A: Adapter Standalone Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Package rpi-local-adapter as a standalone binary that reads I2C sensors and publishes to MQTT end-to-end.

**Architecture:** Three new crates layered bottom-up: `core/mqtt-contract` (DTOs + topic builder + encode/decode), `iotkit-adapter-runner` (MQTT client lifecycle + event publish loop), `iotkit-rpi-local` (binary composition root). One minor change to `rpi-local-adapter` to accept external `adapter_id`. Deploy assets (systemd unit, example config) in `deploy/`.

**Tech Stack:** Rust 2024 edition, rumqttc (MQTT 3.1.1), serde/serde_json, percent-encoding, clap, toml, tokio, tracing.

---

## File Structure

### New Files

| File | Responsibility |
|------|---------------|
| `core/mqtt-contract/Cargo.toml` | Crate manifest |
| `core/mqtt-contract/src/lib.rs` | Re-exports, EventType enum |
| `core/mqtt-contract/src/topic.rs` | Topic builder + percent-encoding |
| `core/mqtt-contract/src/envelope.rs` | Serde DTOs for all envelope types |
| `core/mqtt-contract/src/encode.rs` | AdapterEvent → JSON bytes |
| `core/mqtt-contract/src/decode.rs` | JSON bytes → AdapterEvent |
| `core/mqtt-contract/src/error.rs` | EncodeError, DecodeError |
| `iotkit-adapter-runner/Cargo.toml` | Crate manifest |
| `iotkit-adapter-runner/src/lib.rs` | Public API: `run()`, MqttConfig, RunnerError |
| `iotkit-adapter-runner/src/mqtt_client.rs` | rumqttc wrapper: connect, LWT, TLS |
| `iotkit-adapter-runner/src/publish_loop.rs` | event_rx → encode → publish task |
| `iotkit-adapter-runner/src/inventory.rs` | Active device tracking + retained inventory publish |
| `iotkit-rpi-local/Cargo.toml` | Binary crate manifest |
| `iotkit-rpi-local/src/main.rs` | CLI + config load + adapter start + runner |
| `iotkit-rpi-local/src/config.rs` | TOML config types + validation |
| `deploy/iotkit-rpi-local.service` | systemd unit file |
| `deploy/iotkit-rpi-local.example.toml` | Example config |

### Modified Files

| File | Change |
|------|--------|
| `rpi-local-adapter/src/lib.rs` | Add `start_with_id()` that accepts `AdapterId` |
| `Cargo.toml` (workspace root) | Add 3 new workspace members |

---

### Task 1: core/mqtt-contract — Scaffold + Topic Builder

**Files:**
- Create: `core/mqtt-contract/Cargo.toml`
- Create: `core/mqtt-contract/src/lib.rs`
- Create: `core/mqtt-contract/src/topic.rs`
- Create: `core/mqtt-contract/src/error.rs`

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "iotkit-core-mqtt-contract"
version = "0.1.0"
edition = "2024"

[dependencies]
iotkit-core-types = { path = "../types" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
percent-encoding = "2"

[dev-dependencies]
```

- [ ] **Step 2: Add to workspace**

In `Cargo.toml` (workspace root), add `"core/mqtt-contract"` to the `members` array.

- [ ] **Step 3: Write error types**

```rust
// core/mqtt-contract/src/error.rs
use std::fmt;

#[derive(Debug)]
pub enum EncodeError {
    /// Event type not supported for MQTT encoding (e.g. DeviceConfig)
    UnsupportedEvent(String),
    /// JSON serialization failed
    Json(serde_json::Error),
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedEvent(msg) => write!(f, "unsupported event: {msg}"),
            Self::Json(e) => write!(f, "json encode: {e}"),
        }
    }
}

impl std::error::Error for EncodeError {}

impl From<serde_json::Error> for EncodeError {
    fn from(e: serde_json::Error) -> Self { Self::Json(e) }
}

#[derive(Debug)]
pub enum DecodeError {
    /// JSON deserialization failed
    Json(serde_json::Error),
    /// Unknown or unsupported envelope version
    UnknownVersion(u32),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "json decode: {e}"),
            Self::UnknownVersion(v) => write!(f, "unknown envelope version: {v}"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<serde_json::Error> for DecodeError {
    fn from(e: serde_json::Error) -> Self { Self::Json(e) }
}
```

- [ ] **Step 4: Write topic builder with tests**

```rust
// core/mqtt-contract/src/topic.rs
use iotkit_core_types::{AdapterId, DeviceKey};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

/// Characters that must be percent-encoded in MQTT topic segments.
/// MQTT forbids `+`, `#`, `/` in topic level names; we also encode `:`
/// so adapter IDs like "rpi-local:default" become "rpi-local%3Adefault".
const TOPIC_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b'+')
    .add(b'#')
    .add(b'/')
    .add(b':')
    .add(b'%'); // encode % itself for reversibility

/// Percent-encode a string for use in an MQTT topic segment.
pub fn encode_topic_segment(s: &str) -> String {
    utf8_percent_encode(s, TOPIC_ENCODE_SET).to_string()
}

/// Event types for topic routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    Telemetry,
    Discovery,
    Loss,
    Error,
    Status,
}

impl EventType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Telemetry => "telemetry",
            Self::Discovery => "discovery",
            Self::Loss => "loss",
            Self::Error => "error",
            Self::Status => "status",
        }
    }
}

/// Build the MQTT topic for a given adapter and event type.
pub fn topic(adapter_id: &AdapterId, event_type: EventType) -> String {
    let encoded = encode_topic_segment(adapter_id.as_str());
    format!("iotkit/v1/{encoded}/{}", event_type.as_str())
}

/// Build the MQTT topic for a device inventory retained message.
pub fn inventory_topic(adapter_id: &AdapterId, device_key: &DeviceKey) -> String {
    let encoded_adapter = encode_topic_segment(adapter_id.as_str());
    let encoded_device = encode_topic_segment(device_key.as_str());
    format!("iotkit/v1/{encoded_adapter}/inventory/{encoded_device}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_telemetry() {
        let id = AdapterId::new("rpi-local:default");
        assert_eq!(
            topic(&id, EventType::Telemetry),
            "iotkit/v1/rpi-local%3Adefault/telemetry"
        );
    }

    #[test]
    fn topic_status() {
        let id = AdapterId::new("rpi-local:default");
        assert_eq!(
            topic(&id, EventType::Status),
            "iotkit/v1/rpi-local%3Adefault/status"
        );
    }

    #[test]
    fn topic_encodes_slash() {
        let id = AdapterId::new("bravepi:/dev/ttyAMA0");
        let t = topic(&id, EventType::Telemetry);
        assert!(!t.contains("//"), "slash in adapter_id must be encoded");
        assert!(t.contains("%2F"));
    }

    #[test]
    fn topic_encodes_percent() {
        let id = AdapterId::new("test%id");
        let t = topic(&id, EventType::Telemetry);
        assert!(t.contains("%25"), "percent sign must be double-encoded");
    }

    #[test]
    fn inventory_topic_format() {
        let aid = AdapterId::new("rpi-local:default");
        let dk = DeviceKey::new("i2c:0x60:mcp9600");
        assert_eq!(
            inventory_topic(&aid, &dk),
            "iotkit/v1/rpi-local%3Adefault/inventory/i2c%3A0x60%3Amcp9600"
        );
    }

    #[test]
    fn encode_topic_segment_roundtrip() {
        let original = "rpi-local:default";
        let encoded = encode_topic_segment(original);
        let decoded = percent_encoding::percent_decode_str(&encoded)
            .decode_utf8()
            .unwrap();
        assert_eq!(decoded, original);
    }
}
```

- [ ] **Step 5: Write lib.rs**

```rust
// core/mqtt-contract/src/lib.rs
mod error;
mod topic;

pub use error::{DecodeError, EncodeError};
pub use topic::{encode_topic_segment, inventory_topic, topic, EventType};
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p iotkit-core-mqtt-contract`
Expected: All 6 topic tests PASS.

- [ ] **Step 7: Commit**

```bash
git add core/mqtt-contract/ Cargo.toml
git commit -m "feat(core/mqtt-contract): scaffold crate with topic builder and percent-encoding"
```

---

### Task 2: core/mqtt-contract — Envelope DTOs + Encode/Decode

**Files:**
- Create: `core/mqtt-contract/src/envelope.rs`
- Create: `core/mqtt-contract/src/encode.rs`
- Create: `core/mqtt-contract/src/decode.rs`
- Modify: `core/mqtt-contract/src/lib.rs`

- [ ] **Step 1: Write envelope DTOs**

```rust
// core/mqtt-contract/src/envelope.rs
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Common header present in all envelopes.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Header {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub device_key: String,
    pub sensor_type: String,
    pub ingested_at: i64,
    pub values: Vec<f64>,
    pub labels: Vec<String>,
    pub rssi: Option<i16>,
    pub battery_pct: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityPayload {
    pub manufacturer: String,
    pub ic_part_number: String,
    pub sensor_type: String,
    pub connection: ConnectionPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionPayload {
    pub kind: String,
    pub parameters: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub device_key: String,
    pub identity: IdentityPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LossEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub device_key: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub device_key: Option<String>,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusEnvelope {
    pub v: u32,
    pub adapter_id: String,
    pub ts: i64,
    pub online: bool,
}

/// Used only for version check during decode.
#[derive(Deserialize)]
pub(crate) struct VersionCheck {
    pub v: u32,
}
```

- [ ] **Step 2: Write encode module**

```rust
// core/mqtt-contract/src/encode.rs
use crate::envelope::*;
use crate::error::EncodeError;
use crate::topic::EventType;
use iotkit_core_types::{AdapterId, AdapterEvent};
use std::time::{SystemTime, UNIX_EPOCH};

const ENVELOPE_VERSION: u32 = 1;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn system_time_to_ms(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Encode an AdapterEvent into (EventType, JSON bytes).
pub fn encode_event(
    adapter_id: &AdapterId,
    event: &AdapterEvent,
) -> Result<(EventType, Vec<u8>), EncodeError> {
    let aid = adapter_id.as_str().to_string();
    let ts = now_ms();

    match event {
        AdapterEvent::SensorData {
            device_key,
            reading,
            rssi,
            battery_pct,
            ingested_at,
        } => {
            let env = TelemetryEnvelope {
                v: ENVELOPE_VERSION,
                adapter_id: aid,
                ts,
                device_key: device_key.as_str().to_string(),
                sensor_type: reading.sensor_type.as_db_str().to_string(),
                ingested_at: system_time_to_ms(*ingested_at),
                values: reading.values.clone(),
                labels: reading.labels.iter().map(|s| s.to_string()).collect(),
                rssi: *rssi,
                battery_pct: *battery_pct,
            };
            Ok((EventType::Telemetry, serde_json::to_vec(&env)?))
        }
        AdapterEvent::DeviceDiscovered {
            device_key,
            identity,
        } => {
            let env = DiscoveryEnvelope {
                v: ENVELOPE_VERSION,
                adapter_id: aid,
                ts,
                device_key: device_key.as_str().to_string(),
                identity: IdentityPayload {
                    manufacturer: identity.manufacturer.clone(),
                    ic_part_number: identity.ic_part_number.clone(),
                    sensor_type: identity.sensor_type.as_db_str().to_string(),
                    connection: ConnectionPayload {
                        kind: identity.connection.kind.as_str().to_string(),
                        parameters: identity.connection.parameters.clone(),
                    },
                },
            };
            Ok((EventType::Discovery, serde_json::to_vec(&env)?))
        }
        AdapterEvent::DeviceLost { device_key, reason } => {
            let env = LossEnvelope {
                v: ENVELOPE_VERSION,
                adapter_id: aid,
                ts,
                device_key: device_key.as_str().to_string(),
                reason: reason.clone(),
            };
            Ok((EventType::Loss, serde_json::to_vec(&env)?))
        }
        AdapterEvent::AdapterError { device_key, error } => {
            let env = ErrorEnvelope {
                v: ENVELOPE_VERSION,
                adapter_id: aid,
                ts,
                device_key: device_key.as_ref().map(|k| k.as_str().to_string()),
                error: error.clone(),
            };
            Ok((EventType::Error, serde_json::to_vec(&env)?))
        }
        AdapterEvent::DeviceConfig { .. } => {
            Err(EncodeError::UnsupportedEvent("DeviceConfig not supported in v1 MQTT contract".into()))
        }
    }
}

/// Encode a status message.
pub fn encode_status(adapter_id: &AdapterId, online: bool) -> Vec<u8> {
    let env = StatusEnvelope {
        v: ENVELOPE_VERSION,
        adapter_id: adapter_id.as_str().to_string(),
        ts: if online { now_ms() } else { 0 },
        online,
    };
    serde_json::to_vec(&env).expect("status envelope serialization cannot fail")
}
```

- [ ] **Step 3: Write decode module**

```rust
// core/mqtt-contract/src/decode.rs
use crate::envelope::*;
use crate::error::DecodeError;
use crate::topic::EventType;
use iotkit_core_types::*;
use std::time::{Duration, UNIX_EPOCH};

const SUPPORTED_VERSION: u32 = 1;

fn check_version(payload: &[u8]) -> Result<(), DecodeError> {
    let vc: VersionCheck = serde_json::from_slice(payload)?;
    if vc.v != SUPPORTED_VERSION {
        return Err(DecodeError::UnknownVersion(vc.v));
    }
    Ok(())
}

fn ms_to_system_time(ms: i64) -> std::time::SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms as u64)
}

/// Decode MQTT payload back to (AdapterId, AdapterEvent).
pub fn decode_event(
    event_type: EventType,
    payload: &[u8],
) -> Result<(AdapterId, AdapterEvent), DecodeError> {
    check_version(payload)?;

    match event_type {
        EventType::Telemetry => {
            let env: TelemetryEnvelope = serde_json::from_slice(payload)?;
            let labels: Vec<&'static str> = env
                .labels
                .iter()
                .map(|s| -> &'static str { Box::leak(s.clone().into_boxed_str()) })
                .collect();
            let event = AdapterEvent::SensorData {
                device_key: DeviceKey::new(env.device_key),
                reading: SensorReading::new(
                    SensorType::from_db_str(&env.sensor_type),
                    env.values,
                    labels,
                ),
                rssi: env.rssi,
                battery_pct: env.battery_pct,
                ingested_at: ms_to_system_time(env.ingested_at),
            };
            Ok((AdapterId::new(env.adapter_id), event))
        }
        EventType::Discovery => {
            let env: DiscoveryEnvelope = serde_json::from_slice(payload)?;
            let event = AdapterEvent::DeviceDiscovered {
                device_key: DeviceKey::new(env.device_key),
                identity: SensorIdentity {
                    manufacturer: env.identity.manufacturer,
                    ic_part_number: env.identity.ic_part_number,
                    sensor_type: SensorType::from_db_str(&env.identity.sensor_type),
                    connection: ConnectionInfo {
                        kind: ConnectionKind::from_str(&env.identity.connection.kind),
                        parameters: env.identity.connection.parameters,
                    },
                },
            };
            Ok((AdapterId::new(env.adapter_id), event))
        }
        EventType::Loss => {
            let env: LossEnvelope = serde_json::from_slice(payload)?;
            let event = AdapterEvent::DeviceLost {
                device_key: DeviceKey::new(env.device_key),
                reason: env.reason,
            };
            Ok((AdapterId::new(env.adapter_id), event))
        }
        EventType::Error => {
            let env: ErrorEnvelope = serde_json::from_slice(payload)?;
            let event = AdapterEvent::AdapterError {
                device_key: env.device_key.map(DeviceKey::new),
                error: env.error,
            };
            Ok((AdapterId::new(env.adapter_id), event))
        }
        EventType::Status => {
            Err(DecodeError::Json(serde_json::from_str::<()>("\"use decode_status for status messages\"").unwrap_err()))
        }
    }
}

/// Decode a status message.
pub fn decode_status(payload: &[u8]) -> Result<(AdapterId, bool), DecodeError> {
    check_version(payload)?;
    let env: StatusEnvelope = serde_json::from_slice(payload)?;
    Ok((AdapterId::new(env.adapter_id), env.online))
}
```

- [ ] **Step 4: Add ConnectionKind::from_str and as_str to core/types**

Check if `ConnectionKind` already has `as_str()` and `from_str()`. If not, add them to `core/types/src/lib.rs`:

```rust
impl ConnectionKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Uart => "uart",
            Self::I2c => "i2c",
            Self::Gpio => "gpio",
            Self::Modbus => "modbus",
            Self::Other(s) => s.as_str(),
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "uart" => Self::Uart,
            "i2c" => Self::I2c,
            "gpio" => Self::Gpio,
            "modbus" => Self::Modbus,
            other => Self::Other(other.to_string()),
        }
    }
}
```

- [ ] **Step 5: Update lib.rs to re-export**

```rust
// core/mqtt-contract/src/lib.rs
mod decode;
mod encode;
mod envelope;
mod error;
mod topic;

pub use decode::{decode_event, decode_status};
pub use encode::{encode_event, encode_status};
pub use error::{DecodeError, EncodeError};
pub use topic::{encode_topic_segment, inventory_topic, topic, EventType};
```

- [ ] **Step 6: Write round-trip tests**

Add to `core/mqtt-contract/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_types::*;
    use std::collections::BTreeMap;
    use std::time::{Duration, UNIX_EPOCH};

    fn sample_adapter_id() -> AdapterId {
        AdapterId::new("rpi-local:default")
    }

    #[test]
    fn roundtrip_telemetry() {
        let aid = sample_adapter_id();
        let event = AdapterEvent::SensorData {
            device_key: DeviceKey::new("i2c:0x60:mcp9600"),
            reading: SensorReading::new(
                SensorType::Temperature,
                vec![25.3],
                vec!["temperature_c"],
            ),
            rssi: Some(-70),
            battery_pct: Some(85),
            ingested_at: UNIX_EPOCH + Duration::from_millis(1711700000000),
        };
        let (et, bytes) = encode_event(&aid, &event).unwrap();
        assert_eq!(et, EventType::Telemetry);

        let (decoded_aid, decoded_event) = decode_event(EventType::Telemetry, &bytes).unwrap();
        assert_eq!(decoded_aid.as_str(), aid.as_str());

        if let AdapterEvent::SensorData { device_key, reading, rssi, battery_pct, ingested_at } = decoded_event {
            assert_eq!(device_key.as_str(), "i2c:0x60:mcp9600");
            assert_eq!(reading.sensor_type, SensorType::Temperature);
            assert_eq!(reading.values, vec![25.3]);
            assert_eq!(rssi, Some(-70));
            assert_eq!(battery_pct, Some(85));
            assert_eq!(ingested_at, UNIX_EPOCH + Duration::from_millis(1711700000000));
        } else {
            panic!("expected SensorData");
        }
    }

    #[test]
    fn roundtrip_discovery() {
        let aid = sample_adapter_id();
        let mut params = BTreeMap::new();
        params.insert("address".into(), "0x60".into());
        let event = AdapterEvent::DeviceDiscovered {
            device_key: DeviceKey::new("i2c:0x60:mcp9600"),
            identity: SensorIdentity {
                manufacturer: "Microchip".into(),
                ic_part_number: "MCP9600".into(),
                sensor_type: SensorType::Temperature,
                connection: ConnectionInfo {
                    kind: ConnectionKind::I2c,
                    parameters: params,
                },
            },
        };
        let (et, bytes) = encode_event(&aid, &event).unwrap();
        assert_eq!(et, EventType::Discovery);

        let (_, decoded) = decode_event(EventType::Discovery, &bytes).unwrap();
        if let AdapterEvent::DeviceDiscovered { identity, .. } = decoded {
            assert_eq!(identity.manufacturer, "Microchip");
            assert_eq!(identity.connection.kind, ConnectionKind::I2c);
        } else {
            panic!("expected DeviceDiscovered");
        }
    }

    #[test]
    fn roundtrip_loss() {
        let aid = sample_adapter_id();
        let event = AdapterEvent::DeviceLost {
            device_key: DeviceKey::new("i2c:0x60:mcp9600"),
            reason: "5 consecutive read failures".into(),
        };
        let (et, bytes) = encode_event(&aid, &event).unwrap();
        assert_eq!(et, EventType::Loss);

        let (_, decoded) = decode_event(EventType::Loss, &bytes).unwrap();
        if let AdapterEvent::DeviceLost { reason, .. } = decoded {
            assert_eq!(reason, "5 consecutive read failures");
        } else {
            panic!("expected DeviceLost");
        }
    }

    #[test]
    fn roundtrip_error() {
        let aid = sample_adapter_id();
        let event = AdapterEvent::AdapterError {
            device_key: None,
            error: "bus error".into(),
        };
        let (et, bytes) = encode_event(&aid, &event).unwrap();
        assert_eq!(et, EventType::Error);

        let (_, decoded) = decode_event(EventType::Error, &bytes).unwrap();
        if let AdapterEvent::AdapterError { device_key, error } = decoded {
            assert!(device_key.is_none());
            assert_eq!(error, "bus error");
        } else {
            panic!("expected AdapterError");
        }
    }

    #[test]
    fn roundtrip_status() {
        let aid = sample_adapter_id();
        let bytes = encode_status(&aid, true);
        let (decoded_aid, online) = decode_status(&bytes).unwrap();
        assert_eq!(decoded_aid.as_str(), aid.as_str());
        assert!(online);
    }

    #[test]
    fn encode_device_config_returns_unsupported() {
        let aid = sample_adapter_id();
        let event = AdapterEvent::DeviceConfig {
            device_key: DeviceKey::new("test"),
            config: DeviceConfigData {
                firmware_version: None,
                uplink_interval_secs: None,
                properties: BTreeMap::new(),
            },
        };
        let result = encode_event(&aid, &event);
        assert!(result.is_err());
    }

    #[test]
    fn decode_unknown_version_returns_error() {
        let json = br#"{"v":99,"adapter_id":"test","ts":0,"device_key":"k","sensor_type":"temperature","ingested_at":0,"values":[],"labels":[],"rssi":null,"battery_pct":null}"#;
        let result = decode_event(EventType::Telemetry, json);
        assert!(matches!(result, Err(DecodeError::UnknownVersion(99))));
    }
}
```

- [ ] **Step 7: Run tests**

Run: `cargo test -p iotkit-core-mqtt-contract`
Expected: All tests PASS.

- [ ] **Step 8: Commit**

```bash
git add core/mqtt-contract/ core/types/
git commit -m "feat(core/mqtt-contract): envelope DTOs, encode/decode with round-trip tests"
```

---

### Task 3: rpi-local-adapter — Add start_with_id()

**Files:**
- Modify: `rpi-local-adapter/src/lib.rs`

- [ ] **Step 1: Write test for start_with_id**

Add to tests in `rpi-local-adapter/src/lib.rs`:

```rust
#[test]
fn start_with_custom_id_returns_matching_id() {
    let config = RpiLocalConfig {
        bus_path: "/dev/i2c-1".to_string(),
        poll_interval_ms: 1000,
        targets: vec![RpiLocalTarget::MCP9600 {
            address: 0x60,
            thermocouple_type: ThermocoupleType::K,
        }],
    };
    let custom_id = AdapterId::new("my-custom:adapter");
    let result = start_with_id(custom_id.clone(), config);
    // Will fail without runtime, but that's expected
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p rpi-local-adapter start_with_custom_id`
Expected: FAIL — `start_with_id` not found.

- [ ] **Step 3: Implement start_with_id**

In `rpi-local-adapter/src/lib.rs`, refactor `start()` to delegate to `start_with_id()`:

```rust
/// Start the adapter with the default adapter ID ("rpi-local:default").
pub fn start(config: RpiLocalConfig) -> Result<AdapterHandle, std::io::Error> {
    start_with_id(AdapterId::new("rpi-local:default"), config)
}

/// Start the adapter with a custom adapter ID.
pub fn start_with_id(adapter_id: AdapterId, config: RpiLocalConfig) -> Result<AdapterHandle, std::io::Error> {
    // ... existing start() body, but using adapter_id parameter
    // instead of hardcoded AdapterId::new("rpi-local:default")
}
```

The existing `start()` function body moves into `start_with_id()`, replacing the hardcoded `AdapterId::new("rpi-local:default")` with the `adapter_id` parameter. `start()` becomes a thin wrapper.

- [ ] **Step 4: Run all tests**

Run: `cargo test -p rpi-local-adapter`
Expected: All existing tests PASS + new test PASS.

- [ ] **Step 5: Verify gateway still compiles**

Run: `cargo test --workspace`
Expected: PASS — gateway uses `start()` which still exists with same signature.

- [ ] **Step 6: Commit**

```bash
git add rpi-local-adapter/
git commit -m "feat(rpi-local-adapter): add start_with_id() for custom adapter identity"
```

---

### Task 4: iotkit-adapter-runner — Scaffold + MQTT Client

**Files:**
- Create: `iotkit-adapter-runner/Cargo.toml`
- Create: `iotkit-adapter-runner/src/lib.rs`
- Create: `iotkit-adapter-runner/src/mqtt_client.rs`

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "iotkit-adapter-runner"
version = "0.1.0"
edition = "2024"

[dependencies]
iotkit-core-types = { path = "core/types" }
iotkit-core-mqtt-contract = { path = "core/mqtt-contract" }
rumqttc = "0.24"
tokio = { version = "1", features = ["rt", "sync", "signal", "macros"] }
tracing = "0.1"
```

Note: Verify the path references work. Since `iotkit-adapter-runner` is at the workspace root level, paths should be relative: `path = "core/types"` and `path = "core/mqtt-contract"`.

- [ ] **Step 2: Add to workspace**

In `Cargo.toml` (workspace root), add `"iotkit-adapter-runner"` to the `members` array.

- [ ] **Step 3: Write MqttConfig and RunnerError**

```rust
// iotkit-adapter-runner/src/lib.rs
mod mqtt_client;

use iotkit_core_types::{AdapterId, AdapterEvent};
use std::path::PathBuf;
use tokio::sync::mpsc;

/// MQTT broker connection configuration.
#[derive(Debug, Clone)]
pub struct MqttConfig {
    pub broker_url: String,
    pub client_id: Option<String>,
    pub keepalive_secs: Option<u32>,
    pub ca_path: Option<PathBuf>,
    pub client_cert_path: Option<PathBuf>,
    pub client_key_path: Option<PathBuf>,
}

/// Errors from the adapter runner.
#[derive(Debug)]
pub enum RunnerError {
    /// Invalid MQTT configuration
    Config(String),
    /// MQTT connection error
    Mqtt(String),
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(msg) => write!(f, "config error: {msg}"),
            Self::Mqtt(msg) => write!(f, "mqtt error: {msg}"),
        }
    }
}

impl std::error::Error for RunnerError {}

/// Run the adapter event loop: receive events from adapter, publish to MQTT.
/// Blocks until SIGTERM/SIGINT or fatal error.
pub async fn run(
    adapter_id: AdapterId,
    mqtt_config: MqttConfig,
    event_rx: mpsc::Receiver<AdapterEvent>,
) -> Result<(), RunnerError> {
    let (client, mut eventloop) = mqtt_client::connect(&adapter_id, &mqtt_config)?;

    // Placeholder — filled in next task
    todo!("implement publish loop + eventloop pump + signal handling")
}
```

- [ ] **Step 4: Write mqtt_client module**

```rust
// iotkit-adapter-runner/src/mqtt_client.rs
use crate::{MqttConfig, RunnerError};
use iotkit_core_mqtt_contract::{encode_status, topic, EventType};
use iotkit_core_types::AdapterId;
use rumqttc::{AsyncClient, EventLoop, MqttOptions, QoS, Transport};
use std::time::Duration;

/// Create and configure an MQTT client with LWT.
pub(crate) fn connect(
    adapter_id: &AdapterId,
    config: &MqttConfig,
) -> Result<(AsyncClient, EventLoop), RunnerError> {
    let client_id = config.client_id.clone().unwrap_or_else(|| {
        format!(
            "iotkit-{}-{}",
            adapter_id.as_str().replace(':', "-"),
            &uuid_short()
        )
    });

    let keepalive = Duration::from_secs(config.keepalive_secs.unwrap_or(30) as u64);

    let mut opts = MqttOptions::new(&client_id, parse_host(&config.broker_url)?, parse_port(&config.broker_url)?);
    opts.set_keep_alive(keepalive);

    // TLS configuration
    if config.broker_url.starts_with("mqtts://") {
        let ca = config
            .ca_path
            .as_ref()
            .ok_or_else(|| RunnerError::Config("mqtts:// requires ca_path".into()))?;
        let ca_bytes = std::fs::read(ca)
            .map_err(|e| RunnerError::Config(format!("failed to read CA cert: {e}")))?;

        let mut transport = rumqttc::TlsConfiguration::Simple {
            ca: ca_bytes,
            alpn: None,
            client_auth: None,
        };

        if let (Some(cert_path), Some(key_path)) = (&config.client_cert_path, &config.client_key_path) {
            let cert = std::fs::read(cert_path)
                .map_err(|e| RunnerError::Config(format!("failed to read client cert: {e}")))?;
            let key = std::fs::read(key_path)
                .map_err(|e| RunnerError::Config(format!("failed to read client key: {e}")))?;
            transport = rumqttc::TlsConfiguration::Simple {
                ca: ca_bytes,
                alpn: None,
                client_auth: Some((cert, key)),
            };
        }

        opts.set_transport(Transport::tls_with_config(transport.into()));
    }

    // Last Will and Testament — offline status
    let lwt_topic = topic(adapter_id, EventType::Status);
    let lwt_payload = encode_status(adapter_id, false);
    opts.set_last_will(rumqttc::LastWill::new(
        &lwt_topic,
        lwt_payload,
        QoS::AtLeastOnce,
        true, // retained
    ));

    let (client, eventloop) = AsyncClient::new(opts, 100); // 100 = channel capacity
    Ok((client, eventloop))
}

fn parse_host(url: &str) -> Result<String, RunnerError> {
    let stripped = url
        .strip_prefix("mqtt://")
        .or_else(|| url.strip_prefix("mqtts://"))
        .ok_or_else(|| RunnerError::Config("broker_url must start with mqtt:// or mqtts://".into()))?;
    let host = stripped.split(':').next().unwrap_or(stripped);
    Ok(host.to_string())
}

fn parse_port(url: &str) -> Result<u16, RunnerError> {
    let stripped = url
        .strip_prefix("mqtt://")
        .or_else(|| url.strip_prefix("mqtts://"))
        .ok_or_else(|| RunnerError::Config("broker_url must start with mqtt:// or mqtts://".into()))?;
    let parts: Vec<&str> = stripped.split(':').collect();
    if parts.len() >= 2 {
        parts[1]
            .parse()
            .map_err(|_| RunnerError::Config(format!("invalid port in broker_url: {}", parts[1])))
    } else if url.starts_with("mqtts://") {
        Ok(8883)
    } else {
        Ok(1883)
    }
}

fn uuid_short() -> String {
    // Simple 8-char random hex without external dependency
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:08x}", (n & 0xFFFF_FFFF) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mqtt_url() {
        assert_eq!(parse_host("mqtt://localhost:1883").unwrap(), "localhost");
        assert_eq!(parse_port("mqtt://localhost:1883").unwrap(), 1883);
    }

    #[test]
    fn parse_mqtts_url_default_port() {
        assert_eq!(parse_host("mqtts://broker.example.com").unwrap(), "broker.example.com");
        assert_eq!(parse_port("mqtts://broker.example.com").unwrap(), 8883);
    }

    #[test]
    fn parse_invalid_url() {
        assert!(parse_host("http://localhost").is_err());
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p iotkit-adapter-runner`
Expected: URL parsing tests PASS.

- [ ] **Step 6: Commit**

```bash
git add iotkit-adapter-runner/ Cargo.toml
git commit -m "feat(adapter-runner): scaffold crate with MQTT client, LWT, TLS config"
```

---

### Task 5: iotkit-adapter-runner — Publish Loop + Inventory + Signal Handling

**Files:**
- Create: `iotkit-adapter-runner/src/publish_loop.rs`
- Create: `iotkit-adapter-runner/src/inventory.rs`
- Modify: `iotkit-adapter-runner/src/lib.rs`

- [ ] **Step 1: Write inventory tracker**

```rust
// iotkit-adapter-runner/src/inventory.rs
use iotkit_core_mqtt_contract::{encode_event, inventory_topic, EventType};
use iotkit_core_types::{AdapterId, AdapterEvent, DeviceKey};
use rumqttc::{AsyncClient, QoS};
use std::collections::HashMap;
use tracing;

/// Tracks active devices and manages retained inventory messages.
pub(crate) struct InventoryTracker {
    adapter_id: AdapterId,
    /// device_key → last discovery payload (JSON bytes)
    active_devices: HashMap<String, Vec<u8>>,
}

impl InventoryTracker {
    pub fn new(adapter_id: AdapterId) -> Self {
        Self {
            adapter_id,
            active_devices: HashMap::new(),
        }
    }

    /// Process an event and publish retained inventory if needed.
    /// Returns true if inventory was updated.
    pub async fn process_event(&mut self, event: &AdapterEvent, client: &AsyncClient) -> bool {
        match event {
            AdapterEvent::DeviceDiscovered { device_key, .. } => {
                // Encode the discovery event for the inventory retained message
                if let Ok((_, payload)) = encode_event(&self.adapter_id, event) {
                    let topic = inventory_topic(&self.adapter_id, device_key);
                    if let Err(e) = client.publish(&topic, QoS::AtLeastOnce, true, &payload).await {
                        tracing::warn!(error = %e, device = device_key.as_str(), "failed to publish retained inventory");
                    } else {
                        tracing::debug!(device = device_key.as_str(), "published retained inventory");
                    }
                    self.active_devices.insert(device_key.as_str().to_string(), payload);
                }
                true
            }
            AdapterEvent::DeviceLost { device_key, .. } => {
                // Publish empty retained message to remove from broker
                let topic = inventory_topic(&self.adapter_id, device_key);
                if let Err(e) = client.publish(&topic, QoS::AtLeastOnce, true, Vec::<u8>::new()).await {
                    tracing::warn!(error = %e, device = device_key.as_str(), "failed to clear retained inventory");
                } else {
                    tracing::debug!(device = device_key.as_str(), "cleared retained inventory");
                }
                self.active_devices.remove(device_key.as_str());
                true
            }
            _ => false,
        }
    }

    /// Re-publish all active device inventory (called on MQTT reconnect).
    pub async fn republish_all(&self, client: &AsyncClient) {
        for (device_key_str, payload) in &self.active_devices {
            let dk = iotkit_core_types::DeviceKey::new(device_key_str.clone());
            let topic = inventory_topic(&self.adapter_id, &dk);
            if let Err(e) = client.publish(&topic, QoS::AtLeastOnce, true, payload.clone()).await {
                tracing::warn!(error = %e, device = %device_key_str, "failed to republish inventory on reconnect");
            }
        }
        if !self.active_devices.is_empty() {
            tracing::info!(count = self.active_devices.len(), "republished inventory on reconnect");
        }
    }
}
```

- [ ] **Step 2: Write publish loop**

```rust
// iotkit-adapter-runner/src/publish_loop.rs
use crate::inventory::InventoryTracker;
use iotkit_core_mqtt_contract::{encode_event, topic};
use iotkit_core_types::{AdapterId, AdapterEvent};
use rumqttc::{AsyncClient, QoS};
use tokio::sync::mpsc;
use tracing;

/// Consume events from adapter and publish to MQTT.
/// Runs until event_rx is closed.
pub(crate) async fn run(
    adapter_id: AdapterId,
    client: AsyncClient,
    mut event_rx: mpsc::Receiver<AdapterEvent>,
    mut inventory: InventoryTracker,
) {
    while let Some(event) = event_rx.recv().await {
        // Update inventory tracking (retained publish for discovery/loss)
        inventory.process_event(&event, &client).await;

        // Encode and publish to event topic
        match encode_event(&adapter_id, &event) {
            Ok((event_type, payload)) => {
                let t = topic(&adapter_id, event_type);
                if let Err(e) = client.publish(&t, QoS::AtLeastOnce, false, payload).await {
                    tracing::warn!(
                        error = %e,
                        event_type = ?event_type,
                        "MQTT publish failed, dropping event"
                    );
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "skipping unencodable event");
            }
        }
    }

    tracing::info!("adapter event channel closed, publish loop exiting");
}
```

- [ ] **Step 3: Implement run() in lib.rs**

Replace the `todo!()` in `iotkit-adapter-runner/src/lib.rs`:

```rust
// iotkit-adapter-runner/src/lib.rs
mod inventory;
mod mqtt_client;
mod publish_loop;

use iotkit_core_mqtt_contract::{encode_status, topic, EventType};
use iotkit_core_types::{AdapterId, AdapterEvent};
use rumqttc::{Event, Incoming, QoS};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tracing;

// ... MqttConfig, RunnerError as before ...

/// Run the adapter event loop: receive events from adapter, publish to MQTT.
/// Blocks until SIGTERM/SIGINT or fatal error.
pub async fn run(
    adapter_id: AdapterId,
    mqtt_config: MqttConfig,
    event_rx: mpsc::Receiver<AdapterEvent>,
) -> Result<(), RunnerError> {
    let (client, mut eventloop) = mqtt_client::connect(&adapter_id, &mqtt_config)?;

    let inventory = inventory::InventoryTracker::new(adapter_id.clone());

    // Publish online status (retained)
    let status_topic = topic(&adapter_id, EventType::Status);
    let online_payload = encode_status(&adapter_id, true);
    client
        .publish(&status_topic, QoS::AtLeastOnce, true, online_payload)
        .await
        .map_err(|e| RunnerError::Mqtt(format!("failed to publish online status: {e}")))?;
    tracing::info!(adapter_id = adapter_id.as_str(), "published online status");

    // Spawn MQTT eventloop pump as dedicated task
    let eventloop_handle = tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                    tracing::info!("MQTT connected");
                }
                Ok(_) => {} // PUBACK, PINGRESP, etc.
                Err(e) => {
                    tracing::warn!(error = %e, "MQTT eventloop error, will reconnect");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    });

    // Spawn publish loop as dedicated task
    let publish_handle = tokio::spawn(publish_loop::run(
        adapter_id.clone(),
        client.clone(),
        event_rx,
        inventory,
    ));

    // Wait for signal
    tokio::signal::ctrl_c().await.ok();
    tracing::info!("shutdown signal received");

    // Publish offline status
    let offline_payload = encode_status(&adapter_id, false);
    let _ = client
        .publish(&status_topic, QoS::AtLeastOnce, true, offline_payload)
        .await;
    let _ = client.disconnect().await;

    // Abort tasks
    eventloop_handle.abort();
    publish_handle.abort();

    Ok(())
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p iotkit-adapter-runner`
Expected: PASS (existing URL tests + compilation).

- [ ] **Step 5: Run workspace test**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add iotkit-adapter-runner/
git commit -m "feat(adapter-runner): implement publish loop, inventory tracking, signal handling"
```

---

### Task 6: iotkit-rpi-local — Binary Crate + Config

**Files:**
- Create: `iotkit-rpi-local/Cargo.toml`
- Create: `iotkit-rpi-local/src/main.rs`
- Create: `iotkit-rpi-local/src/config.rs`

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "iotkit-rpi-local"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "iotkit-rpi-local"
path = "src/main.rs"

[dependencies]
iotkit-adapter-runner = { path = "../iotkit-adapter-runner" }
iotkit-core-types = { path = "../core/types" }
rpi-local-adapter = { path = "../rpi-local-adapter" }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
toml = "0.8"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

- [ ] **Step 2: Add to workspace**

In `Cargo.toml` (workspace root), add `"iotkit-rpi-local"` to the `members` array.

- [ ] **Step 3: Write config module**

```rust
// iotkit-rpi-local/src/config.rs
use iotkit_adapter_runner::MqttConfig;
use rpi_local_adapter::{RpiLocalConfig, RpiLocalTarget};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct StandaloneConfig {
    pub adapter_id: String,
    pub mqtt: MqttToml,
    pub adapter: AdapterToml,
}

#[derive(Debug, Deserialize)]
pub struct MqttToml {
    pub broker_url: String,
    pub client_id: Option<String>,
    pub keepalive_secs: Option<u32>,
    pub ca_path: Option<String>,
    pub client_cert_path: Option<String>,
    pub client_key_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AdapterToml {
    pub bus_path: String,
    pub poll_interval_ms: u64,
    pub targets: Vec<TargetToml>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "driver")]
pub enum TargetToml {
    #[serde(rename = "mcp9600")]
    Mcp9600 {
        address: u8,
        thermocouple_type: Option<String>,
    },
    #[serde(rename = "opt3001")]
    Opt3001 {
        address: u8,
    },
}

impl StandaloneConfig {
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read config file {}: {e}", path.display()))?;
        let config: StandaloneConfig = toml::from_str(&contents)
            .map_err(|e| format!("failed to parse config: {e}"))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if self.adapter_id.is_empty() {
            return Err("adapter_id must not be empty".into());
        }
        if !self.mqtt.broker_url.starts_with("mqtt://") && !self.mqtt.broker_url.starts_with("mqtts://") {
            return Err("mqtt.broker_url must start with mqtt:// or mqtts://".into());
        }
        if self.mqtt.broker_url.starts_with("mqtts://") && self.mqtt.ca_path.is_none() {
            return Err("mqtts:// requires mqtt.ca_path".into());
        }
        if let Some(k) = self.mqtt.keepalive_secs {
            if k == 0 {
                return Err("mqtt.keepalive_secs must be > 0".into());
            }
        }
        if self.adapter.targets.is_empty() {
            return Err("adapter.targets must not be empty".into());
        }
        Ok(())
    }

    pub fn to_mqtt_config(&self) -> MqttConfig {
        MqttConfig {
            broker_url: self.mqtt.broker_url.clone(),
            client_id: self.mqtt.client_id.clone(),
            keepalive_secs: self.mqtt.keepalive_secs,
            ca_path: self.mqtt.ca_path.as_ref().map(PathBuf::from),
            client_cert_path: self.mqtt.client_cert_path.as_ref().map(PathBuf::from),
            client_key_path: self.mqtt.client_key_path.as_ref().map(PathBuf::from),
        }
    }

    pub fn to_rpi_local_config(&self) -> RpiLocalConfig {
        use bravepi_sensors::ThermocoupleType;
        let targets = self.adapter.targets.iter().map(|t| match t {
            TargetToml::Mcp9600 { address, thermocouple_type } => {
                let tc = match thermocouple_type.as_deref().unwrap_or("K") {
                    "J" => ThermocoupleType::J,
                    "T" => ThermocoupleType::T,
                    "N" => ThermocoupleType::N,
                    "S" => ThermocoupleType::S,
                    "E" => ThermocoupleType::E,
                    "B" => ThermocoupleType::B,
                    "R" => ThermocoupleType::R,
                    _ => ThermocoupleType::K,
                };
                RpiLocalTarget::MCP9600 {
                    address: *address,
                    thermocouple_type: tc,
                }
            }
            TargetToml::Opt3001 { address } => {
                RpiLocalTarget::OPT3001 { address: *address }
            }
        }).collect();

        RpiLocalConfig {
            bus_path: self.adapter.bus_path.clone(),
            poll_interval_ms: self.adapter.poll_interval_ms,
            targets,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_config() {
        let toml_str = r#"
adapter_id = "rpi-local:default"

[mqtt]
broker_url = "mqtt://localhost:1883"

[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000

[[adapter.targets]]
driver = "mcp9600"
address = 96
thermocouple_type = "K"

[[adapter.targets]]
driver = "opt3001"
address = 68
"#;
        let config: StandaloneConfig = toml::from_str(toml_str).unwrap();
        assert!(config.validate().is_ok());
        assert_eq!(config.adapter_id, "rpi-local:default");
        assert_eq!(config.adapter.targets.len(), 2);
    }

    #[test]
    fn validate_empty_adapter_id() {
        let toml_str = r#"
adapter_id = ""
[mqtt]
broker_url = "mqtt://localhost:1883"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
[[adapter.targets]]
driver = "opt3001"
address = 68
"#;
        let config: StandaloneConfig = toml::from_str(toml_str).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_invalid_broker_url() {
        let toml_str = r#"
adapter_id = "test"
[mqtt]
broker_url = "http://localhost"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
[[adapter.targets]]
driver = "opt3001"
address = 68
"#;
        let config: StandaloneConfig = toml::from_str(toml_str).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_mqtts_requires_ca() {
        let toml_str = r#"
adapter_id = "test"
[mqtt]
broker_url = "mqtts://broker.example.com"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
[[adapter.targets]]
driver = "opt3001"
address = 68
"#;
        let config: StandaloneConfig = toml::from_str(toml_str).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_empty_targets() {
        let toml_str = r#"
adapter_id = "test"
[mqtt]
broker_url = "mqtt://localhost:1883"
[adapter]
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000
"#;
        // This should fail to parse because targets is missing
        let result: Result<StandaloneConfig, _> = toml::from_str(toml_str);
        assert!(result.is_err() || result.unwrap().validate().is_err());
    }
}
```

- [ ] **Step 4: Write main.rs**

```rust
// iotkit-rpi-local/src/main.rs
mod config;

use clap::Parser;
use iotkit_core_types::AdapterId;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "iotkit-rpi-local", version, about = "Standalone I2C sensor adapter with MQTT output")]
struct Cli {
    /// Path to TOML config file
    #[arg(short, long, default_value = "iotkit-rpi-local.toml")]
    config: PathBuf,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // Try default paths if specified file doesn't exist
    let config_path = if cli.config.exists() {
        cli.config
    } else if PathBuf::from("/etc/iotkit/iotkit-rpi-local.toml").exists() {
        PathBuf::from("/etc/iotkit/iotkit-rpi-local.toml")
    } else {
        cli.config // will produce a clear error in load()
    };

    let config = match config::StandaloneConfig::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, path = %config_path.display(), "failed to load config");
            std::process::exit(1);
        }
    };

    tracing::info!(
        adapter_id = %config.adapter_id,
        broker = %config.mqtt.broker_url,
        bus_path = %config.adapter.bus_path,
        targets = config.adapter.targets.len(),
        "config loaded"
    );

    let adapter_id = AdapterId::new(&config.adapter_id);
    let mqtt_config = config.to_mqtt_config();
    let rpi_config = config.to_rpi_local_config();

    // Validate adapter config
    if let Err(e) = rpi_local_adapter::validate(&rpi_config) {
        tracing::error!(error = %e, "adapter config validation failed");
        std::process::exit(1);
    }

    // Create runtime — must exist before adapter start
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async {
        // Start adapter
        let mut handle = match rpi_local_adapter::start_with_id(adapter_id.clone(), rpi_config) {
            Ok(h) => {
                tracing::info!(adapter_id = %h.id, "adapter started");
                h
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to start adapter");
                std::process::exit(1);
            }
        };

        let parts = handle.into_parts();

        // Run adapter runner (blocks until signal)
        if let Err(e) = iotkit_adapter_runner::run(
            adapter_id,
            mqtt_config,
            parts.event_rx,
        ).await {
            tracing::error!(error = %e, "adapter runner failed");
        }

        // Shutdown adapter
        if let Err(e) = parts.shutdown.shutdown().await {
            tracing::warn!(error = %e, "adapter shutdown error");
        }

        tracing::info!("shutdown complete");
    });
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p iotkit-rpi-local`
Expected: Config tests PASS.

- [ ] **Step 6: Build binary**

Run: `cargo build -p iotkit-rpi-local`
Expected: Binary compiles successfully.

- [ ] **Step 7: Commit**

```bash
git add iotkit-rpi-local/ Cargo.toml
git commit -m "feat(iotkit-rpi-local): standalone binary with TOML config and CLI"
```

---

### Task 7: Deploy Assets

**Files:**
- Create: `deploy/iotkit-rpi-local.service`
- Create: `deploy/iotkit-rpi-local.example.toml`

- [ ] **Step 1: Create systemd unit**

```ini
# deploy/iotkit-rpi-local.service
[Unit]
Description=iotkit rpi-local I2C sensor adapter
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/opt/iotkit/bin/iotkit-rpi-local --config /opt/iotkit/etc/iotkit-rpi-local.toml
Restart=on-failure
RestartSec=5
User=iotkit
Group=iotkit
WorkingDirectory=/opt/iotkit/data
StandardOutput=journal
StandardError=journal

# Security hardening
ProtectSystem=strict
ReadWritePaths=/opt/iotkit/data
NoNewPrivileges=true
ProtectHome=true

# I2C access
SupplementaryGroups=i2c

[Install]
WantedBy=multi-user.target
```

- [ ] **Step 2: Create example config**

```toml
# deploy/iotkit-rpi-local.example.toml
#
# Configuration for iotkit-rpi-local standalone I2C sensor adapter.
# Copy to /opt/iotkit/etc/iotkit-rpi-local.toml and edit.

# Unique identifier for this adapter instance.
adapter_id = "rpi-local:default"

[mqtt]
# MQTT broker URL. Use mqtt:// for plain TCP, mqtts:// for TLS.
broker_url = "mqtt://localhost:1883"

# Optional: explicit client ID (auto-generated if omitted)
# client_id = "iotkit-rpi-local-01"

# Optional: keepalive interval in seconds (default: 30)
# keepalive_secs = 30

# TLS settings (required for mqtts://)
# ca_path = "/opt/iotkit/etc/certs/ca.pem"
# client_cert_path = "/opt/iotkit/etc/certs/client.pem"
# client_key_path = "/opt/iotkit/etc/certs/client.key"

[adapter]
# I2C bus device path
bus_path = "/dev/i2c-1"

# Sensor polling interval in milliseconds
poll_interval_ms = 1000

# Sensor targets — add one [[adapter.targets]] block per sensor

[[adapter.targets]]
driver = "mcp9600"
address = 0x60
thermocouple_type = "K"  # K, J, T, N, S, E, B, R

[[adapter.targets]]
driver = "opt3001"
address = 0x44
```

- [ ] **Step 3: Commit**

```bash
git add deploy/
git commit -m "feat(deploy): add systemd unit and example config for rpi-local standalone"
```

---

### Task 8: Workspace Integration + Final Verification

**Files:**
- Verify: all workspace members compile and tests pass

- [ ] **Step 1: Run full workspace test**

Run: `cargo test --workspace`
Expected: All tests PASS.

- [ ] **Step 2: Verify binary builds**

Run: `cargo build -p iotkit-rpi-local`
Expected: Binary at `target/debug/iotkit-rpi-local`.

- [ ] **Step 3: Verify binary runs (help)**

Run: `./target/debug/iotkit-rpi-local --help`
Expected: Shows help text with --config option.

- [ ] **Step 4: Verify binary shows version**

Run: `./target/debug/iotkit-rpi-local --version`
Expected: Shows version string.

- [ ] **Step 5: Commit (if any fixes were needed)**

```bash
git add -A
git commit -m "fix: workspace integration fixes for Phase 1A"
```
