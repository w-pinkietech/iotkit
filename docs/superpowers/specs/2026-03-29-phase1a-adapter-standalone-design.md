# Phase 1A: Adapter Standalone — Design Spec

Date: 2026-03-29
Status: Draft
Issues: #40, #41, #42, #31

## Goal

rpi-local-adapter を独立バイナリとしてパッケージングし、I2C センサー読み取り → MQTT publish が end-to-end で動く状態にする。既存の gateway 組み込みモードは変更しない。

## Architecture

```
┌─────────────────────────────────────────────┐
│ iotkit-rpi-local (binary crate)             │
│                                             │
│  TOML config → rpi-local-adapter::start()   │
│                    │                        │
│                event_rx                     │
│                    │                        │
│           adapter-runner::run()             │
│              │            │                 │
│         envelope      MQTT client           │
│         conversion    (rumqttc)             │
│              │            │                 │
│         mqtt-contract  publish              │
└─────────────────────────────────────────────┘
        │
        ▼ MQTT
   Any broker (Mosquitto, AWS IoT, HiveMQ, etc.)
```

### Crate Structure

| Crate | Type | 責務 |
|-------|------|------|
| `core/mqtt-contract` | lib | MQTT topic schema + JSON envelope DTOs + AdapterEvent 変換 |
| `iotkit-adapter-runner` | lib | 共通 edge runtime: MQTT client, LWT, event publish loop (no signal handling — signals are the binary's responsibility) |
| `iotkit-rpi-local` | bin | rpi-local-adapter を standalone 実行する composition root |

依存関係: `iotkit-rpi-local` → `iotkit-adapter-runner` → `core/mqtt-contract` → `core/types`

既存 crate (`iotkit-polling-adapter-runtime`, `iotkit-gateway`) は変更しない。`rpi-local-adapter` は adapter_id を外部から受け取れるよう `start()` の signature を拡張する（後述）。`core/types` は `SensorReading.labels` を `Vec<&'static str>` → `Vec<String>` に変更する（MQTT decode で owned string が必要なため）。

---

## 1. MQTT Event Envelope (`core/mqtt-contract`)

### 1.1 Topic Schema

All topics share the prefix `iotkit/v1/`. The `{adapter_id}` and `{device_key}` path segments are percent-encoded (see Section 1.2).

| Topic Pattern | Payload Type | Retained | QoS |
|---|---|---|---|
| `iotkit/v1/{adapter_id}/telemetry` | SensorData envelope | No | 1 |
| `iotkit/v1/{adapter_id}/discovery` | DeviceDiscovered envelope | No | 1 |
| `iotkit/v1/{adapter_id}/loss` | DeviceLost envelope | No | 1 |
| `iotkit/v1/{adapter_id}/error` | AdapterError envelope | No | 1 |
| `iotkit/v1/{adapter_id}/status` | Status envelope | Yes | 1 |
| `iotkit/v1/{adapter_id}/inventory/{device_key}` | Discovery envelope (per-device) | Yes | 1 |

**Rules:**

- Non-retained topics carry live event streams. Subscribers that connect late will miss historical events.
- The `status` topic carries the adapter's online/offline state and MUST be retained so late subscribers immediately know adapter liveness.
- Each `inventory/{device_key}` topic carries the latest discovery envelope for that device. It MUST be retained. A DeviceLost event triggers an inventory tombstone: an empty retained payload (`b""`) published to the device's inventory topic, clearing the retained message.
- No topic in this schema uses wildcard segments (`+` or `#`) as a literal part of the topic string; those characters are reserved for MQTT subscribers to use in subscriptions.

### 1.2 Segment Encoding

`adapter_id` and `device_key` values are opaque strings that may contain characters reserved by MQTT topic syntax. Before embedding a value in a topic, apply percent-encoding using the following mandatory substitutions. Encoding is applied character-by-character; no other transformation is applied.

| Character | Encoded form |
|---|---|
| `:` | `%3A` |
| `/` | `%2F` |
| `+` | `%2B` |
| `#` | `%23` |
| `%` | `%25` |

All other characters are passed through unchanged. Decoding reverses this substitution. Encoding is **reversible**: `decode(encode(s)) == s` for all inputs.

The `encode_topic_segment` function (Section 1.8) implements this transformation.

**Examples:**

| Raw value | Encoded |
|---|---|
| `bravepi:0` | `bravepi%3A0` |
| `i2c:0x44:sht31` | `i2c%3A0x44%3Asht31` |
| `sensor+type` | `sensor%2Btype` |
| `100%` | `100%25` |

### 1.3 Envelope Format

All messages are UTF-8 encoded JSON objects. Every envelope contains a common header; payload-specific fields follow.

#### Common Header

| Field | JSON type | Description |
|---|---|---|
| `v` | integer | Schema version. Currently always `1`. |
| `adapter_id` | string | The adapter that produced the event. Raw (un-encoded) value. |
| `ts` | integer | Unix timestamp in milliseconds (UTC) at time of encoding. Must be >= 0. Exception: LWT status messages use `ts = 0` (see Section 1.3.6). |

#### 1.3.1 Telemetry Envelope (`SensorData`)

Published to `iotkit/v1/{adapter_id}/telemetry`.

Additional fields:

| Field | JSON type | Nullable | Description |
|---|---|---|---|
| `device_key` | string | No | Opaque device identifier. Raw value. |
| `sensor_type` | string | No | Sensor type string using `SensorType::as_db_str()` values (e.g. `"temperature"`, `"illuminance"`, `"differential_pressure"`). |
| `ingested_at` | integer | No | Unix timestamp in milliseconds when the reading was ingested by the adapter. Must be >= 0. |
| `values` | array of number | No | Sensor reading values (JSON numbers, decoded as `f64`). **Must** be the same length as `labels`; mismatch produces `DecodeError::InvalidPayload`. |
| `labels` | array of string | No | Names for each value in `values`. **Must** be the same length as `values`; mismatch produces `DecodeError::InvalidPayload`. |
| `rssi` | integer or null | Yes | Received signal strength in dBm, if available. |
| `battery_pct` | integer or null | Yes | Battery level as a raw value in the range [0, 255] (`u8`), if available. Interpretation is device-specific. |

**Complete JSON example:**

```json
{
  "v": 1,
  "adapter_id": "bravepi:0",
  "ts": 1743206400000,
  "device_key": "i2c:0x44:sht31",
  "sensor_type": "temperature",
  "ingested_at": 1743206399850,
  "values": [23.4, 61.2],
  "labels": ["temperature_c", "humidity_pct"],
  "rssi": -72,
  "battery_pct": null
}
```

#### 1.3.2 Discovery Envelope (`DeviceDiscovered`)

Published to `iotkit/v1/{adapter_id}/discovery` (non-retained) AND to `iotkit/v1/{adapter_id}/inventory/{device_key}` (retained). Both publications carry the identical JSON payload.

Additional fields:

| Field | JSON type | Nullable | Description |
|---|---|---|---|
| `device_key` | string | No | Opaque device identifier. Raw value. |
| `identity` | object | No | Device identity record (see below). |

`identity` object fields:

| Field | JSON type | Nullable | Description |
|---|---|---|---|
| `manufacturer` | string | No | Manufacturer name. Empty string `""` if unknown. Matches `SensorIdentity.manufacturer: String` (non-nullable). |
| `ic_part_number` | string | No | IC part number or model identifier. Empty string `""` if unknown. Matches `SensorIdentity.ic_part_number: String` (non-nullable). |
| `sensor_type` | string | No | Sensor type string using `SensorType::as_db_str()` values (e.g. `"temperature"`, `"illuminance"`). |
| `connection` | object | No | Connection descriptor (see below). |

`identity.connection` object fields:

| Field | JSON type | Nullable | Description |
|---|---|---|---|
| `kind` | string | No | Connection kind identifier using `ConnectionKind::as_str()` values: `"i2c"`, `"uart"`, `"gpio"`, `"modbus"`, or a custom string for `Other(s)`. |
| `parameters` | object | No | Freeform key-value map of connection parameters specific to `kind`. String values only. An empty object `{}` is valid. |

**Complete JSON example:**

```json
{
  "v": 1,
  "adapter_id": "bravepi:0",
  "ts": 1743206400000,
  "device_key": "i2c:0x44:sht31",
  "identity": {
    "manufacturer": "Sensirion",
    "ic_part_number": "SHT31-DIS",
    "sensor_type": "temperature",
    "connection": {
      "kind": "i2c",
      "parameters": {
        "bus": "1",
        "address": "0x44"
      }
    }
  }
}
```

#### 1.3.3 Loss Envelope (`DeviceLost`)

Published to `iotkit/v1/{adapter_id}/loss`. Also triggers an inventory tombstone: an empty retained payload (`b""`) published to `iotkit/v1/{adapter_id}/inventory/{device_key}`.

Additional fields:

| Field | JSON type | Nullable | Description |
|---|---|---|---|
| `device_key` | string | No | Opaque device identifier. Raw value. |
| `reason` | string | No | Human-readable description of why the device was lost (e.g. `"timeout"`, `"removed"`, `"io_error: ..."`). Not machine-parsed. |

**Complete JSON example:**

```json
{
  "v": 1,
  "adapter_id": "bravepi:0",
  "ts": 1743206500000,
  "device_key": "i2c:0x44:sht31",
  "reason": "timeout after 30s without response"
}
```

#### 1.3.4 Error Envelope (`AdapterError`)

Published to `iotkit/v1/{adapter_id}/error`.

Additional fields:

| Field | JSON type | Nullable | Description |
|---|---|---|---|
| `device_key` | string or null | Yes | Device associated with the error, if applicable. Null for adapter-level errors. |
| `error` | string | No | Human-readable error description. Not machine-parsed. |

**Complete JSON example (device-scoped error):**

```json
{
  "v": 1,
  "adapter_id": "bravepi:0",
  "ts": 1743206600000,
  "device_key": "i2c:0x44:sht31",
  "error": "CRC mismatch reading SHT31 measurement register"
}
```

**Complete JSON example (adapter-level error):**

```json
{
  "v": 1,
  "adapter_id": "bravepi:0",
  "ts": 1743206601000,
  "device_key": null,
  "error": "I2C bus /dev/i2c-1 became unavailable"
}
```

#### 1.3.5 Inventory Topic

The `iotkit/v1/{adapter_id}/inventory/{device_key}` topic carries the same payload as a Discovery envelope (Section 1.3.2). No additional fields. The payload is identical to what was published to the non-retained `discovery` topic for that device.

When a DeviceLost event is processed, the broker-side retained message for the device's inventory topic is cleared by publishing an empty payload (`b""`) with `retain = true` to that topic.

#### 1.3.6 Status Envelope

Published to `iotkit/v1/{adapter_id}/status` with `retain = true`, `QoS = 1`.

Additional fields:

| Field | JSON type | Nullable | Description |
|---|---|---|---|
| `online` | boolean | No | `true` when the adapter comes online; `false` when it goes offline. |

**Timestamp semantics:**

- **Graceful offline** (adapter shutting down cleanly): `ts = now_ms()` at the moment of shutdown.
- **LWT (Last Will and Testament)** — abnormal disconnect: `ts = 0`. The broker publishes this on the adapter's behalf. The `ts = 0` sentinel allows consumers to distinguish a broker-injected LWT from a gracefully sent offline message.
- **Online**: `ts = now_ms()` at startup.

Status messages MUST be decoded with `decode_status()`, not `decode_event()`. `decode_event()` returns `DecodeError::InvalidPayload` if called with a status payload.

**Complete JSON example (online):**

```json
{
  "v": 1,
  "adapter_id": "bravepi:0",
  "ts": 1743206000000,
  "online": true
}
```

**Complete JSON example (graceful offline):**

```json
{
  "v": 1,
  "adapter_id": "bravepi:0",
  "ts": 1743209600000,
  "online": false
}
```

**Complete JSON example (LWT / abnormal disconnect):**

```json
{
  "v": 1,
  "adapter_id": "bravepi:0",
  "ts": 0,
  "online": false
}
```

### 1.4 Validation Rules for Decode

All decode functions MUST apply these validation rules in the order listed. The first failing rule terminates decoding and returns the corresponding error variant.

#### 1.4.1 Universal Rules (apply to all decode paths)

1. **JSON parse failure** -> `DecodeError::Json(serde_json::Error)`
2. **Unknown version** -- `v` field is present but `v != 1` -> `DecodeError::UnknownVersion(v)` where `v` is the parsed integer value.
3. **Missing required field** -- any required field (including `v`, `adapter_id`, `ts`) is absent or has the wrong JSON type -> `DecodeError::InvalidPayload(description)`.

#### 1.4.2 Timestamp Rules

4. **Negative `ts`** -- `ts < 0` -> `DecodeError::InvalidTimestamp(ts)`. Exception: `ts = 0` is valid only in status envelopes decoded via `decode_status()`. `decode_status()` accepts `ts = 0` without error; it does not accept other negative values.
5. **Negative `ingested_at`** -- in telemetry envelopes, `ingested_at < 0` -> `DecodeError::InvalidTimestamp(ingested_at)`.

#### 1.4.3 Telemetry-Specific Rules

6. **Label/value length mismatch** -- `labels.len() != values.len()` -> `DecodeError::InvalidPayload("labels length N does not match values length M")`.

#### 1.4.4 Routing Rules

7. **Status via wrong function** -- `decode_event()` MUST NOT be called on status payloads. Since status payloads lack the fields required for any `AdapterEvent` variant, the JSON parse step will produce `DecodeError::InvalidPayload`. Callers are responsible for routing by topic; the API does not auto-detect payload type.
8. **`EventType::Inventory`** -- `decode_event(EventType::Inventory, payload)` is equivalent to `decode_event(EventType::Discovery, payload)`. The `Inventory` variant exists for topic construction only; it does not change decode behavior.

#### 1.4.5 Unknown Fields

Unknown fields in the JSON object MUST be silently ignored (`#[serde(deny_unknown_fields)]` MUST NOT be used). This preserves forward compatibility when new optional fields are added in future minor revisions.

### 1.5 Adapter Event Mapping

This table defines how each `AdapterEvent` variant is handled by `encode_event()` and what side effects the caller (e.g. the MQTT adapter bridge) must perform.

| `AdapterEvent` variant | `encode_event()` output | `EventType` | Caller side effects |
|---|---|---|---|
| `SensorData` | Encoded | `Telemetry` | Publish to `telemetry` topic (non-retained). |
| `DeviceDiscovered` | Encoded | `Discovery` | Publish to `discovery` topic (non-retained). Also publish the same payload to `inventory/{device_key}` (retained). |
| `DeviceLost` | Encoded | `Loss` | Publish to `loss` topic (non-retained). Also publish empty payload `b""` to `inventory/{device_key}` (retained) to clear the tombstone. |
| `AdapterError` | Encoded | `Error` | Publish to `error` topic (non-retained). `device_key` field is nullable. |
| `DeviceConfig` | Dropped | -- | `encode_event()` returns `Err(EncodeError::UnsupportedEvent("DeviceConfig"))`. Caller MUST log this at `debug` level and discard. No MQTT publish is performed. Rationale: output/actuator device configuration is out of scope for this version; `DeviceConfig` events carry no telemetry and have no defined MQTT representation. |

**Note on `labels`:** The `SensorData` variant carries `labels: Vec<String>`. These are emitted as-is into the `labels` array of the telemetry envelope. The caller does not transform or filter labels.

### 1.6 Design Decisions

**Why JSON:** Easy to debug. Binary formats (MessagePack, etc.) considered only when throughput becomes an issue. 10 sensors x 1Hz = 10 msg/sec, JSON 1msg ~ 200 bytes -> 2KB/sec. Mosquitto local throughput is 10,000+ msg/sec, giving 99.9%+ headroom.

**Why QoS 1:** At-least-once delivery. QoS 0 risks data loss on unstable networks. QoS 2 has excessive overhead. Gateway-side idempotent writes (ON CONFLICT DO NOTHING) already handle duplicates safely.

**Why adapter_id in topic:** Gateway can subscribe/filter per adapter. Wildcard subscribe (`iotkit/v1/+/telemetry`) receives all adapters.

**Rejected: per-device telemetry topic (`iotkit/v1/{adapter_id}/{device_key}/telemetry`):** Topic count scales linearly with sensor count. Per-adapter aggregation is more scalable. Inventory uses per-device retained topics because inventory recovery requires it.

**Why per-device retained inventory topics:** Adapter can update/delete inventory per device. Subscribers can wildcard subscribe (`iotkit/v1/+/inventory/+`) to get all devices from all adapters. A single adapter-level inventory snapshot would require republishing the entire inventory on any single device change.

### 1.7 `core/types` Changes

#### 1.7.1 `SensorReading.labels`: `Vec<&'static str>` -> `Vec<String>`

**Change:** The `labels` field on `SensorReading` (within `AdapterEvent::SensorData`) changes from `Vec<&'static str>` to `Vec<String>`.

**Justification:**

Labels are semantic names for sensor value slots (e.g. `"temperature_c"`, `"humidity_pct"`). In the current model they are `&'static str`, implying they are compile-time constants. This creates a mismatch in two places:

1. **MQTT decode path:** When deserializing a telemetry envelope, labels arrive as heap-allocated JSON strings. Mapping them to `&'static str` requires either a static lookup table (fragile, non-exhaustive) or unsafe transmutation (unsound). `Vec<String>` is the correct owned type for deserialized data.
2. **Future persistence:** Any storage or forwarding layer that persists `SensorReading` values must own the label strings. `&'static str` cannot be serialized into a database or transmitted across a process boundary without conversion.

This is a **domain correction**: labels are data values, not compile-time constants. Sensor drivers that currently produce `&'static str` literals will pass them through `String::from()` (or use string literal coercion), which is a zero-logic change at the call site and carries no semantic cost.

**Rejected alternative -- `Cow<'static, str>`:** `Cow` preserves the static borrow optimization for drivers but adds type complexity throughout the codebase and still requires `.into_owned()` at decode boundaries. The optimization is premature; label strings are short and infrequently allocated.

**Migration:** All call sites that construct `SensorReading` with `labels: vec!["foo", "bar"]` continue to compile after adding `.to_string()` or by relying on `String::from` coercion via `Into<String>`. No semantic changes to existing adapters are required.

#### 1.7.2 `ConnectionKind`: Add `as_str()` and `from_str()` Methods

**Change:** Add two methods to `ConnectionKind`:

```rust
impl ConnectionKind {
    /// Returns the canonical lowercase string identifier for this variant.
    /// Used when serializing the `connection.kind` field in Discovery envelopes.
    ///
    /// | Variant      | Return value     |
    /// |-------------|------------------|
    /// | Uart        | `"uart"`         |
    /// | I2c         | `"i2c"`          |
    /// | Gpio        | `"gpio"`         |
    /// | Modbus      | `"modbus"`       |
    /// | Other(s)    | `s` (passthrough)|
    pub fn as_str(&self) -> &str;

    /// Parse a `ConnectionKind` from its canonical string identifier.
    /// Case-sensitive. Returns `Other(s.to_string())` for unrecognised strings
    /// (never fails — unknown kinds are captured via `Other`).
    ///
    /// Round-trip: `ConnectionKind::from_str(k.as_str()) == k` for all values.
    pub fn from_str(s: &str) -> Self;
}
```

**Justification:** The `connection.kind` field in Discovery envelopes is serialized as a freeform string. Without `as_str()` / `from_str()`, each serialization site must independently implement the string mapping, risking inconsistency. Centralizing the mapping in `core/types` ensures the encode and decode paths are always symmetric and that adding a new `ConnectionKind` variant forces a single update point.

**Note:** These methods do not replace `serde` derive attributes on `ConnectionKind`; `serde` serialization for other use cases remains unchanged. `as_str()` and `from_str()` are used explicitly by the `mqtt-contract` encode/decode logic for the `connection.kind` field.

### 1.8 Public API (Rust Signatures)

The `mqtt-contract` crate exposes a pure data encoding/decoding interface. It has no runtime dependency (no tokio, no async, no MQTT client).

```rust
use iotkit_types::{AdapterId, AdapterEvent, DeviceKey};

// ---------------------------------------------------------------------------
// EventType
// ---------------------------------------------------------------------------

/// Identifies which topic/envelope type a message belongs to.
/// `Inventory` is included for topic construction; it shares decode logic with `Discovery`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    Telemetry,
    Discovery,
    Loss,
    Error,
    Status,
    Inventory,
}

impl EventType {
    /// Returns the topic path segment for this event type (e.g. `"telemetry"`).
    /// `Inventory` returns `"inventory"` (the base segment; the device_key suffix
    /// is appended separately by `inventory_topic()`).
    pub fn as_str(&self) -> &'static str;
}

// ---------------------------------------------------------------------------
// Topic building
// ---------------------------------------------------------------------------

/// Build the topic for a non-inventory event type.
/// Panics if called with `EventType::Inventory`; use `inventory_topic()` instead.
///
/// Example: `topic(&adapter_id, EventType::Telemetry)`
///   -> `"iotkit/v1/bravepi%3A0/telemetry"`
pub fn topic(adapter_id: &AdapterId, event_type: EventType) -> String;

/// Build the per-device inventory topic.
///
/// Example: `inventory_topic(&adapter_id, &device_key)`
///   -> `"iotkit/v1/bravepi%3A0/inventory/i2c%3A0x44%3Asht31"`
pub fn inventory_topic(adapter_id: &AdapterId, device_key: &DeviceKey) -> String;

/// Percent-encode a single topic segment value.
/// Encodes `:` -> `%3A`, `/` -> `%2F`, `+` -> `%2B`, `#` -> `%23`, `%` -> `%25`.
/// All other bytes are passed through unchanged.
pub fn encode_topic_segment(s: &str) -> String;

/// Decode a percent-encoded topic segment back to its original value.
/// Returns `Err` if the input contains a malformed percent sequence.
pub fn decode_topic_segment(s: &str) -> Result<String, DecodeError>;

// ---------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------

/// Encode an `AdapterEvent` into `(EventType, JSON payload bytes)`.
///
/// Returns `Err(EncodeError::UnsupportedEvent)` for `AdapterEvent::DeviceConfig`.
/// The `ts` field is set to the current wall-clock time in Unix milliseconds.
///
/// The caller is responsible for:
/// - Publishing to the correct topic (use `topic()` / `inventory_topic()`).
/// - Setting `retain = true` for `EventType::Discovery` inventory publishes and
///   `EventType::Status`.
/// - Publishing the inventory tombstone on `EventType::Loss`.
pub fn encode_event(
    adapter_id: &AdapterId,
    event: &AdapterEvent,
) -> Result<(EventType, Vec<u8>), EncodeError>;

/// Encode a status message.
///
/// `ts`: Unix milliseconds. Pass `0` for LWT (broker-injected offline).
///        Pass `now_ms()` for graceful offline and for online messages.
/// `online`: `true` = adapter online, `false` = adapter offline.
///
/// Always returns a `Vec<u8>` (infallible; status payloads are statically structured).
pub fn encode_status(adapter_id: &AdapterId, online: bool, ts: i64) -> Vec<u8>;

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/// Decode a non-status event payload.
///
/// `event_type`: The type inferred from the MQTT topic. MUST NOT be `EventType::Status`.
///               `EventType::Inventory` is treated identically to `EventType::Discovery`.
///
/// Returns `(adapter_id, adapter_event)` on success.
///
/// Returns `DecodeError` if:
/// - The payload is not valid UTF-8 JSON.
/// - The `v` field is not `1`.
/// - Any required field is missing or has the wrong type.
/// - `ts < 0` or `ingested_at < 0`.
/// - `labels.len() != values.len()` for telemetry payloads.
pub fn decode_event(
    event_type: EventType,
    payload: &[u8],
) -> Result<(AdapterId, AdapterEvent), DecodeError>;

/// Decode a status payload.
///
/// Returns `(adapter_id, online, ts)` on success.
/// `ts` is the Unix timestamp in milliseconds. `ts = 0` is accepted (LWT sentinel).
/// Other negative `ts` values are rejected.
///
/// The caller needs `ts` to distinguish LWT offline (ts=0) from graceful offline (ts>0)
/// and to record when the adapter went online/offline.
///
/// MUST be called for payloads received on the `.../status` topic.
/// MUST NOT be used for other event types.
pub fn decode_status(payload: &[u8]) -> Result<(AdapterId, bool, i64), DecodeError>;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    /// The `AdapterEvent` variant has no defined MQTT encoding.
    /// The inner `String` is the variant name (e.g. `"DeviceConfig"`).
    #[error("unsupported event variant: {0}")]
    UnsupportedEvent(String),

    /// JSON serialization failed.
    #[error("json encode error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// JSON deserialization failed (includes UTF-8 errors via serde_json).
    #[error("json decode error: {0}")]
    Json(#[from] serde_json::Error),

    /// The `v` field was present but not equal to `1`.
    /// The inner `u32` is the version value found in the payload.
    #[error("unknown envelope version: {0}")]
    UnknownVersion(u32),

    /// A `ts` or `ingested_at` field had a negative value (other than the LWT `ts = 0` sentinel).
    /// The inner `i64` is the invalid value found in the payload.
    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(i64),

    /// A structural or semantic constraint was violated.
    /// The inner `String` is a human-readable description.
    /// Examples: missing required field, labels/values length mismatch,
    ///           malformed percent-encoded topic segment.
    #[error("invalid payload: {0}")]
    InvalidPayload(String),
}
```

### 1.9 Crate Dependencies

#### 1.9.1 `core/mqtt-contract` Dependency Graph

```
core/mqtt-contract
├── core/types          (workspace member; AdapterId, AdapterEvent, DeviceKey, SensorReading, ConnectionKind)
├── serde               (derive feature: Serialize, Deserialize)
├── serde_json          (JSON encode/decode)
├── percent-encoding    (topic segment encode/decode)
└── thiserror           (EncodeError, DecodeError derive)
```

#### 1.9.2 Explicit Non-Dependencies

| Crate | Reason excluded |
|---|---|
| `rumqttc` | MQTT client/transport. `mqtt-contract` is pure data; it does not open connections or send packets. |
| `tokio` | Async runtime. All functions are synchronous. No I/O is performed. |
| `async-trait` | No async traits in this crate. |
| `log` / `tracing` | Logging of dropped `DeviceConfig` events is the caller's responsibility. `mqtt-contract` does not log. |

#### 1.9.3 Rationale for Pure-Data Design

Keeping `mqtt-contract` free of transport and runtime dependencies means:

- It can be used in test harnesses, offline tooling, and simulators without a running broker.
- Unit tests for encode/decode correctness run without any async executor.
- Downstream crates choose their own MQTT client and runtime independently of this crate.

---

## 2. Runner State Machine, Task Model, and Supervision (`iotkit-adapter-runner`)

### 2.1 Runner State Machine

The runner (`adapter_runner::run()`) does NOT own the adapter or handle signals. It receives `event_rx` and processes events until the channel closes. The **binary** (main.rs) owns the adapter `ShutdownHandle`, signal handlers, and the decision of when to shutdown.

#### 2.1.1 Runner States (internal to `run()`)

```dot
digraph runner_states {
    rankdir=TB;
    node [shape=box, style=rounded, fontname="monospace"];
    edge [fontname="monospace", fontsize=10];

    Connecting [label="Connecting\n(MQTT client init)"];
    Reconnecting [label="Reconnecting\n(waiting for ConnAck)"];
    Online [label="Online\n(publishing events)"];
    Draining [label="Draining\n(event_rx closed,\nflushing buffer)"];
    Done [label="Done", shape=doubleoctagon];
    Failed [label="Failed", shape=doubleoctagon];

    Connecting -> Reconnecting [label="MQTT client created,\neventloop_task spawned\n/ enter event loop"];
    Connecting -> Failed [label="MQTT client creation fails\n/ return Err"];

    Reconnecting -> Online [label="ConnAck received\n/ connack_notify,\nconnected.store(true)"];
    Reconnecting -> Draining [label="event_rx closed\n/ begin drain"];
    Reconnecting -> Failed [label="eventloop_task exits\nunexpectedly\n/ return Err"];

    Online -> Reconnecting [label="Disconnect detected\n/ connected.store(false)"];
    Online -> Draining [label="event_rx closed\n/ begin drain"];
    Online -> Failed [label="eventloop_task exits\nunexpectedly\n/ return Err"];

    Draining -> Done [label="drain completes\n+ offline status published\n/ return Ok"];
    Draining -> Done [label="drain timeout (2s)\n/ return Ok (best-effort)"];
}
```

The runner NEVER initiates shutdown. The runner NEVER knows about signals. It simply processes events until `event_rx` closes, then drains its buffer and publishes offline status.

#### 2.1.2 Binary States (main.rs lifecycle)

```dot
digraph binary_states {
    rankdir=TB;
    node [shape=box, style=rounded, fontname="monospace"];
    edge [fontname="monospace", fontsize=10];

    ValidatingConfig [label="ValidatingConfig"];
    Starting [label="Starting\n(adapter + runner)"];
    Running [label="Running\n(adapter producing,\nrunner consuming)"];
    ShuttingDown [label="ShuttingDown\n(adapter stopping,\nwaiting for runner)"];
    Exit0 [label="Exit(0)", shape=doubleoctagon];
    Exit1 [label="Exit(1)", shape=doubleoctagon];

    ValidatingConfig -> Starting [label="config valid"];
    ValidatingConfig -> Exit1 [label="config invalid\n/ log error"];

    Starting -> Running [label="adapter started\n+ runner spawned"];
    Starting -> Exit1 [label="adapter start fails\nOR MQTT init fails\n/ log error"];

    Running -> ShuttingDown [label="SIGINT/SIGTERM received\n/ call adapter.shutdown()"];
    Running -> Exit1 [label="runner returns Err\n(MQTT fatal error)"];

    ShuttingDown -> Exit0 [label="runner returns Ok\n(clean drain)"];
    ShuttingDown -> Exit1 [label="adapter shutdown timeout (5s)\nOR 2nd signal\nOR runner returns Err"];
}
```

#### 2.1.3 Ownership Boundary Summary

| Component | Binary (main.rs) owns | Runner (`run()`) owns |
|---|---|---|
| Adapter ShutdownHandle | YES | NO |
| Signal handlers (SIGINT/SIGTERM) | YES | NO |
| Shutdown decision | YES | NO |
| MQTT client lifecycle | NO | YES |
| event_rx consumption | NO | YES |
| Inventory tracking | NO | YES |
| Buffer management | NO | YES |
| Offline status publish | NO | YES (on drain) |

#### 2.1.4 Shutdown Flow

1. Binary catches SIGINT/SIGTERM.
2. Binary calls `adapter_handle.shutdown().await` with a 5-second timeout. This stops the adapter producer and drops `event_tx`, causing `event_rx` to close.
3. Runner detects `event_rx` closure (recv returns None) -> enters Draining state -> flushes buffer -> publishes offline status -> returns `Ok(())`.
4. Binary checks runner result -> exit 0 on Ok, exit 1 on Err.
5. If adapter shutdown hangs beyond 5 seconds, binary aborts and exits 1.
6. If a 2nd signal arrives during shutdown, binary calls `std::process::exit(1)` immediately.

#### Transition Table (Runner)

| From | To | Trigger | Actions |
|---|---|---|---|
| Connecting | Reconnecting | MQTT client + eventloop created | Spawn eventloop_task, spawn publish_task, enter event loop |
| Connecting | Failed | MQTT client creation fails | Return `Err(RunnerError::MqttInit(...))` |
| Reconnecting | Online | ConnAck received by eventloop_task | `connack_notify.notify_one()`, `connected.store(true, Release)` |
| Online | Reconnecting | Disconnect detected by eventloop_task | `connected.store(false, Release)` |
| Reconnecting/Online | Draining | `event_rx` closes (recv returns None) | publish_task exits recv loop, begins drain |
| Reconnecting/Online | Failed | eventloop_task exits unexpectedly | Return `Err(RunnerError::EventLoopDied)` |
| Draining | Done | Drain completes + offline status published | Return `Ok(())` |
| Draining | Done | Drain timeout (2s) exceeded | Return `Ok(())` (best-effort; LWT serves as fallback) |

#### Transition Table (Binary)

| From | To | Trigger | Actions |
|---|---|---|---|
| ValidatingConfig | Starting | Config valid | Create tokio runtime, construct MQTT client with LWT |
| ValidatingConfig | Exit(1) | Config invalid | Log validation error |
| Starting | Running | Adapter started + runner spawned | Enter signal wait loop |
| Starting | Exit(1) | Any init step fails | Log error, exit |
| Running | ShuttingDown | 1st SIGINT/SIGTERM | Call `adapter_handle.shutdown()` with 5s timeout |
| Running | Exit(1) | Runner returns Err (before signal) | Log error, exit |
| ShuttingDown | Exit(0) | Runner returns Ok | Clean exit |
| ShuttingDown | Exit(1) | Adapter shutdown timeout (5s) / 2nd signal / runner Err | Exit immediately |

### 2.2 Task Model and State Ownership

The runner spawns two internal tasks. Each task has **exclusive ownership** of its state. No `Mutex` or `RwLock` exists. Cross-task coordination uses only `Arc<AtomicBool>` and `Arc<Notify>`.

**Important:** The runner does NOT spawn signal handlers or own the adapter's ShutdownHandle. Signal handling and adapter lifecycle are the binary's responsibility (see Section 2.1.2).

#### 2.2.1 eventloop_task (spawned tokio task)

**Owns:**
- `rumqttc::EventLoop` (the MQTT event loop instance)

**Writes (shared atomic):**
- `connected: Arc<AtomicBool>` -- sets `true` on ConnAck, `false` on disconnect/error

**Signals:**
- `connack_notify: Arc<Notify>` -- calls `notify_one()` on each ConnAck

**Behavior:**
- Runs `eventloop.poll().await` in a loop.
- On `Event::Incoming(Packet::ConnAck(_))`: stores `connected = true` (Release), calls `connack_notify.notify_one()`.
- On connection error or disconnect: stores `connected = false` (Release).
- rumqttc handles reconnection internally; this task does not exit on transient disconnects.
- Task exits only when: (a) `EventLoop` is dropped/aborted by the runner on cleanup, or (b) an unrecoverable internal error.

**Return type:** `Result<(), EventLoopError>` -- runner inspects this on join.

#### 2.2.2 publish_task (spawned tokio task)

**Owns (exclusive, moved in):**
- `event_rx: mpsc::Receiver<AdapterEvent>` -- receives adapter events
- `desired_inventory: HashMap<String, Option<Vec<u8>>>` -- sole inventory model (see Section 3.8)
- `outbound_buffer: VecDeque<PublishItem>` -- buffered publishes for when disconnected
- `client: rumqttc::AsyncClient` (clone) -- used for `publish()` / `publish_bytes()` calls

**Thread safety of `desired_inventory`:** This HashMap is exclusively owned by publish_task. No sharing, no Mutex needed.

**Reads (shared atomic):**
- `connected: Arc<AtomicBool>` -- checks before attempting publish

**Awaits:**
- `connack_notify: Arc<Notify>` -- waits for connectivity to flush inventory and outbound buffer

**Behavior loop:**
```
loop {
    tokio::select! {
        event = event_rx.recv() => {
            match event {
                Some(ev) => process_event(ev),  // buffer or publish
                None => break,                   // channel closed, exit task
            }
        }
        _ = connack_notify.notified(), if !desired_inventory.is_empty()
                                         || !outbound_buffer.is_empty() => {
            flush_pending();
        }
    }
}
// After loop: drain phase
drain_remaining();
```

- When `connected.load(Acquire)` is `false`: enqueue publishes to `outbound_buffer`.
- When `connected` is `true` and `connack_notify` fires: replay everything in `desired_inventory` first (retained), then drain `outbound_buffer` (telemetry).
- On `event_rx` closed (`recv()` returns `None`): exit the loop. This is the shutdown signal propagated from the binary via the adapter stopping.

**Drain timeout:** The drain phase has an internal 2-second timeout. If drain takes longer, it returns anyway (best-effort). The LWT serves as fallback for offline notification.

**What if ConnAck arrives during drain?** Ignored. Once in drain mode, the publish_task does not re-enter the select! loop. It flushes what it can and exits.

**Return type:** `Result<(), PublishTaskError>` -- runner inspects on join.

#### 2.2.3 runner main task (the `run()` async fn)

**Owns (exclusive):**
- `eventloop_join: JoinHandle<Result<(), EventLoopError>>` -- eventloop_task handle
- `publish_join: JoinHandle<Result<(), PublishTaskError>>` -- publish_task handle
- `client: rumqttc::AsyncClient` (clone) -- for offline status publish during shutdown

**Does NOT own:**
- Signal handlers (binary's responsibility)
- Adapter ShutdownHandle (binary's responsibility)
- `shutdown_initiated` flag (not needed -- runner simply processes until event_rx closes)

**Behavior:**
```
// Wait for publish_task to complete (it exits when event_rx closes)
let publish_result = publish_join.await;

// Publish offline status
client.publish(status_topic, QoS1, retained=true, offline_payload).await;
client.disconnect().await;

// Grace period: let eventloop transmit the above
timeout(2s, eventloop_join).await;
// If eventloop didn't finish, abort it
eventloop_join.abort();

// Return Ok or Err based on publish_result
```

#### 2.2.4 Shared State Summary

| Item | Type | Writer | Reader(s) |
|---|---|---|---|
| `connected` | `Arc<AtomicBool>` | eventloop_task | publish_task |
| `connack_notify` | `Arc<Notify>` | eventloop_task | publish_task |

No other shared mutable state exists. `AsyncClient` is cloned (rumqttc's `AsyncClient` is backed by an internal channel and is `Clone + Send`); each clone is independently owned.

`desired_inventory` is exclusively owned by publish_task -- no sharing needed.

### 2.3 Task Supervision

#### 2.3.1 Runner-Internal Task Monitoring

The runner's `run()` function monitors its two internal tasks:

| Source | Monitoring method | Condition |
|---|---|---|
| publish_task exit | `publish_join.await` | JoinHandle resolves (normal: event_rx closed) |
| eventloop_task exit | `eventloop_join.await` (after publish_task) | JoinHandle resolves |

The runner does NOT monitor signals. Signal handling is the binary's responsibility.

#### 2.3.2 Runner Return Values

| Event | Runner action | Return value |
|---|---|---|
| publish_task exits Ok (event_rx closed) | Publish offline status, gracefully stop eventloop | `Ok(())` |
| publish_task exits Err/panic | Log error, abort eventloop | `Err(RunnerError::PublishTaskFailed)` |
| eventloop_task exits unexpectedly (before publish_task) | publish_task will detect via publish failures | `Err(RunnerError::EventLoopDied)` |

#### 2.3.3 Binary-Level Supervision

The binary monitors both the runner and signals:

| Event | Binary action | Exit code |
|---|---|---|
| 1st signal (SIGINT/SIGTERM) | Call `adapter_handle.shutdown()` with 5s timeout; wait for runner to return | 0 if runner Ok |
| 2nd signal during shutdown | `std::process::exit(1)` immediately | 1 |
| Adapter shutdown timeout (5s) | Abort runner task, exit | 1 |
| Runner returns Ok (before signal) | Unexpected -- adapter died; exit | 1 |
| Runner returns Err (before signal) | Log error, exit | 1 |
| Runner returns Ok (after signal) | Clean shutdown | 0 |
| Runner returns Err (after signal) | Log error, exit | 1 |

#### 2.3.4 Panic Propagation

Both runner-internal tasks use `JoinHandle`. If a task panics, `JoinHandle::await` returns `Err(JoinError)` where `JoinError::is_panic() == true`. The runner logs the panic payload and returns `Err`. Panics are never caught or restarted.

### 2.4 Startup Sequence

Startup is split between the binary and the runner. The binary handles steps 1-5 (config, runtime, adapter); the runner handles steps 6-8 (MQTT, tasks).

#### 2.4.1 Binary Startup (main.rs)

```
(1) Config load + validate
        |
        v  [fail: log error, exit 1 (EX_CONFIG=78)]
(2) Create tokio runtime (Runtime::new())
        |
        v  [fail: panic -- unrecoverable, no runtime to log]
(3) Install signal handlers (SIGINT, SIGTERM)
        |
        v  [fail: panic -- unrecoverable]
(4) Start adapter
    - Call adapter's start() function, obtain event_rx and ShutdownHandle
    - For polling adapters: validates bus_path accessibility
        |
        v  [fail: log error, exit 1]
(5) Spawn runner: tokio::spawn(adapter_runner::run(adapter_id, mqtt_config, event_rx))
        |
        v
(6) Enter binary event loop: select! on { signal, runner_join }
```

#### 2.4.2 Runner Startup (inside `run()`)

```
(1) Create MQTT client + EventLoop
    - MqttOptions with LWT: topic=`{prefix}/status`, payload=`{"v":1,"adapter_id":"...","ts":0,"online":false}`, retain=true, QoS 1
    - AsyncClient::new(options, cap=256)
        |
        v  [fail: return Err(RunnerError::MqttInit(...))]
(2) Spawn eventloop_task
    - tokio::spawn(eventloop_run(eventloop, connected, connack_notify))
    - Returns JoinHandle
        |
        v  [fail: impossible -- spawn does not fail]
(3) Spawn publish_task
    - tokio::spawn(publish_run(event_rx, client.clone(), connected, connack_notify, ...))
    - Returns JoinHandle
        |
        v  [fail: impossible -- spawn does not fail]
(4) Await publish_task completion (event_rx closure or error)
    - State: Reconnecting (waiting for first ConnAck)
```

**Runner init failure:** If MQTT client creation fails, `run()` returns `Err` immediately. The binary receives this, logs the error, shuts down the adapter, and exits 1.

### 2.5 Shutdown Sequence

Shutdown is a coordinated sequence between the binary and the runner.

#### 2.5.1 Binary-Side Shutdown (triggered by signal)

```
(1) Signal received (SIGINT or SIGTERM)
    |
    v
(2) Binary calls adapter_handle.shutdown() with 5s timeout
    - This sends AdapterCommand::Shutdown to the adapter
    - Adapter stops producing events, drops event_tx
    - event_rx yields None in runner's publish_task
    |
    | timeout: 5 seconds
    v  [timeout: abort runner task, exit 1]
(3) Binary awaits runner_join (runner::run() returning)
    |
    v  [Ok -> exit 0, Err -> exit 1]
```

#### 2.5.2 Runner-Side Shutdown (triggered by event_rx closure)

The runner never initiates shutdown. It reacts to `event_rx` closing:

```
(1) publish_task detects event_rx.recv() == None
    |
    v
(2) Drain phase: flush outbound_buffer
    - If connected: publish remaining buffered events
    - If disconnected: skip (data loss accepted on shutdown while disconnected)
    |
    | internal timeout: 2 seconds
    v
(3) Offline status publish
    - runner publishes retained status: {"v":1,"adapter_id":"...","ts":<real_unix_ms>,"online":false}
    - Topic: {prefix}/status, QoS 1, retain=true
    - This overwrites the LWT's ts=0 with a real timestamp
    |
    v
(4) Disconnect
    - client.disconnect().await (best effort)
    |
    v
(5) Eventloop grace period (2 seconds)
    - Allow eventloop to flush offline status + DISCONNECT to TCP
    |
    | timeout: 2 seconds
    v
(6) Eventloop abort
    - If eventloop_join has not completed: eventloop_join.abort()
    |
    v
(7) Return Ok(()) to binary
```

**Can runner return Ok vs Err?**
- `Ok(())` = event_rx closed cleanly, drain completed (or timed out, best-effort).
- `Err(RunnerError)` = MQTT fatal error at startup, or eventloop/publish_task panicked.

**What happens to eventloop_task when runner returns?** The runner aborts it in step 6. The binary does not need to manage it.

#### Timeout Budget

| Phase | Owner | Max duration |
|---|---|---|
| Adapter shutdown | Binary | 5 seconds |
| Publish drain | Runner | 2 seconds |
| Offline status + disconnect | Runner | Within drain window |
| Eventloop grace | Runner | 2 seconds |
| Eventloop abort | Runner | ~0 (immediate) |
| **Total maximum** | | **5 + 4 = 9 seconds** |

Note: The binary's 5s adapter timeout runs concurrently with (not before) the runner's drain. In practice, once the adapter drops event_tx, the runner begins draining immediately, so the actual wall time is typically 5s + small overlap.

#### 2.5.3 2nd Signal During Shutdown

If a second SIGINT or SIGTERM arrives while the binary's shutdown sequence is in progress, the binary calls `std::process::exit(1)` immediately. This bypasses all remaining drain/flush steps. The LWT (with ts=0) will inform subscribers of the ungraceful exit.

#### 2.5.4 Adapter Shutdown Hangs

If `adapter_handle.shutdown()` does not complete within 5 seconds, the binary:
1. Logs a warning: "adapter shutdown timed out after 5s"
2. Aborts the runner task.
3. Exits with code 1.

The LWT fires after the broker's keepalive timeout (default 30s), providing eventual offline notification.

### 2.6 Exit Code Contract

| Scenario | Exit code | Rationale |
|---|---|---|
| Config validation failure | 1 (78 EX_CONFIG) | Cannot run with invalid config |
| Adapter start failure | 1 | No data source |
| Runner returns Err (MQTT init failure) | 1 | Cannot connect to broker |
| Runner returns Err (task panic/crash) | 1 | Critical infrastructure gone |
| Signal + runner returns Ok (clean drain) | 0 | Graceful shutdown |
| Signal + adapter shutdown timeout (5s) | 1 | Adapter hung |
| 2nd signal during shutdown | 1 | Forced exit |
| Runner returns Ok before signal (adapter died) | 1 | Data source died unexpectedly |

Implementation: main returns `ExitCode` (or calls `std::process::exit`). The `fn main()` signature is:

```rust
fn main() -> ExitCode {
    // ... config, runtime, rt.block_on(run(...))
}
```

### 2.7 Failure Classification

#### 2.7.1 Recoverable: MQTT disconnect

- **Detection:** eventloop_task observes connection error from `eventloop.poll()`.
- **Action:** `connected.store(false, Release)`. rumqttc reconnects automatically with exponential backoff.
- **publish_task behavior:** Stops publishing, buffers to `outbound_buffer`. Waits on `connack_notify`.
- **No task exits.** No supervision intervention. The runner remains in `Reconnecting` state.

#### 2.7.2 Per-device: sensor read failure

- **Detection:** Adapter sends `AdapterEvent::AdapterError { device_key, error, .. }` through `event_rx`.
- **publish_task behavior:** Publishes the error to the device's error topic. Does not affect other devices. Does not trigger shutdown.
- **Runner state:** Unchanged (stays Online or Reconnecting).

#### 2.7.3 Process-fatal

| Failure | Detection point | Action |
|---|---|---|
| Config error | Binary step 1 (before runtime) | Log, exit 1 |
| Adapter start failure | Binary step 4 | Log, exit 1 |
| MQTT client init failure | Runner step 1 | Runner returns Err, binary logs + exits 1 |
| eventloop_task exit (unexpected) | Runner detects via publish failures | Runner returns Err, binary logs + exits 1 |
| publish_task panic | Runner detects via JoinError | Runner returns Err, binary logs + exits 1 |
| Adapter crash (event_rx closes without signal) | Runner returns Ok, binary detects no signal preceded it | Binary logs + exits 1 |

Process-fatal failures are never retried. The process exits and the service manager (systemd) is responsible for restart policy.

### 2.8 event_rx Closure Semantics

The `event_rx` channel (from adapter to publish_task) can close in two scenarios. The **binary** distinguishes them via a local `shutdown_initiated: bool` flag. The runner does not distinguish -- it always drains and returns Ok.

#### 2.8.1 Binary State Tracking

```rust
// In binary main, before signal loop:
let mut shutdown_initiated = false;
```

`shutdown_initiated` is set to `true` **only** when:
- A signal is received and the binary calls `adapter_handle.shutdown()`.

It is never set by the runner. It is a binary-local variable.

#### 2.8.2 Clean Shutdown Path

```
signal received
  -> binary sets shutdown_initiated = true
  -> binary calls adapter_handle.shutdown().await (adapter stops, drops event_tx)
  -> event_rx.recv() returns None in runner's publish_task
  -> publish_task enters drain phase
  -> runner publishes offline status, cleans up eventloop
  -> runner returns Ok(())
  -> binary checks: shutdown_initiated == true, runner Ok -> exit 0
```

#### 2.8.3 Adapter Crash Path

```
adapter panics / drops event_tx unexpectedly
  -> event_rx.recv() returns None in runner's publish_task
  -> publish_task enters drain phase
  -> runner publishes offline status, cleans up eventloop
  -> runner returns Ok(())
  -> binary checks: shutdown_initiated == false
  -> binary logs "adapter crash: event_rx closed without shutdown signal"
  -> binary exits 1
```

#### 2.8.4 Runner Return Type

The runner always returns `Ok(())` when event_rx closes (regardless of whether closure was clean or a crash). The runner returns `Err` only for internal failures (MQTT init, task panic). The binary decides the exit code based on `shutdown_initiated`:

```rust
// Binary logic (simplified):
match runner_result {
    Ok(()) if shutdown_initiated => ExitCode::SUCCESS,  // clean shutdown
    Ok(()) => { error!("adapter died unexpectedly"); ExitCode::FAILURE },  // crash
    Err(e) => { error!("runner error: {e}"); ExitCode::FAILURE },
}
```

### 2.9 Public API

```rust
/// Run the MQTT adapter runner until event_rx closes.
///
/// The runner creates an MQTT client, spawns internal tasks (eventloop, publish),
/// and processes events from event_rx until the channel closes. On closure,
/// it drains buffered events, publishes offline status, and returns.
///
/// The runner does NOT handle signals or own the adapter. The caller (binary)
/// is responsible for:
/// - Installing signal handlers
/// - Calling adapter shutdown (which closes event_rx)
/// - Interpreting the return value for exit code decisions
///
/// Returns Ok(()) on clean event_rx closure (drain completed).
/// Returns Err on MQTT init failure or internal task crash.
pub async fn run(
    adapter_id: AdapterId,
    mqtt_config: MqttConfig,
    event_rx: mpsc::Receiver<AdapterEvent>,
) -> Result<(), RunnerError>;
```

Dependencies: `core/mqtt-contract`, `core/types`, `rumqttc`, `url`, `tokio`, `tracing`.

Note: `tokio::signal` is NOT a dependency of the runner crate. Signal handling lives in the binary.

---

## 3. MQTT Connection Lifecycle and Protocol (`iotkit-adapter-runner`)

### 3.1 Definition of "Connected"

The runner considers itself **connected** if and only if a `ConnAck` packet has been received from the broker on the current TCP session. The following events do **not** constitute "connected":

- Successful TCP connect (SYN/SYN-ACK/ACK complete)
- `eventloop.poll()` returning `Ok` for any packet other than `ConnAck`
- Successful DNS resolution

The connection state is tracked via a shared `AtomicBool` (`connected`), written with `Ordering::Release` and read with `Ordering::Acquire`. This ensures the publish_task sees a consistent view of the connection state relative to any prior ConnAck processing. In practice, the only consequence of a stale read is a brief period where the publish loop either buffers unnecessarily or attempts a publish that will be enqueued to rumqttc's internal queue (which handles disconnection internally).

### 3.2 Session Model

```
clean_session = true   (always)
```

The runner **never** relies on broker-side persistent sessions. Every `ConnAck` is treated identically whether it is the initial connection or a reconnection. The broker holds no subscription state or undelivered QoS 1 messages for this client across TCP sessions.

**Rationale:** The runner is publish-only. There are no subscriptions to restore. `clean_session=true` eliminates an entire class of broker-side state bugs and simplifies reconnect logic to a single code path.

### 3.3 Connection State Machine

The connection lifecycle within the eventloop_task operates as follows (this is the internal detail of the eventloop_task described in Section 2.2.1):

```dot
digraph connection_lifecycle {
    rankdir=LR;
    node [shape=box, style=rounded];

    Disconnected [label="Disconnected\n(AtomicBool=false)"];
    Connected [label="Connected\n(AtomicBool=true)"];

    Disconnected -> Disconnected [label="poll() -> Err\n(backoff sleep)"];
    Disconnected -> Connected [label="poll() -> ConnAck\n-> store(true)\n-> reset attempt=0\n-> re-publish online status\n-> notify publish_task"];
    Connected -> Connected [label="poll() -> Ok(non-ConnAck)\n(PUBACK, PINGRESP, etc.)"];
    Connected -> Disconnected [label="poll() -> Err\n-> store(false)\n-> increment attempt\n-> backoff sleep"];
}
```

### 3.4 ConnAck Processing (identical for initial and reconnect)

On every `ConnAck`, the following sequence executes **in order**:

1. `connected.store(true, Release)`
2. `reconnect_attempt = 0`
3. **Re-publish online status** -- `encode_status(adapter_id, true, now_ms())` to `iotkit/v1/{adapter_id}/status`, QoS 1, retained.
4. **Notify publish_task** via `reconnect_notify.notify_one()`.

The publish_task, upon receiving the notification, executes:

5. **Inventory reconcile** -- replay all entries in `desired_inventory` as retained publishes (see Section 3.8).
6. **Buffer replay** -- replay up to 10 items from `outbound_buffer` in FIFO order, then return to `select!` loop (see Section 3.7.5).
7. **Resume live event processing** -- the `select!` loop naturally interleaves further buffer replay batches with live events.

Steps 5-6-7 are handled within the publish_task's `tokio::select!` loop. Inventory reconcile runs in full (bounded by device count). Buffer replay is batched (max 10 per iteration) to interleave with live events from `event_rx`.

### 3.5 Reconnect Backoff

| Parameter | Value |
|-----------|-------|
| Base delay | 1000 ms |
| Maximum delay | 30000 ms |
| Growth | Exponential: `base * 2^attempt`, capped at max |
| Jitter | +/- 30% uniform random on the capped value |
| Minimum effective delay | 100 ms (floor after jitter subtraction) |
| Attempt counter | `u32`, saturating add, reset to 0 on `ConnAck` |

Formula: `delay = clamp(base_ms * 2^min(attempt, 15) + uniform(-30%, +30%), 100ms, ...)`.

The backoff is implemented in the eventloop pump task. rumqttc's built-in reconnect is used -- after `poll()` returns `Err`, the next `poll()` call will attempt a new TCP connection.

### 3.6 Infinite Disconnect Tolerance

There is **no maximum reconnect count** and **no timeout** after which the runner gives up. The runner reconnects forever until:

- The binary receives SIGTERM/SIGINT, shuts down the adapter, which closes `event_rx`, causing the runner to drain and exit.
- A fatal configuration error prevents even TCP connect attempts (invalid hostname -- rumqttc will still retry).

During extended disconnection:
- The adapter continues producing events into `event_rx`.
- The publish_task continues consuming from `event_rx`, tracking inventory locally, and buffering non-retained events (subject to the 1000-event cap).
- The eventloop pump task continues calling `poll()` with backoff sleeps.

### 3.7 Publish, Buffer, and Replay Policy

#### 3.7.1 Definition of "Publish Succeeded"

`AsyncClient::publish().await` returning `Ok(())` means the message has been **enqueued into rumqttc's internal bounded channel** (capacity: 100, set at `AsyncClient::new(opts, 100)`). It does **not** mean:

- The message has been written to the TCP socket.
- The message has been received by the broker.
- A `PUBACK` has been received (for QoS 1).

This is a fundamental constraint of rumqttc's API. The runner **must not** treat enqueue success as delivery confirmation for any correctness-critical operation.

#### 3.7.2 Disconnect Buffering Policy by Event Class

Events are classified into two categories based on their MQTT retain semantics and recoverability:

| Event class | Events | Retained? | Disconnect policy | Rationale |
|-------------|--------|-----------|-------------------|-----------|
| **Inventory/Status (recoverable)** | `DeviceDiscovered`, `DeviceLost`, status online/offline | Yes | MUST NOT drop. Tracked in `desired_inventory` (HashMap). Re-published on every ConnAck. | These represent current device state. Dropping would leave broker state inconsistent. Local tracking makes them fully recoverable. |
| **Transient (non-recoverable)** | `SensorData` (telemetry), `AdapterError`, non-retained copies of discovery/loss events | No | Bounded buffer (`outbound_buffer`), capacity 1000. Drop oldest on overflow. | Telemetry is time-series data; stale readings have diminishing value. Bounded buffer prevents OOM on extended disconnects. |

When the buffer is full and a new transient event arrives:

1. `outbound_buffer.pop_front()` -- discard oldest.
2. `warn!("pending event buffer full, dropping oldest event")` -- exactly one log line per drop.
3. `outbound_buffer.push_back((event_type, payload))` -- enqueue new event.

**Buffer capacity constant:**

```rust
const PENDING_BUFFER_CAP: usize = 1000;
```

This is a compile-time constant. There is no runtime configuration for buffer size in v1.

#### 3.7.3 What Gets Buffered vs. What Gets Tracked

When disconnected, for each incoming `AdapterEvent`:

1. **Always:** `inventory.track_event(&event)` -- updates `desired_inventory` HashMap locally.
2. **Then:** `buffer_event(adapter_id, &event, &mut outbound_buffer)` -- encodes the event and pushes to `outbound_buffer` VecDeque.

Both paths execute regardless of event type. Inventory events are tracked in step 1 AND buffered in step 2. On reconnect, inventory is reconciled via `desired_inventory` (step 5 of ConnAck processing), and buffered events are replayed separately (step 6). This means discovery/loss events may be published twice on reconnect (once as retained inventory, once as non-retained event replay). This is acceptable -- consumers must be idempotent.

#### 3.7.4 ConnAck Replay Order

On `reconnect_notify.notified()`, the publish_task executes in strict order:

```
(1) Online status        -- published by eventloop task in ConnAck handler, BEFORE notify
(2) Inventory reconcile  -- inventory.republish_all(&client)
(3) Buffer replay        -- replay max 10 items from outbound_buffer FIFO per select! iteration
(4) Live events          -- resume normal select! loop processing
```

Step (1) is executed by the eventloop pump task, not the publish_task. Steps (2)-(4) execute in the publish_task after it receives the notification.

#### 3.7.5 Buffer Replay Mechanics

On ConnAck notification, the publish_task replays **at most 10 items** from the buffer, then returns to the main `select!` loop. This ensures live events from `event_rx` are not starved during replay.

```
// Inside the connack_notify branch of the select! loop:
fn flush_pending():
    let batch_size = min(outbound_buffer.len(), 10)
    for _ in 0..batch_size:
        let (event_type, payload) = outbound_buffer.front()
        topic = topic(adapter_id, event_type)
        result = client.publish(topic, QoS1, retain=false, payload).await
        if result is Err:
            warn!("flush publish failed, keeping remaining buffer")
            return                     // <- stop replay, keep remaining
        outbound_buffer.pop_front()    // <- remove only after successful enqueue
    // After batch: return to select! loop
    // If outbound_buffer is still non-empty, the next select! iteration
    // will process more (either via another connack_notify or via the
    // guard condition !outbound_buffer.is_empty())
```

The `select!` loop naturally interleaves buffer replay with live event processing:

```
loop {
    tokio::select! {
        event = event_rx.recv() => { ... }
        _ = connack_notify.notified(), if !desired_inventory.is_empty()
                                         || !outbound_buffer.is_empty() => {
            replay_inventory();   // full inventory replay (small, bounded by device count)
            flush_pending();      // max 10 items from outbound_buffer
        }
    }
}
```

If the buffer still has items after a batch of 10, the `select!` guard `!outbound_buffer.is_empty()` ensures the connack branch is re-entered on the next iteration, alternating with any live events on `event_rx`. This avoids the `yield_now()` problem where yielding within a `select!` arm does not re-enter the `select!` macro.

Key behaviors:

- **FIFO order preserved.** Events are replayed in the exact order they were buffered.
- **Batch of 10, then return to select!.** This is a cooperative interleave with live events from `event_rx`, handled naturally by the `select!` macro.
- **Break on failure.** If `client.publish().await` returns `Err`, replay stops immediately. Remaining events stay in the buffer. They will be retried on the **next** `ConnAck` (not immediately).
- **No partial retry.** There is no retry loop within a single replay pass. A publish failure during replay means the connection is likely broken; the eventloop will detect this and transition to Disconnected.

### 3.8 Retained Inventory Semantics

#### 3.8.1 Data Model — Single `desired_inventory`

There is ONE inventory model. No separate `pending_retained_ops`. All inventory state lives in a single HashMap:

```rust
/// device_key (String) -> Some(payload) for active, None for tombstone
desired_inventory: HashMap<String, Option<Vec<u8>>>
```

This HashMap is the **sole source of truth** for device inventory. It is exclusively owned by publish_task (no sharing, no Mutex). The broker's retained message store is treated as a cache that is unconditionally overwritten on every reconnect.

On ConnAck, replay **everything** in `desired_inventory`. This is simple and correct.

| `desired_inventory` value | Meaning | MQTT action on publish/reconcile |
|---------------------------|---------|----------------------------------|
| `Some(payload)` | Device is active | Publish `payload` to `inventory/{device_key}`, QoS 1, **retained** |
| `None` | Device was lost (tombstone) | Publish **empty payload** (`Vec::new()`) to `inventory/{device_key}`, QoS 1, **retained** |
| Key absent | Device never seen or process restarted | No action |

#### 3.8.2 Event Tracking

**`DeviceDiscovered`:**

1. Encode event via `encode_event(adapter_id, event)` to get payload bytes.
2. `desired_inventory.insert(device_key_str, Some(payload))`.
3. If connected: publish `payload` to `iotkit/v1/{adapter_id}/inventory/{device_key}`, QoS 1, retained.

**`DeviceLost`:**

1. `desired_inventory.insert(device_key_str, None)` -- overwrites any previous `Some(payload)`.
2. If connected: publish **empty bytes** to `iotkit/v1/{adapter_id}/inventory/{device_key}`, QoS 1, retained. This clears the broker's retained message for that topic.

**All other events:** No inventory tracking. `track_event` returns `false`.

#### 3.8.3 Reconnect Reconciliation

On every `ConnAck`, the publish_task iterates **all entries** in `desired_inventory`:

```
for (device_key_str, maybe_payload) in &desired_inventory:
    topic = inventory_topic(adapter_id, device_key)
    payload = maybe_payload.clone().unwrap_or(Vec::new())  // Some -> data, None -> empty
    client.publish(topic, QoS1, retained=true, payload).await
```

This is an **unconditional full overwrite** of the broker's retained inventory state. There is no diffing, no checking what the broker currently holds. Every active device gets its payload re-published. Every tombstone gets an empty retained publish.

**Why unconditional overwrite:** With `clean_session=true`, the runner cannot know which QoS 1 publishes were actually delivered before the previous disconnection. A retained message may have been enqueued to rumqttc but never transmitted. The only safe strategy is to republish everything.

#### 3.8.4 Offline State Transitions

**Scenario: Device lost, then rediscovered, while disconnected.**

```
t=0: Connected. desired_inventory = {"sensor-a": Some(p1)}
t=1: Disconnected.
t=2: DeviceLost{sensor-a}   -> desired_inventory = {"sensor-a": None}
t=3: DeviceDiscovered{sensor-a, p2} -> desired_inventory = {"sensor-a": Some(p2)}
t=4: ConnAck -> republish_all publishes Some(p2) as retained.
```

The intermediate tombstone (`None` at t=2) is overwritten by the rediscovery (`Some(p2)` at t=3). On reconnect, only the **latest state** is published. The broker never sees the intermediate loss.

**Scenario: Device discovered while disconnected, then lost while still disconnected.**

```
t=0: Connected. desired_inventory = {}
t=1: Disconnected.
t=2: DeviceDiscovered{sensor-b, p1} -> desired_inventory = {"sensor-b": Some(p1)}
t=3: DeviceLost{sensor-b}           -> desired_inventory = {"sensor-b": None}
t=4: ConnAck -> republish_all publishes None (empty retained) for sensor-b.
```

The broker gets an empty retained message, which effectively clears any stale retained data (there should be none for a newly-discovered device, but this is defensive).

#### 3.8.5 Tombstone Lifetime

Tombstones (`None` entries) persist in `desired_inventory` for the **entire process lifetime**. They are:

- **Re-sent on every `ConnAck`.** Each reconnect publishes an empty retained message to clear the broker.
- **Never removed** from the HashMap during normal operation.
- **Cleared on process restart.** When the adapter process starts fresh, `desired_inventory` is empty. The adapter will re-discover devices, populating `desired_inventory` with fresh `Some(payload)` entries. Devices that no longer exist simply won't appear.

**Why keep tombstones forever:** A tombstone publish (`empty retained`) may have been enqueued to rumqttc but never delivered (the connection dropped before TCP write). Without per-message delivery confirmation, the only safe strategy is to re-send tombstones on every reconnect.

#### 3.8.6 Graceful Shutdown Inventory Behavior

On graceful shutdown (adapter closes `event_rx`, publish_task exits):

1. **Inventory is NOT cleared.** No tombstone publishes for active devices. Retained inventory messages remain on the broker.
2. **Only status changes:** Offline status is published (`encode_status(adapter_id, false, now_ms())`).
3. Devices may still be visible to consumers via retained inventory topics.

**Rationale:** In a multi-adapter deployment, other adapters may be publishing to the same broker. Clearing inventory on shutdown would create a false "all devices gone" signal. The offline status is sufficient for consumers to know this adapter is no longer active.

#### 3.8.7 Crash (Ungraceful Shutdown)

On crash or SIGKILL:

1. **LWT fires:** Broker publishes offline status with `ts=0` (timestamp unknown at LWT registration time). The LWT payload is `encode_status(adapter_id, false, 0)`.
2. **Inventory stays stale:** Retained inventory messages remain on the broker with the last-known payloads. No tombstones are published.
3. **On restart:** The new process starts with empty `desired_inventory`. As the adapter re-discovers devices, `republish_all` on the first `ConnAck` publishes only the currently-active devices. Devices that no longer exist will have stale retained messages on the broker **until the broker retains them indefinitely or another mechanism clears them**.

**Known limitation:** Stale retained inventory for devices that existed before the crash but do not exist after restart will persist on the broker indefinitely. This is acceptable for v1. A future version may implement a "full inventory sync" protocol where the runner publishes a manifest and a separate garbage collector clears orphaned retained messages.

### 3.9 Delivery Semantics

#### 3.9.1 rumqttc Delivery Model

rumqttc provides a two-component architecture:

- **`AsyncClient`**: Exposes `publish().await` which enqueues messages into an internal bounded async channel (capacity set at construction, currently 100).
- **`EventLoop`**: Must be polled continuously via `eventloop.poll().await`. It reads from the internal channel, serializes MQTT packets, writes to the TCP socket, and handles QoS 1 PUBACK tracking internally.

The critical constraint:

```
AsyncClient::publish().await Ok(())
    = message enqueued to internal channel
    != message written to TCP socket
    != message received by broker
    != PUBACK received
```

rumqttc handles QoS 1 PUBACK tracking internally (retransmission on timeout, packet ID management), but provides **no per-message delivery confirmation callback or future**. There is no way to know, from the `AsyncClient` API, whether a specific publish was acknowledged by the broker.

#### 3.9.2 Implications for Retained Operations

Because enqueue success does not guarantee delivery:

1. **`desired_inventory` entries MUST NOT be retired on enqueue success.** A `DeviceDiscovered` publish that returns `Ok(())` may never reach the broker if the connection drops before the eventloop transmits it.

2. **`desired_inventory` is replayed on EVERY `ConnAck`.** The full `republish_all` runs unconditionally, regardless of how many previous `ConnAck` cycles have occurred. This is the only way to guarantee eventual consistency given the lack of per-message delivery confirmation.

3. **Tombstones persist for the process lifetime.** A tombstone that was "successfully" published (enqueue returned `Ok`) may not have been delivered. It must be re-sent on every reconnect.

#### 3.9.3 Implications for Transient Events

Transient events (telemetry, errors) use fire-and-forget semantics:

- If `client.publish().await` returns `Ok`, the event is considered "best-effort sent". No retry.
- If `client.publish().await` returns `Err`, the event is logged and dropped (when connected) or the replay loop breaks (during buffer flush).
- There is **no application-level acknowledgment** for transient events. Data loss is possible and accepted.

#### 3.9.4 Graceful Offline Publish -- Eventloop Grace Period

On event_rx closure, the runner (not the binary) publishes offline status and gracefully stops:

```rust
// Runner's cleanup after publish_task exits:
// Publish offline status
client.publish(status_topic, QoS1, retained=true, offline_payload).await?;
client.disconnect().await;

// Grace period: let eventloop actually transmit the above
match timeout(Duration::from_secs(2), eventloop_join).await {
    Ok(_) => {},  // eventloop exited cleanly
    Err(_) => eventloop_join.abort(),  // timed out, abort
}
```

The 2-second grace period exists because:

1. `client.publish().await` only enqueues the offline status.
2. `client.disconnect().await` only enqueues the DISCONNECT packet.
3. The eventloop must still `poll()` to actually write these to the TCP socket.
4. Without the grace period, `eventloop_handle.abort()` would kill the eventloop before transmission.

| Scenario | Outcome |
|----------|---------|
| Local broker, low latency | 2s is sufficient. Offline status + DISCONNECT transmitted. |
| Remote broker, high latency | 2s may not be sufficient. Offline status may not reach broker. LWT will fire after keepalive timeout. |
| Broker unreachable at shutdown | 2s wasted. LWT fires after keepalive timeout. Offline status never delivered. |

The 2-second value is a pragmatic choice for v1, optimized for the primary deployment scenario (local Mosquitto on the same Raspberry Pi). There is no harm in the LWT firing as a fallback -- it publishes the same offline status, just with `ts=0` instead of the actual shutdown timestamp.

### 3.10 Failure Mode Tables

#### 3.10.1 Connection Lifecycle

| Trigger | Current state | Action | Next state | Observable effect |
|---------|--------------|--------|------------|-------------------|
| `poll()` returns `ConnAck` | Disconnected | store(true), reset attempt, publish online, notify | Connected | Online status published; inventory reconciled; buffer replayed |
| `poll()` returns `ConnAck` | Connected | store(true), reset attempt, publish online, notify | Connected | Duplicate ConnAck (unusual but safe); full reconcile re-runs |
| `poll()` returns `Err(ConnectionError)` | Connected | store(false), increment attempt, sleep(backoff) | Disconnected | `warn!` log; publish_task starts buffering |
| `poll()` returns `Err(Timeout)` | Connected | store(false), increment attempt, sleep(backoff) | Disconnected | Same as ConnectionError |
| `poll()` returns `Err` | Disconnected | increment attempt, sleep(backoff) | Disconnected | `warn!` log; backoff grows |
| `poll()` returns `Ok(PubAck)` | Connected | no-op | Connected | rumqttc retires internal pending |
| `poll()` returns `Ok(PingResp)` | Connected | no-op | Connected | Keepalive confirmed |
| Broker sends DISCONNECT | Connected | Next `poll()` returns Err | Disconnected | Handled via Err path |
| TCP RST from broker | Connected | Next `poll()` returns Err | Disconnected | Handled via Err path |
| DNS resolution fails | Disconnected | `poll()` returns Err | Disconnected | Backoff continues |
| `event_rx` closed (adapter stopped by binary) | Any | publish_task exits; runner drains buffer, publishes offline status; 2s grace; eventloop aborted; runner returns Ok | Terminated | Runner exits cleanly; binary decides exit code |

#### 3.10.2 Publish and Buffer

| Trigger | Current state | Action | Next state | Observable effect |
|---------|--------------|--------|------------|-------------------|
| `AdapterEvent` received | Connected | `track_event` + `publish_event` (inventory) + `publish_event` (non-retained) | Connected | Event published to broker (enqueued) |
| `AdapterEvent` received | Disconnected | `track_event` + `buffer_event` | Disconnected | Event tracked locally; buffered if transient |
| Buffer overflow (len >= 1000) | Disconnected | `pop_front` oldest + `push_back` new | Disconnected | `warn!` log; oldest event lost |
| `reconnect_notify` received | publish_task | `republish_all` + drain buffer | Connected (processing) | Inventory reconciled; buffer draining |
| `client.publish` fails during replay | Buffer draining | `break` from drain loop | Disconnected (pending re-notify) | Remaining buffer preserved; `warn!` log |
| `client.publish` fails for live event | Connected | Log `warn!`, drop the single event | Connected | Event lost; non-retained events are best-effort |
| `event_rx` closed | Any | Exit publish_task loop | Terminated | Publish_task returns |

#### 3.10.3 Inventory

| Trigger | State | Action | Result | Observable effect |
|---------|-------|--------|--------|-------------------|
| `DeviceDiscovered` while connected | desired_inventory[k] = any | Insert `Some(payload)` + retained publish | Broker has current inventory | Device visible to consumers |
| `DeviceDiscovered` while disconnected | desired_inventory[k] = any | Insert `Some(payload)` only | Local tracking updated | No broker publish; reconciled on ConnAck |
| `DeviceLost` while connected | desired_inventory[k] = Some | Insert `None` + empty retained publish | Broker inventory cleared | Device no longer visible |
| `DeviceLost` while disconnected | desired_inventory[k] = Some | Insert `None` only | Local tracking updated | Tombstone sent on ConnAck |
| `DeviceLost` for unknown device | desired_inventory[k] absent | Insert `None` | Tombstone created | Defensive; empty retained on ConnAck clears any stale data |
| Reconnect (ConnAck) | N active, M tombstones | Publish all N as retained + all M as empty retained | Broker state = local state | Full reconcile; `info!` log with counts |
| `republish_all` publish fails for one device | Iterating desired_inventory | `warn!` log, continue to next device | Partial reconcile | Failed device retried on next ConnAck |
| Graceful shutdown | N active devices | Publish offline status only; inventory unchanged | Broker retains last inventory | Consumers see offline status + stale inventory |
| Crash / SIGKILL | N active devices | LWT publishes offline (ts=0); inventory unchanged | Broker retains last inventory | Consumers see offline (ts=0) + stale inventory |
| Process restart after crash | Empty desired_inventory | Re-discover devices; first ConnAck reconciles | Broker gets fresh inventory | Previously-lost devices have stale retained (known limitation) |

#### 3.10.4 Delivery

| Trigger | Action | Outcome | Recovery |
|---------|--------|---------|----------|
| `client.publish().await` returns `Ok` | Message enqueued to rumqttc channel | May or may not reach broker | For retained ops: replayed on next ConnAck. For transient: no retry. |
| `client.publish().await` returns `Err(ClientError::Request)` | rumqttc internal channel full (100 items) | Message not enqueued | For live events: logged + dropped. For replay: break + retry on next ConnAck. |
| `client.publish().await` returns `Err(ClientError::TrySend)` | rumqttc internal channel closed | EventLoop has been dropped/aborted | Fatal -- process is shutting down. |
| Connection drops after enqueue, before TCP write | eventloop detects on next poll | QoS 1 messages in rumqttc's internal pending queue are lost (clean_session=true, no broker-side session) | Retained ops: replayed on next ConnAck from `desired_inventory`. Transient: lost. |
| PUBACK not received within rumqttc timeout | rumqttc retransmits internally | Eventually succeeds or connection declared broken | Transparent to runner; handled by rumqttc. |
| Graceful shutdown: eventloop aborted before 2s flush | Offline status enqueued but not transmitted | LWT fires after keepalive expiry (30s default) | LWT provides eventual offline status (ts=0). |
| Graceful shutdown: eventloop flushes within 2s | Offline status transmitted + DISCONNECT sent | Broker immediately knows adapter is offline with accurate timestamp | Clean shutdown. |

### 3.11 Complete State Diagram

```dot
digraph mqtt_protocol {
    rankdir=TB;
    compound=true;
    node [shape=box, style=rounded, fontsize=10];
    edge [fontsize=9];

    subgraph cluster_eventloop {
        label="Eventloop Pump Task";
        style=dashed;
        EL_Poll [label="poll()"];
        EL_ConnAck [label="ConnAck received\n1. store(true)\n2. reset attempt\n3. publish online status\n4. notify publish_task"];
        EL_Err [label="Error received\n1. store(false)\n2. increment attempt\n3. sleep(backoff)"];
        EL_Ok [label="Other Ok\n(no-op)"];

        EL_Poll -> EL_ConnAck [label="ConnAck"];
        EL_Poll -> EL_Err [label="Err"];
        EL_Poll -> EL_Ok [label="PubAck/PingResp/..."];
        EL_ConnAck -> EL_Poll;
        EL_Err -> EL_Poll;
        EL_Ok -> EL_Poll;
    }

    subgraph cluster_publish {
        label="Publish Task";
        style=dashed;
        PT_Select [label="select! {\n  event_rx.recv()\n  reconnect_notify\n}"];
        PT_Event_Connected [label="Connected path:\n1. track_event\n2. publish inventory\n3. publish event"];
        PT_Event_Disconnected [label="Disconnected path:\n1. track_event\n2. buffer_event"];
        PT_Reconnect [label="Reconnect path:\n1. republish_all inventory\n2. drain buffer (10+yield)\n3. resume"];
        PT_Exit [label="event_rx closed\n-> exit"];

        PT_Select -> PT_Event_Connected [label="event + connected=true"];
        PT_Select -> PT_Event_Disconnected [label="event + connected=false"];
        PT_Select -> PT_Reconnect [label="notified()"];
        PT_Select -> PT_Exit [label="recv() = None"];
        PT_Event_Connected -> PT_Select;
        PT_Event_Disconnected -> PT_Select;
        PT_Reconnect -> PT_Select;
    }

    subgraph cluster_shutdown {
        label="Runner Drain Sequence\n(triggered by event_rx closure)";
        style=dashed;
        S1 [label="event_rx closed\n(adapter stopped by binary)"];
        S2 [label="Publish task drains buffer\n(2s timeout)"];
        S3 [label="Publish offline status\n(enqueue)"];
        S4 [label="client.disconnect()\n(enqueue)"];
        S5 [label="Eventloop grace (2s)"];
        S6 [label="Abort eventloop"];
        S7 [label="Runner returns Ok(())"];

        S1 -> S2 -> S3 -> S4 -> S5 -> S6 -> S7;
    }
}
```

### 3.12 Constants Summary

| Constant | Value | Location | Configurable? |
|----------|-------|----------|---------------|
| `PENDING_BUFFER_CAP` | 1000 | `publish_loop.rs` | No (compile-time) |
| rumqttc channel capacity | 100 | `mqtt_client.rs` (`AsyncClient::new(opts, 100)`) | No (compile-time) |
| Backoff base | 1000 ms | `lib.rs` (`backoff_with_jitter`) | No (compile-time) |
| Backoff max | 30000 ms | `lib.rs` (`backoff_with_jitter`) | No (compile-time) |
| Backoff jitter | +/- 30% | `lib.rs` (`backoff_with_jitter`) | No (compile-time) |
| Graceful shutdown grace period | 2000 ms | `lib.rs` (`run`) | No (compile-time) |
| Keepalive | 30 s (default) | `mqtt_client.rs` | Yes (`MqttConfig.keepalive_secs`) |
| Replay batch size | 10 items per select! iteration | `publish_loop.rs` | No (compile-time) |
| `clean_session` | `true` | rumqttc default | No |
| QoS | `AtLeastOnce` (1) | All publishes | No |
| LWT timestamp | `0` (unknown) | `mqtt_client.rs` | No |

---

## 4. Configuration, Identity, and Deployment (`iotkit-rpi-local`)

### 4.1 TOML Config Schema

The configuration file uses TOML. Every field is validated at startup before any I/O is attempted.

```toml
# Top-level fields

adapter_id = "rpi-local:default"
# Type: String
# Required: YES
# Constraints: non-empty after trim. Colons and slashes are valid (encoded in MQTT client_id).

[mqtt]
broker_url = "mqtt://localhost:1883"
# Type: String
# Required: YES
# Constraints: must begin with "mqtt://" or "mqtts://". Full URL including host.
#   Port is optional; defaults applied at parse time (see Section 4.6).

client_id = "custom-id"
# Type: String
# Required: NO
# Default: "iotkit-<percent_encoded(adapter_id)>"
# Constraints: non-empty if present. Validated after resolution (never written as empty string).

keepalive_secs = 30
# Type: u16
# Required: NO
# Default: 30
# Constraints: must be >= 1. Zero is rejected (see Section 4.2).

ca_path = "/path/to/ca.pem"
# Type: String (filesystem path)
# Required: YES if broker_url uses mqtts://; MUST NOT be present if broker_url uses mqtt://.
# Constraints: non-empty. File existence is checked at startup.

client_cert_path = "/path/to/cert.pem"
# Type: String (filesystem path)
# Required: NO
# Constraints: non-empty. MUST be paired with client_key_path (both present or both absent).
#   MUST NOT be present if broker_url uses mqtt://.

client_key_path = "/path/to/key.pem"
# Type: String (filesystem path)
# Required: NO
# Constraints: non-empty. MUST be paired with client_cert_path (both present or both absent).
#   MUST NOT be present if broker_url uses mqtt://.

[adapter]
bus_path = "/dev/i2c-1"
# Type: String (filesystem path)
# Required: YES
# Constraints: non-empty after trim. File existence is NOT checked at parse time
#   (device node may appear after udev settles); checked at adapter start.

poll_interval_ms = 1000
# Type: u64
# Required: YES
# Constraints: must be >= 1. Zero is rejected (see Section 4.2).

[[adapter.targets]]
# One or more entries required. Empty targets array is rejected (see Section 4.2).

driver = "mcp9600"
# Type: String
# Required: YES per target
# Constraints: non-empty. Must be a known driver name.
#   Known drivers: "mcp9600", "opt3001"
#   Unknown driver name -> config error (not silent skip).

address = 0x60
# Type: u8 (parsed as integer, hex literals supported)
# Required: YES per target
# Constraints: valid I2C address range 0x08-0x77 (standard 7-bit).

thermocouple_type = "K"
# Type: String
# Required: YES when driver = "mcp9600"
# Constraints: must be one of: "K", "J", "T", "N", "S", "E", "B", "R" (case-sensitive).
#   Any other value -> config error (not silent K fallback).
# Not applicable to other drivers; presence on non-mcp9600 target -> config error.

[[adapter.targets]]
driver = "opt3001"
address = 0x44
# opt3001 has no driver-specific required fields beyond driver and address.
```

#### Parsed Rust Types

```rust
#[derive(Deserialize)]
pub struct Config {
    pub adapter_id: String,
    pub mqtt: MqttConfig,
    pub adapter: AdapterConfig,
}

#[derive(Deserialize)]
pub struct MqttConfig {
    pub broker_url: String,
    pub client_id: Option<String>,
    pub keepalive_secs: Option<u16>,
    pub ca_path: Option<String>,
    pub client_cert_path: Option<String>,
    pub client_key_path: Option<String>,
}

#[derive(Deserialize)]
pub struct AdapterConfig {
    pub bus_path: String,
    pub poll_interval_ms: u64,
    pub targets: Vec<TargetConfig>,
}

#[derive(Deserialize)]
pub struct TargetConfig {
    pub driver: String,
    pub address: u8,
    pub thermocouple_type: Option<String>,
}
```

All validation beyond deserialization is performed in a `Config::validate(&self) -> Result<ValidatedConfig, ConfigError>` method called immediately after `toml::from_str`.

### 4.2 Config Cross-Field Validation Rules

Validation is performed top-to-bottom. All errors are collected (not fail-fast) so the user sees every problem in one run. The process exits with code 78 (EX_CONFIG) after printing all errors to stderr.

Error messages use the format: `config error: <field-path>: <reason>`

#### 4.2.1 adapter_id

| Condition | Error message |
|---|---|
| `adapter_id` is empty or whitespace-only | `config error: adapter_id: must not be empty` |

#### 4.2.2 mqtt.broker_url

| Condition | Error message |
|---|---|
| `broker_url` is empty | `config error: mqtt.broker_url: must not be empty` |
| `broker_url` does not begin with `mqtt://` or `mqtts://` | `config error: mqtt.broker_url: scheme must be "mqtt" or "mqtts", got "<actual-scheme>"` |
| `broker_url` is not a valid URL after scheme substitution (see Section 4.6) | `config error: mqtt.broker_url: invalid URL: <url-parse-error>` |
| `broker_url` has no host component | `config error: mqtt.broker_url: host must not be empty` |

#### 4.2.3 TLS Field Rules

| Condition | Error message |
|---|---|
| scheme is `mqtt://` AND `ca_path` is present | `config error: mqtt.ca_path: must not be set when broker_url uses mqtt:// (non-TLS)` |
| scheme is `mqtt://` AND `client_cert_path` is present | `config error: mqtt.client_cert_path: must not be set when broker_url uses mqtt:// (non-TLS)` |
| scheme is `mqtt://` AND `client_key_path` is present | `config error: mqtt.client_key_path: must not be set when broker_url uses mqtt:// (non-TLS)` |
| scheme is `mqtts://` AND `ca_path` is absent | `config error: mqtt.ca_path: required when broker_url uses mqtts://` |
| `client_cert_path` is present AND `client_key_path` is absent | `config error: mqtt.client_key_path: must be set when mqtt.client_cert_path is set` |
| `client_key_path` is present AND `client_cert_path` is absent | `config error: mqtt.client_cert_path: must be set when mqtt.client_key_path is set` |

File existence checks (only when scheme is mqtts://):

| Condition | Error message |
|---|---|
| `ca_path` file does not exist | `config error: mqtt.ca_path: file not found: "<path>"` |
| `client_cert_path` file does not exist | `config error: mqtt.client_cert_path: file not found: "<path>"` |
| `client_key_path` file does not exist | `config error: mqtt.client_key_path: file not found: "<path>"` |

#### 4.2.4 mqtt.keepalive_secs

| Condition | Error message |
|---|---|
| `keepalive_secs` is `Some(0)` | `config error: mqtt.keepalive_secs: must be >= 1, got 0` |

#### 4.2.5 mqtt.client_id

| Condition | Error message |
|---|---|
| `client_id` is `Some("")` (explicitly set to empty string) | `config error: mqtt.client_id: must not be empty if specified` |

#### 4.2.6 adapter.bus_path

| Condition | Error message |
|---|---|
| `bus_path` is empty or whitespace-only | `config error: adapter.bus_path: must not be empty` |

#### 4.2.7 adapter.poll_interval_ms

| Condition | Error message |
|---|---|
| `poll_interval_ms` is `0` | `config error: adapter.poll_interval_ms: must be >= 1, got 0` |

#### 4.2.8 adapter.targets

| Condition | Error message |
|---|---|
| `targets` array is empty | `config error: adapter.targets: must contain at least one target` |

Per-target validation (index is 0-based):

| Condition | Error message |
|---|---|
| `driver` is empty | `config error: adapter.targets[<i>].driver: must not be empty` |
| `driver` is not a known value | `config error: adapter.targets[<i>].driver: unknown driver "<value>"; known drivers: mcp9600, opt3001` |
| `address` is outside 0x08-0x77 | `config error: adapter.targets[<i>].address: I2C address 0x<hex> out of valid range 0x08-0x77` |
| `driver` is `"mcp9600"` AND `thermocouple_type` is absent | `config error: adapter.targets[<i>].thermocouple_type: required for driver "mcp9600"` |
| `driver` is `"mcp9600"` AND `thermocouple_type` is not in {K,J,T,N,S,E,B,R} | `config error: adapter.targets[<i>].thermocouple_type: unknown type "<value>"; valid values: K, J, T, N, S, E, B, R` |
| `driver` is NOT `"mcp9600"` AND `thermocouple_type` is present | `config error: adapter.targets[<i>].thermocouple_type: not applicable to driver "<driver>"` |

#### 4.2.9 Duplicate Target Address

| Condition | Error message |
|---|---|
| Two targets share the same `address` value | `config error: adapter.targets: duplicate I2C address 0x<hex> at indices <i> and <j>` |

### 4.3 Identity Derivation

#### 4.3.1 adapter_id

`adapter_id` comes exclusively from the TOML config file. It is never derived from hostname, process arguments, or environment variables. It is the single source of truth for this adapter instance's identity across restarts.

The validated `adapter_id` string is stored as-is in `ValidatedConfig` and passed to every component that needs it (MQTT topic builder, log fields, metrics labels).

#### 4.3.2 MQTT client_id

**Default derivation** (when `mqtt.client_id` is absent from config):

```
client_id = "iotkit-" + percent_encode(adapter_id)
```

**Percent-encoding rules:**

- Encoding applies the `NON_ALPHANUMERIC` set from the `percent-encoding` crate (i.e. every byte that is not `A-Z`, `a-z`, `0-9` is encoded).
- Specific examples: `:` -> `%3A`, `/` -> `%2F`, `-` -> `%2D`, `_` -> `%5F`, `.` -> `%2E`, space -> `%20`.
- The encoding is deterministic and reversible.
- The result is ASCII-safe and valid as an MQTT client identifier per MQTT 3.1.1 section 3.1.3.

**Example:**

| adapter_id | client_id |
|---|---|
| `rpi-local:default` | `iotkit-rpi%2Dlocal%3Adefault` |
| `rpi-local/zone-a` | `iotkit-rpi%2Dlocal%2Fzone%2Da` |

**Override:** If `mqtt.client_id` is explicitly set in the TOML (non-empty string), that value is used verbatim. No encoding is applied to the override value; the operator is responsible for its validity.

**Stability guarantee:** The derived `client_id` is identical across every restart of the same binary with the same config file. Random suffixes are never appended. Using random suffixes would create multiple simultaneous MQTT sessions (split-brain) during rapid restart cycles, causing persistent session data to accumulate on the broker.

### 4.4 Config Path Resolution

The binary accepts an optional `--config <path>` CLI argument parsed as `Option<PathBuf>`.

#### 4.4.1 Explicit Path (`--config` provided)

Use the provided path exactly. Do not canonicalize or search for alternatives.

If the file does not exist or is not readable:
```
error: config file not found: "<path>"
```
Exit with code 78 (EX_CONFIG).

#### 4.4.2 Default Search (`--config` omitted)

Try the following paths in order:

1. `./iotkit-rpi-local.toml` (relative to the process working directory)
2. `/etc/iotkit/iotkit-rpi-local.toml`

For each path: attempt to open the file. If it opens successfully, use it and stop searching. If the open fails with `NotFound`, continue to the next candidate. Any other I/O error (permission denied, etc.) is treated as fatal:
```
error: failed to read config file "<path>": <os-error>
```
Exit with code 78.

If neither candidate exists:
```
error: no config file found; tried:
  ./iotkit-rpi-local.toml
  /etc/iotkit/iotkit-rpi-local.toml
hint: use --config <path> to specify a config file explicitly
```
Exit with code 78.

The resolved path is logged at INFO level on startup:
```
[INFO] loading config from /etc/iotkit/iotkit-rpi-local.toml
```

### 4.5 Binary Entrypoint

```rust
// iotkit-rpi-local/src/main.rs (pseudo)
fn main() -> ExitCode {
    // 1. Init tracing
    // 2. Parse CLI args (--config path)
    // 3. Load + validate TOML config
    // 4. Build RpiLocalConfig from adapter section
    // 5. Validate adapter config (rpi_local_adapter::validate)
    // 6. Create tokio runtime
    // 7. rt.block_on(async {
    //      a. Install signal handlers (SIGINT, SIGTERM)
    //      b. Start adapter (rpi_local_adapter::start) -> ShutdownHandle + event_rx
    //      c. Spawn runner: tokio::spawn(adapter_runner::run(adapter_id, mqtt_config, event_rx))
    //      d. select! {
    //           signal => {
    //             shutdown_initiated = true;
    //             timeout(5s, adapter_handle.shutdown()).await;
    //             runner_join.await -> Ok => exit 0, Err => exit 1
    //           }
    //           runner_result = runner_join => {
    //             if !shutdown_initiated { log "adapter died"; exit 1 }
    //             match runner_result { Ok => exit 0, Err => exit 1 }
    //           }
    //         }
    //    })
}
// Note: rpi_local_adapter::start() requires a live tokio runtime,
// so runtime creation MUST precede adapter start.
// Note: The binary owns signal handlers and adapter ShutdownHandle.
// The runner NEVER handles signals.
```

CLI:

```
iotkit-rpi-local --config /path/to/config.toml
iotkit-rpi-local --help
iotkit-rpi-local --version
```

Dependencies: `iotkit-adapter-runner`, `rpi-local-adapter`, `toml`, `clap`, `tracing`, `tracing-subscriber`, `tokio`.

### 4.6 URL Parsing Rules

#### 4.6.1 Parser Choice

Use the `url` crate (version 2.x) for all URL parsing. String-prefix matching (`starts_with`) is used only for scheme extraction before handing off to the URL parser. No URL component is extracted by manual string splitting.

#### 4.6.2 Scheme Normalization

The `url` crate does not natively understand `mqtt://` or `mqtts://` schemes. Before parsing, substitute the scheme for a known HTTP-family scheme that shares the same syntax:

| Config scheme | Substitute for parsing | Default port |
|---|---|---|
| `mqtt://` | `http://` | 1883 |
| `mqtts://` | `https://` | 8883 |

Procedure:

```rust
fn normalize_broker_url(raw: &str) -> Result<(url::Url, u16), ConfigError> {
    let (substituted, default_port) = if let Some(rest) = raw.strip_prefix("mqtts://") {
        (format!("https://{rest}"), 8883u16)
    } else if let Some(rest) = raw.strip_prefix("mqtt://") {
        (format!("http://{rest}"), 1883u16)
    } else {
        return Err(ConfigError::InvalidScheme(raw.to_string()));
    };
    let parsed = url::Url::parse(&substituted)
        .map_err(|e| ConfigError::InvalidUrl(raw.to_string(), e.to_string()))?;
    Ok((parsed, default_port))
}
```

#### 4.6.3 Port Resolution

After parsing, resolve the effective port:

```rust
let port = parsed.port().unwrap_or(default_port);
```

#### 4.6.4 Host Extraction

Extract the host string from the parsed URL:

```rust
let host_str = parsed.host_str()
    .ok_or_else(|| ConfigError::EmptyHost(raw.to_string()))?;
```

**IPv6 bracket stripping:** The `url` crate represents IPv6 addresses with surrounding brackets in the serialized form but `host_str()` returns the bare address without brackets. Pass `host_str()` directly to `rumqttc`. Do NOT use `parsed.host()` (which returns `Host::Ipv6(addr)`) and then re-serialize it, as that may reintroduce brackets.

Verification: if `host_str` starts with `[` or ends with `]`, strip them. This is a defensive fallback only; under normal url crate behavior it should not be needed.

#### 4.6.5 Components Passed to rumqttc

From the parsed URL, extract and pass to `rumqttc::MqttOptions`:

- `host`: `host_str` (brackets stripped as above)
- `port`: resolved port (u16)
- `client_id`: from Section 4.3.2
- `keep_alive`: `Duration::from_secs(keepalive_secs as u64)`

Username and password, if present in the URL, are extracted from `parsed.username()` and `parsed.password()`. The password is treated as sensitive (see Section 4.8).

### 4.7 Deploy Layout

```
/opt/iotkit/
├── bin/
│   └── iotkit-rpi-local          # compiled release binary, mode 0755
├── etc/
│   ├── iotkit-rpi-local.toml     # primary config, mode 0640, owner root:iotkit
│   └── certs/
│       ├── ca.pem                # CA certificate, mode 0640, owner root:iotkit
│       ├── client.pem            # client certificate, mode 0640, owner root:iotkit
│       └── client.key            # client private key, mode 0640, owner root:iotkit
└── data/                         # reserved for future persistent state, mode 0750
```

System user and group:

```
User:  iotkit   (no login shell, no home directory)
Group: iotkit
```

The `iotkit` user must be a member of the `i2c` group (Raspberry Pi OS default: `i2c`) to access `/dev/i2c-*` without root.

```bash
useradd --system --no-create-home --shell /usr/sbin/nologin iotkit
usermod -aG i2c iotkit
```

The binary at `/opt/iotkit/bin/iotkit-rpi-local` is owned `root:root`, mode `0755`. It does not run as root.

For multiple adapter instances (see Section 4.10), each instance uses a separate config file and a separate systemd unit, but shares the same binary and `iotkit` user.

### 4.8 Sensitive Value Redaction

The following rules apply to all log output (tracing spans, events, and structured fields) and to any diagnostic output printed to stderr.

#### 4.8.1 client_key_path Content

The file contents of `client_key_path` (the PEM-encoded private key) MUST NEVER be logged, printed, or included in any diagnostic struct's `Debug` or `Display` implementation.

The file path string itself (e.g. `/opt/iotkit/etc/certs/client.key`) may be logged at DEBUG level when loading the TLS configuration. Example:

```
[DEBUG] loading client key from /opt/iotkit/etc/certs/client.key
```

The bytes read from the file are used exclusively by the TLS stack and then dropped. They are never stored in a struct that derives `Debug`.

#### 4.8.2 broker_url Password

If the `broker_url` contains a password component (e.g. `mqtt://user:secret@host:1883`), the password MUST be redacted in all log output. Replace the password with `[REDACTED]`.

```
[INFO] connecting to broker mqtt://user:[REDACTED]@host:1883
```

Implementation: extract password with `parsed.password()` before logging. Build the display URL by calling `parsed.clone()` followed by `set_password(Some("[REDACTED]"))`.

#### 4.8.3 ca_path and client_cert_path

The file contents of `ca_path` and `client_cert_path` are never logged. The file paths may appear in log output at DEBUG level. Example:

```
[DEBUG] loading CA certificate from /opt/iotkit/etc/certs/ca.pem
[DEBUG] loading client certificate from /opt/iotkit/etc/certs/client.pem
```

#### 4.8.4 Debug Derives

The `MqttConfig` struct MUST NOT derive `Debug` directly. Instead, implement `Debug` manually, replacing `client_key_path` content (the path string is acceptable, the file bytes are not) and redacting any password in `broker_url`. Alternatively, use a wrapper type `Redacted<T>` that implements `Debug` as `"[REDACTED]"`.

#### 4.8.5 Summary

| Value | Log file path? | Log file content? | Log in Debug? |
|---|---|---|---|
| `ca_path` (path string) | YES (DEBUG) | NO | path only |
| `client_cert_path` (path string) | YES (DEBUG) | NO | path only |
| `client_key_path` (path string) | YES (DEBUG) | NO | path only |
| `client_key_path` (file bytes) | NO | NO | NO |
| `broker_url` with password | redacted form only | N/A | redacted |
| `broker_url` without password | YES (INFO) | N/A | YES |

### 4.9 systemd Unit

File path: `/etc/systemd/system/iotkit-rpi-local.service`

For a second adapter instance, copy this file to `iotkit-rpi-local@.service` and parametrize with `%i` (see Section 4.10).

```ini
[Unit]
Description=IoTKit Raspberry Pi Local Adapter
Documentation=https://github.com/iotkit/iotkit-next
After=network-online.target
Wants=network-online.target
# Ensure the I2C bus is available before starting.
# The i2c-dev module must be loaded; add it to /etc/modules if not auto-loaded.

[Service]
Type=simple
User=iotkit
Group=iotkit
SupplementaryGroups=i2c

ExecStart=/opt/iotkit/bin/iotkit-rpi-local --config /opt/iotkit/etc/iotkit-rpi-local.toml

Restart=on-failure
RestartSec=5s
# Limit restart attempts to avoid flooding the broker with reconnects.
StartLimitIntervalSec=60s
StartLimitBurst=5

# Working directory (affects ./iotkit-rpi-local.toml default search, though
# --config is explicit above).
WorkingDirectory=/opt/iotkit

# Environment
Environment=RUST_LOG=info

# --- Security Hardening ---

# Prevent privilege escalation.
NoNewPrivileges=true

# Expose only the necessary parts of the filesystem.
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/opt/iotkit/data

# I2C device access. Enumerate all I2C buses that may be used.
# Adjust if the bus is not /dev/i2c-1.
DeviceAllow=/dev/i2c-1 rw
DevicePolicy=closed

# Private /tmp and /var/tmp.
PrivateTmp=true

# Restrict system calls to a safe set for a network+I2C application.
SystemCallFilter=@system-service @network-io
SystemCallErrorNumber=EPERM

# Restrict address families.
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX

# Prevent loading kernel modules.
ProtectKernelModules=true
ProtectKernelTunables=true
ProtectKernelLogs=true

# Prevent writing to /proc or /sys.
ProtectControlGroups=true

# Limit capabilities to none (the process needs no Linux capabilities).
CapabilityBoundingSet=
AmbientCapabilities=

# Restrict realtime scheduling (not needed).
RestrictRealtime=true

# Prevent namespace creation.
RestrictNamespaces=true

# Lock down the personality syscall.
LockPersonality=true

# Memory execution protection.
MemoryDenyWriteExecute=true

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
systemctl daemon-reload
systemctl enable iotkit-rpi-local.service
systemctl start  iotkit-rpi-local.service
```

### 4.10 Process Model

#### 4.10.1 One Adapter per Process

Each `iotkit-rpi-local` process manages exactly one I2C bus (one `bus_path`) with its own set of targets, its own MQTT connection, and its own `adapter_id`. There is no intra-process multiplexing of adapters.

Rationale: I2C bus errors, MQTT reconnects, and target failures are scoped to one process. A crash or restart of one adapter does not affect others.

#### 4.10.2 Multiple Adapters

To run multiple adapter instances on the same host (e.g. `/dev/i2c-1` and `/dev/i2c-3`):

1. Create a separate config file for each instance, each with a distinct `adapter_id` and `bus_path`.
2. Create a separate systemd unit for each instance.

Recommended naming convention using systemd template units:

**Unit file:** `/etc/systemd/system/iotkit-rpi-local@.service`

Replace the `ExecStart` and `DeviceAllow` lines with:

```ini
ExecStart=/opt/iotkit/bin/iotkit-rpi-local --config /opt/iotkit/etc/iotkit-rpi-local-%i.toml
DeviceAllow=/dev/i2c-%i rw
```

The `%i` in `DeviceAllow` is expanded by systemd to the instance name (e.g. `i2c1` -> `/dev/i2c-i2c1`). For numeric instance names like `1`, `3`, this produces `/dev/i2c-1`, `/dev/i2c-3`. Choose instance names accordingly (e.g. `iotkit-rpi-local@1.service` for `/dev/i2c-1`).

**Config files:**

```
/opt/iotkit/etc/iotkit-rpi-local-i2c1.toml   # adapter_id = "rpi-local:i2c1"
/opt/iotkit/etc/iotkit-rpi-local-i2c3.toml   # adapter_id = "rpi-local:i2c3"
```

**Enable and start:**

```bash
systemctl enable iotkit-rpi-local@i2c1.service
systemctl enable iotkit-rpi-local@i2c3.service
systemctl start  iotkit-rpi-local@i2c1.service
systemctl start  iotkit-rpi-local@i2c3.service
```

Each instance has an independent systemd restart policy. A failure in `@i2c1` does not trigger a restart of `@i2c3`.

#### 4.10.3 Uniqueness Invariants

Across all running instances on a single host:

| Property | Must be unique | Enforced by |
|---|---|---|
| `adapter_id` | YES | Operator config discipline (not enforced at runtime) |
| MQTT `client_id` | YES | Follows from unique `adapter_id` via Section 4.3.2 |
| `bus_path` | YES | Operator config discipline |
| systemd unit name | YES | Follows from unique instance name |

No runtime locking mechanism prevents two processes from using the same `adapter_id` or `bus_path`. Correctness depends on correct deployment configuration.

---

## 5. Quantitative Targets

| Metric | Target | Rationale |
|--------|--------|-----------|
| MQTT publish latency (local broker) | < 5ms per message | rumqttc async publish, local loopback |
| Memory footprint (rpi-local binary) | < 20MB RSS | RPi Zero 2W 512MB, must stay under 4% |
| Startup to first publish | < 3s | adapter detect + init + first read + MQTT connect |
| Reconnect backoff range | 1s -> 30s | exponential with +/-30% jitter |
| Event throughput | 100 msg/sec sustainable | 10x headroom over 10 sensors x 1Hz |

---

## 6. Required Automated Tests

### 6.1 `core/mqtt-contract` Tests

- **Serde round-trip:** `encode_event` -> `decode_event` -> assert equality for all event types (SensorData, DeviceDiscovered, DeviceLost, AdapterError).
- **Topic builder:** Verify `topic()` produces correct topic strings for all `EventType` variants. Verify percent-encoding of `adapter_id` containing `:`, `/`, `+`, `#`, `%`.
- **Inventory topic builder:** Verify `inventory_topic()` encodes both `adapter_id` and `device_key`.
- **Segment encoding round-trip:** `decode_topic_segment(encode_topic_segment(s)) == s` for edge cases (empty string, all-special-characters, already-encoded-looking strings).
- **Unknown version handling:** `v: 99` envelope -> `DecodeError::UnknownVersion(99)`.
- **Negative timestamp:** `ts: -1` -> `DecodeError::InvalidTimestamp(-1)`.
- **Negative ingested_at:** Telemetry with `ingested_at: -1` -> `DecodeError::InvalidTimestamp(-1)`.
- **Label/value length mismatch:** `labels: ["a"]`, `values: [1.0, 2.0]` -> `DecodeError::InvalidPayload`.
- **Status encode/decode round-trip:** `encode_status` -> `decode_status` for online=true, online=false, ts=0 (LWT). Verify returned `(adapter_id, online, ts)` triple matches input.
- **Status via decode_event:** Passing a status payload to `decode_event` -> `DecodeError::InvalidPayload`.
- **DeviceConfig encoding:** `encode_event` with `DeviceConfig` -> `EncodeError::UnsupportedEvent`.
- **Unknown fields ignored:** Envelope with extra fields decodes successfully (forward compatibility).
- **ConnectionKind as_str/from_str symmetry:** `ConnectionKind::from_str(k.as_str()) == k` for all variants (including `Other`).

### 6.2 `iotkit-adapter-runner` Tests

- **Adapter task exit -> runner exits:** Adapter drops `event_tx` unexpectedly -> runner drains, publishes offline, returns Ok. (Binary decides exit code based on shutdown_initiated.)
- **Disconnect + DeviceDiscovered -> inventory tracking:** Device discovered while disconnected -> `desired_inventory` updated -> on ConnAck, retained publish occurs.
- **Disconnect + DeviceLost -> tombstone:** Device lost while disconnected -> tombstone recorded -> on ConnAck, empty retained publish.
- **ConnAck -> full inventory republish:** After ConnAck, all entries in `desired_inventory` are published as retained.
- **Buffer overflow:** Generate > 1000 events while disconnected -> oldest events dropped, newest retained, no panic.
- **Buffer replay FIFO order:** Buffer 5 events while disconnected -> reconnect -> verify they are replayed in original order.
- **Buffer replay batch fairness:** Buffer 30 events -> reconnect -> verify only 10 are replayed per select! iteration, with live events interleaved between batches.
- **Graceful shutdown sequence:** Binary receives signal -> adapter stopped (closes event_rx) -> runner drains buffer -> runner publishes offline status -> runner returns Ok -> binary exits 0.
- **Offline status timestamp:** Graceful offline status has `ts > 0`. LWT has `ts = 0`.
- **2nd signal -> immediate exit:** Second signal during shutdown -> binary calls `std::process::exit(1)`.
- **publish_task panic -> runner returns Err:** Simulate publish_task panicking -> runner returns Err(RunnerError::PublishTaskFailed).
- **eventloop_task unexpected exit -> runner returns Err:** Simulate eventloop_task returning unexpectedly -> runner returns Err(RunnerError::EventLoopDied).
- **Backoff calculation:** Verify exponential growth with jitter: attempt 0 -> ~1s, attempt 5 -> ~32s (capped at 30s), jitter within +/-30%.
- **Reconnect counter reset:** ConnAck resets attempt counter to 0.
- **Device lost then rediscovered while disconnected:** `desired_inventory` reflects latest state (rediscovered); intermediate tombstone not published.

### 6.3 `iotkit-rpi-local` Config Tests

- **Valid TOML parse:** Well-formed config -> `ValidatedConfig` with all fields populated.
- **Empty adapter_id:** -> `config error: adapter_id: must not be empty`.
- **Empty broker_url:** -> `config error: mqtt.broker_url: must not be empty`.
- **Invalid scheme:** `tcp://localhost` -> config error with scheme message.
- **Missing host:** `mqtt://` -> `config error: mqtt.broker_url: host must not be empty`.
- **mqtt:// with ca_path:** -> `config error: mqtt.ca_path: must not be set when broker_url uses mqtt:// (non-TLS)`.
- **mqtts:// without ca_path:** -> `config error: mqtt.ca_path: required when broker_url uses mqtts://`.
- **Half-configured TLS:** `client_cert_path` without `client_key_path` -> config error.
- **TLS settings on mqtt://:** Any TLS field on plain mqtt:// -> config error.
- **keepalive_secs = 0:** -> `config error: mqtt.keepalive_secs: must be >= 1, got 0`.
- **Empty client_id:** `client_id = ""` -> `config error: mqtt.client_id: must not be empty if specified`.
- **Empty targets:** -> `config error: adapter.targets: must contain at least one target`.
- **Unknown driver:** `driver = "unknown"` -> config error listing known drivers.
- **Address out of range:** `address = 0x07` or `address = 0x78` -> config error.
- **Missing thermocouple_type for mcp9600:** -> config error.
- **Invalid thermocouple_type for mcp9600:** `thermocouple_type = "X"` -> config error.
- **thermocouple_type on non-mcp9600:** `driver = "opt3001"` with `thermocouple_type = "K"` -> config error.
- **Duplicate I2C address:** Two targets with same address -> config error with both indices.
- **poll_interval_ms = 0:** -> `config error: adapter.poll_interval_ms: must be >= 1, got 0`.
- **Explicit --config bad path:** Non-existent path -> error exit.
- **Default config search:** Neither `./iotkit-rpi-local.toml` nor `/etc/iotkit/iotkit-rpi-local.toml` exists -> error with hint.
- **IPv6 broker URL:** `mqtt://[::1]:1883` -> host extracted without brackets.
- **Deterministic client_id:** `adapter_id = "rpi-local:default"` -> `client_id = "iotkit-rpi%2Dlocal%3Adefault"`.
- **Default port resolution:** `mqtt://localhost` -> port 1883. `mqtts://localhost` -> port 8883.
- **Collect-all-errors:** Config with multiple errors -> all errors reported in single output.

---

## 7. Out of Scope

The following items are explicitly excluded from Phase 1A:

- **Inbound MQTT commands** -- No subscribe-side logic. The runner is publish-only. Command handling (e.g. `DeviceConfig` writes, actuator control) is deferred to Phase 2 command bridge (#46).
- **Hot reload** -- Config changes require a process restart. There is no file watcher, no SIGHUP handler, and no runtime reconfiguration mechanism.
- **Health API endpoints** -- No HTTP health check endpoint. Liveness is observed via the retained `status` MQTT topic. A future version may expose a local socket for systemd `Type=notify` or an HTTP `/healthz` endpoint.
- **Persistence / local disk buffer** -- The `outbound_buffer` is in-memory only. On process restart, all buffered events are lost. Local disk persistence (e.g. SQLite WAL, append-only log) is deferred due to SD card write amplification concerns on Raspberry Pi.
- **MQTT v5** -- The runner uses MQTT v3.1.1 exclusively. MQTT v5 features (user properties, shared subscriptions, topic aliases, message expiry) are not used. rumqttc supports MQTT v5 but enabling it would require protocol-level testing against all target brokers.
- **Gateway MQTT subscriber** (#45) -- Phase 2.
- **DeviceKey bus identity change** (#33) -- Phase 2. Current `i2c:0x{addr}:{suffix}` format is preserved.
- **Transform layer** (#43, #44) -- Phase 1B.
- **BravePI standalone adapter** (#46) -- Phase 2.
- **Auto-detection** (#35) -- Phase 2.
