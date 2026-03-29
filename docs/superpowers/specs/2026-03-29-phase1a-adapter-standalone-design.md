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

Published to `iotkit/v1/{adapter_id}/discovery` (non-retained). A separate inventory payload (Section 1.3.5) is published to `iotkit/v1/{adapter_id}/inventory/{device_key}` (retained). The two payloads differ: the inventory payload includes `session_id`, while the discovery notification does not.

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

The `iotkit/v1/{adapter_id}/inventory/{device_key}` topic carries a Discovery-like payload with an additional `session_id` field (see below). A DeviceLost event triggers an inventory tombstone: an empty retained payload (`b""`) published to the device's inventory topic, clearing the retained message.

**Inventory payload fields** (same as Discovery envelope, plus `session_id` and `first_seen_at`):

All fields from Section 1.3.2, plus:

| Field | JSON type | Nullable | Description |
|---|---|---|---|
| `session_id` | string | No | Runner session identifier (see Section 1.3.7). Allows subscribers to distinguish current vs stale retained inventory. |
| `first_seen_at` | integer | No | Unix timestamp in milliseconds when this device was **first discovered** by the adapter. This value is set at discovery time and never changes across reconnects or inventory reconcile cycles. Must be >= 0. |

Note: The non-retained `discovery` topic (Section 1.3.2) does NOT include `session_id` or `first_seen_at`. Only the retained `inventory/{device_key}` topic includes both fields.

**Timestamp semantics for inventory:**
- `ts`: The time at which this inventory payload was encoded and published. Refreshed on every reconnect reconcile. Use `ts` for **message freshness** (e.g., "how recently was this inventory message published?").
- `first_seen_at`: The original discovery time. Never changes for a given device within a process lifetime. Use `first_seen_at` for **device freshness** (e.g., "how long has this device been known to the adapter?").

Consumers use `first_seen_at` for device freshness and `ts` for message freshness.

**Complete JSON example (inventory):**

```json
{
  "v": 1,
  "adapter_id": "bravepi:0",
  "ts": 1743206400000,
  "session_id": "a1b2c3d4e5f67890fedcba0987654321",
  "device_key": "i2c:0x44:sht31",
  "first_seen_at": 1743206300000,
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

#### 1.3.6 Status Envelope

Published to `iotkit/v1/{adapter_id}/status` with `retain = true`, `QoS = 1`.

Additional fields:

| Field | JSON type | Nullable | Description |
|---|---|---|---|
| `online` | boolean | No | `true` when the adapter comes online; `false` when it goes offline. |
| `session_id` | string | No | Runner session identifier (see Section 1.3.7). |

**Timestamp semantics:**

- **Graceful offline** (adapter shutting down cleanly): `ts = now_ms()` at the moment of shutdown.
- **LWT (Last Will and Testament)** — abnormal disconnect: `ts = 0`. The broker publishes this on the adapter's behalf. The `ts = 0` sentinel allows consumers to distinguish a broker-injected LWT from a gracefully sent offline message.
- **Online**: `ts = now_ms()` at startup.

**LWT and session_id:** The LWT payload is pre-registered at connection time and includes the current `session_id`. This is correct -- the LWT fires for the session that registered it.

Status messages MUST be decoded with `decode_status()`, not `decode_event()`. `decode_event()` returns `DecodeError::InvalidPayload` if called with a status payload.

**Complete JSON example (online):**

```json
{
  "v": 1,
  "adapter_id": "bravepi:0",
  "ts": 1743206000000,
  "online": true,
  "session_id": "a1b2c3d4e5f67890fedcba0987654321"
}
```

**Complete JSON example (graceful offline):**

```json
{
  "v": 1,
  "adapter_id": "bravepi:0",
  "ts": 1743209600000,
  "online": false,
  "session_id": "a1b2c3d4e5f67890fedcba0987654321"
}
```

**Complete JSON example (LWT / abnormal disconnect):**

```json
{
  "v": 1,
  "adapter_id": "bravepi:0",
  "ts": 0,
  "online": false,
  "session_id": "a1b2c3d4e5f67890fedcba0987654321"
}
```

#### 1.3.7 Session ID

The `session_id` is a 32-character lowercase hex string generated once at runner startup and included in all retained messages (`status` and `inventory/{device_key}`). It uniquely identifies a runner process lifetime.

**Generation:** `format!("{:016x}{:016x}", high, low)` where `high = SystemTime::now().duration_since(UNIX_EPOCH).as_nanos() as u64` and `low = std::process::id() as u64 ^ (high.wrapping_mul(0x517cc1b727220a95))`. No external UUID crate is needed. Collisions are astronomically unlikely given nanosecond-precision timestamps combined with process IDs.

**Stability:** The `session_id` is constant for the entire process lifetime. It does NOT change on MQTT reconnect. It changes only on process restart.

**Subscriber protocol for stale inventory detection:**

1. Subscriber receives `status` message with `online: true`. Note the `session_id`.
2. All `inventory/{device_key}` messages with matching `session_id` are current.
3. All `inventory/{device_key}` messages with a different `session_id` (or missing `session_id`) are stale and SHOULD be discarded by the subscriber.
4. On process restart, the new `session_id` in the `status` message invalidates all previously-retained inventory. The subscriber discards old inventory and rebuilds from the new session's inventory messages.

**Why session_id instead of timestamp-based staleness windows:** A fixed 30-second staleness window is fragile -- it breaks if inventory reconcile is delayed (slow broker, many devices, network congestion). The `session_id` provides a definitive current/stale classification with zero timing assumptions.

**Non-retained topics do NOT include `session_id`.** The `telemetry`, `discovery`, `loss`, and `error` topics are non-retained live streams. Late subscribers miss them by design. Adding `session_id` to transient messages would add payload overhead with no benefit.

#### 1.3.8 Subscriber Bootstrap Protocol

MQTT does not guarantee retained message delivery order. A subscriber may receive `inventory/{device_key}` messages before the `status` message for the same adapter. The following protocol handles this correctly:

1. Subscribe to `iotkit/v1/+/status` and `iotkit/v1/+/inventory/+`.
2. Buffer received inventory messages per `adapter_id` (keyed by `device_key`).
3. When a `status` message is received for an `adapter_id`, use its `session_id` to filter the buffered inventory:
   - Inventory with matching `session_id` → current; accept and apply to local state.
   - Inventory with different or missing `session_id` → stale; discard.
4. If a `status` message is not received within 10 seconds for a buffered `adapter_id`, treat all buffered inventory for that adapter as stale and discard it.
5. If the received `status` has `online = false`, inventory messages with a matching `session_id` represent the adapter's **last-known state** before it went offline. These may be accepted and displayed as "last seen" (see Section 3.8.6).
6. If no `status` message exists for an adapter (never published, or broker cleared its retained state), discard all buffered inventory for that adapter. Do not apply inventory without a corresponding authoritative `session_id`.

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
8. **Inventory via wrong function** -- `decode_event()` MUST NOT be called on inventory payloads. Inventory payloads include `session_id` which is not present in discovery payloads. Use `decode_inventory()` for `inventory/{device_key}` topics. `decode_event(EventType::Discovery, ...)` is for non-retained discovery notifications only.

#### 1.4.5 Unknown Fields

Unknown fields in the JSON object MUST be silently ignored (`#[serde(deny_unknown_fields)]` MUST NOT be used). This preserves forward compatibility when new optional fields are added in future minor revisions.

#### 1.4.6 Topic/Payload Identity Consistency

The `adapter_id` and `device_key` values appear in both the MQTT topic (percent-encoded) and the JSON payload (raw). The following rule governs mismatches:

**Payload is authoritative. Topic is for routing only.** If a consumer decodes a message from topic `iotkit/v1/X/telemetry` and the payload contains `adapter_id: Y` where `X != percent_decode(Y)`, the payload's `adapter_id` is authoritative. The same rule applies to `device_key` in inventory topics.

**However, the runner MUST always publish with consistent topic/payload identity.** The runner constructs both the topic and the payload from the same `adapter_id` and `device_key` values, guaranteeing consistency by construction. A topic/payload mismatch in production is a bug in the runner, not a protocol feature.

**No runtime validation in decode functions.** The `decode_event()` and `decode_status()` functions do not accept a topic parameter and do not validate topic/payload consistency. This keeps the decode API simple. Consistency is enforced at the publisher side (runner), not the consumer side.

### 1.5 Adapter Event Mapping

This table defines how each `AdapterEvent` variant is handled by `encode_event()` and what side effects the caller (e.g. the MQTT adapter bridge) must perform.

| `AdapterEvent` variant | `encode_event()` output | `EventType` | Caller side effects |
|---|---|---|---|
| `SensorData` | Encoded | `Telemetry` | Publish to `telemetry` topic (non-retained). |
| `DeviceDiscovered` | Encoded | `Discovery` | Publish to `discovery` topic (non-retained). Also publish a separate inventory payload (via `encode_inventory()`) to `inventory/{device_key}` (retained). The inventory payload includes `session_id`; the discovery notification does not. |
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

    /// Parse a `ConnectionKind` from its string identifier.
    /// Case-sensitive. Returns `Other(s.to_string())` for unrecognised strings
    /// (never fails — unknown kinds are captured via `Other`).
    ///
    /// **Normalization, not strict round-trip:** `from_str("i2c")` returns `I2c`,
    /// while `Other("i2c")` cannot exist in practice because `from_str` normalizes
    /// known strings to their typed variants. The round-trip property holds for
    /// canonical values: `ConnectionKind::from_str(k.as_str()) == k` for all `k`.
    /// However, `ConnectionKind::from_str("i2c").as_str() == "i2c"` regardless of
    /// whether the input was `I2c` or `Other("i2c")` — known strings are always
    /// normalized to the typed variant.
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
pub fn encode_status(adapter_id: &AdapterId, online: bool, ts: i64, session_id: &str) -> Vec<u8>;

/// Encode an inventory payload for a retained `inventory/{device_key}` topic.
///
/// This is separate from `encode_event` because inventory payloads include `session_id`
/// and are re-encoded at publish time (not at event-receipt time).
///
/// `data`: The identity data for the device. `data.first_seen_at` is included verbatim
///         in the payload (original discovery time; must not be refreshed on reconnect).
/// `session_id`: The runner's session identifier (constant per process lifetime).
/// `ts`: Unix milliseconds at the time of publish (refreshed on every reconnect).
///
/// Always returns a `Vec<u8>` (infallible; inventory payloads are statically structured).
pub fn encode_inventory(
    adapter_id: &AdapterId,
    data: &InventoryData,
    session_id: &str,
    ts: i64,
) -> Vec<u8>;

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/// Decode a non-status, non-inventory event payload.
///
/// `event_type`: The type inferred from the MQTT topic. MUST NOT be `EventType::Status`
///               or `EventType::Inventory`. Use `decode_inventory()` for inventory payloads.
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
/// Returns `(adapter_id, online, ts, session_id)` on success.
/// `ts` is the Unix timestamp in milliseconds. `ts = 0` is accepted (LWT sentinel).
/// Other negative `ts` values are rejected.
/// `session_id` is the runner's session identifier (32-char hex string).
///
/// The caller needs `ts` to distinguish LWT offline (ts=0) from graceful offline (ts>0)
/// and to record when the adapter went online/offline. The caller uses `session_id`
/// to correlate inventory messages with the current session (Section 1.3.7).
///
/// MUST be called for payloads received on the `.../status` topic.
/// MUST NOT be used for other event types.
pub fn decode_status(payload: &[u8]) -> Result<(AdapterId, bool, i64, String), DecodeError>;
// Returns (adapter_id, online, ts, session_id)

/// Decode an inventory payload from a retained `inventory/{device_key}` topic.
///
/// Returns `(adapter_id, adapter_event, session_id, first_seen_at)` on success.
/// The `adapter_event` is an `AdapterEvent::DeviceDiscovered` variant.
/// The `session_id` is the runner's session identifier (32-char hex string),
/// used by subscribers to distinguish current vs stale retained inventory
/// (Section 1.3.7).
/// The `first_seen_at` is the original discovery time in Unix milliseconds
/// (Section 1.3.5). Use it for device freshness; use `ts` for message freshness.
///
/// MUST be called for payloads received on `.../inventory/{device_key}` topics.
/// MUST NOT be used for non-retained discovery payloads (use `decode_event` instead).
///
/// Returns `DecodeError` if:
/// - The payload is not valid UTF-8 JSON.
/// - The `v` field is not `1`.
/// - Any required field (including `session_id`, `first_seen_at`) is missing or has the wrong type.
/// - `ts < 0` or `first_seen_at < 0`.
pub fn decode_inventory(payload: &[u8]) -> Result<(AdapterId, AdapterEvent, String, i64), DecodeError>;
// Returns (adapter_id, device_discovered_event, session_id, first_seen_at)

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
    Done [label="Done", shape=doubleoctagon];
    Failed [label="Failed", shape=doubleoctagon];

    Connecting -> Reconnecting [label="MQTT client created,\neventloop_task spawned\n/ enter event loop"];
    Connecting -> Failed [label="MQTT client creation fails\n/ return Err"];

    Reconnecting -> Online [label="ConnAck received\n/ conn_tx.send(Connected)"];
    Reconnecting -> Done [label="event_rx closed\n/ publish offline + return Ok"];
    Reconnecting -> Failed [label="eventloop_task exits\nunexpectedly\n/ return Err"];

    Online -> Reconnecting [label="Disconnect detected\n/ conn_tx.send(Disconnected)"];
    Online -> Done [label="event_rx closed\n/ publish offline + return Ok"];
    Online -> Failed [label="eventloop_task exits\nunexpectedly\n/ return Err"];

    Done [label="Done\n(offline status published)", shape=doubleoctagon];
}
```

The runner NEVER initiates shutdown. The runner NEVER knows about signals. It simply processes events until `event_rx` closes, then publishes offline status.

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

    ShuttingDown -> Exit0 [label="runner returns Ok\n(clean shutdown)"];
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
| Offline status publish | NO | YES (on event_rx close) |

#### 2.1.4 Shutdown Flow

1. Binary catches SIGINT/SIGTERM.
2. Binary calls `adapter_handle.shutdown().await` with a 5-second timeout. This stops the adapter producer and drops `event_tx`, causing `event_rx` to close.
3. Runner detects `event_rx` closure (recv returns None) -> publish_task breaks loop -> runner publishes offline status -> grace period -> returns `Ok(())`.
4. Binary checks runner result -> exit 0 on Ok, exit 1 on Err.
5. If adapter shutdown hangs beyond 5 seconds, binary aborts and exits 1.
6. If a 2nd signal arrives during shutdown, binary calls `std::process::exit(1)` immediately.

#### Transition Table (Runner)

| From | To | Trigger | Actions |
|---|---|---|---|
| Connecting | Reconnecting | MQTT client + eventloop created | Spawn eventloop_task, spawn publish_task, enter event loop |
| Connecting | Failed | MQTT client creation fails | Return `Err(RunnerError::MqttInit(...))` |
| Reconnecting | Online | ConnAck received by eventloop_task | `conn_tx.send(Connected)` |
| Online | Reconnecting | Disconnect detected by eventloop_task | `conn_tx.send(Disconnected)` |
| Reconnecting/Online | Done | `event_rx` closes (recv returns None) | publish_task exits loop; runner publishes offline status, grace period, return `Ok(())` |
| Reconnecting/Online | Failed | eventloop_task exits unexpectedly (detected via `select!` on `eventloop_join`) | Abort publish_task; return `Err(RunnerError::EventLoopDied)` |

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

The runner spawns two internal tasks. Each task has **exclusive ownership** of its state. No `Mutex` or `RwLock` exists. Cross-task coordination uses a single `tokio::sync::watch` channel for connection state.

**Important:** The runner does NOT spawn signal handlers or own the adapter's ShutdownHandle. Signal handling and adapter lifecycle are the binary's responsibility (see Section 2.1.2).

#### 2.2.1 Connection State Coordination

Connection state is communicated via a `tokio::sync::watch` channel, replacing the previous `AtomicBool + Notify` split:

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
enum ConnectionState { Disconnected, Connected }

let (conn_tx, conn_rx) = tokio::sync::watch::channel(ConnectionState::Disconnected);
```

**Why watch instead of AtomicBool + Notify:** `watch` is **level-triggered**: the receiver always sees the latest state. This eliminates the race condition where `AtomicBool` could be stale relative to `Notify` wakeups. If eventloop_task sends `Connected` then immediately `Disconnected`, publish_task sees `Disconnected` and does not reconcile. With `AtomicBool + Notify`, the publish_task would wake on the `Notify`, read `AtomicBool = false`, and silently miss the brief connection.

#### 2.2.2 eventloop_task (spawned tokio task)

**Owns:**
- `rumqttc::EventLoop` (the MQTT event loop instance)
- `conn_tx: watch::Sender<ConnectionState>` -- sends connection state changes

**Behavior:**
- Runs `eventloop.poll().await` in a loop.
- On `Event::Incoming(Packet::ConnAck(_))`: calls `conn_tx.send(Connected)`. **Does NOT call `client.publish()`.** All publish operations happen in publish_task to avoid deadlock (see Section 3.4.1).
- On connection error or disconnect: calls `conn_tx.send(Disconnected)`.
- rumqttc handles reconnection internally; this task does not exit on transient disconnects.
- Task exits only when: (a) `EventLoop` is dropped/aborted by the runner on cleanup, or (b) an unrecoverable internal error.

**Return type:** `Result<(), EventLoopError>` -- runner inspects this on join.

#### 2.2.3 publish_task (spawned tokio task)

**Owns (exclusive, moved in):**
- `event_rx: mpsc::Receiver<AdapterEvent>` -- receives adapter events
- `desired_inventory: HashMap<String, Option<InventoryData>>` -- sole inventory model (see Section 3.8)
- `session_id: String` -- 32-char hex string generated at runner startup (see Section 1.3.7), constant for process lifetime
- `conn_rx: watch::Receiver<ConnectionState>` -- receives connection state changes
- `client: rumqttc::AsyncClient` (clone) -- used for `publish()` / `publish_bytes()` calls

**Thread safety of `desired_inventory`:** This HashMap is exclusively owned by publish_task. No sharing, no Mutex needed.

**No outbound buffer.** Non-retained events (telemetry, discovery notification, loss notification, error) are dropped when disconnected. Retained state (status, inventory) is tracked in `desired_inventory` and reconciled on every ConnAck.

**Disconnect policy:**
- **Retained ops (status, inventory):** Tracked in `desired_inventory`. Reconciled on every ConnAck. MUST NOT lose state.
- **Non-retained events (telemetry, discovery notification, loss notification, error):** Drop when disconnected. Emit `warn!` log per dropped event. No buffer.

**Telemetry loss tradeoff:** At 10 sensors x 1Hz and a typical 30-second disconnect, approximately 300 telemetry readings are lost. This is acceptable because: (a) the gateway maintains timeseries history for longer analysis, (b) telemetry is time-series data with diminishing value when stale, (c) eliminating the outbound buffer removes significant complexity (buffer management, replay fairness, drain phase, backpressure, ordering races).

**Behavior loop:**
```
loop {
    tokio::select! {
        event = event_rx.recv() => {
            match event {
                Some(ev) => {
                    // Always track inventory for lifecycle events
                    track_inventory(&ev);

                    if *conn_rx.borrow() == Connected {
                        // Encode + publish with timeout
                        publish_event_with_timeout(&ev);
                    } else {
                        // Non-retained: drop with warn log
                        // Retained (inventory): already tracked above, will reconcile on ConnAck
                        warn!("disconnected, dropping non-retained event");
                    }
                }
                None => break, // event_rx closed, exit task
            }
        }

        _ = conn_rx.changed() => {
            if *conn_rx.borrow() == Connected {
                // Reconcile: publish online status + all desired_inventory
                reconcile_all();
            }
            // If Disconnected: no action needed. Next event_rx.recv() will
            // check conn_rx.borrow() and drop non-retained events.
        }
    }
}
```

**Reconcile on ConnAck:** When `conn_rx.changed()` delivers `Connected`, the publish_task publishes online status followed by the full `desired_inventory`. This is the only path that publishes retained state. Steps:

1. Publish online status (`encode_status(adapter_id, true, now_ms(), session_id)`, retained).
2. For each entry in `desired_inventory`: publish retained inventory (see Section 3.8.3).
3. Resume normal event processing.

Each publish call uses a 5-second timeout (see Section 3.7.2).

**Reconcile is fail-fast.** If any publish during the reconcile loop fails (timeout or error):
- Log a `warn!` with the count of remaining unreconciled entries.
- Stop reconcile immediately (do not continue to the next entry).
- Stay in the connected state (do not transition to Disconnected; the eventloop handles disconnect detection independently).
- The next `ConnAck` will retry full reconcile from scratch.

**Rationale for fail-fast:** A best-effort reconcile that skips failures can stall for `N × 5s` (up to `100 × 5s = 500s`) on a half-broken connection where publishes time out individually. Fail-fast limits the stall to one 5-second timeout and yields back to the `select!` loop, allowing event processing to resume and eventloop disconnect detection to fire promptly.

**Reconcile failure (disconnect mid-reconcile):** If MQTT disconnects during reconcile, the first publish timeout (5s) triggers fail-fast exit. `conn_rx.changed()` will deliver `Disconnected` on the next `select!` iteration. On the next `ConnAck`, reconcile restarts from scratch.

**Reconcile duration:** For typical deployments (< 100 devices), reconcile takes < 100ms (bounded by device count, not buffer size).

**watch channel coalescing:** If eventloop_task sends `Connected` then immediately `Disconnected` before publish_task reads, `conn_rx.changed()` delivers the latest (`Disconnected`). The publish_task does not reconcile. This is correct behavior.

**Message ordering on reconnect:** On ConnAck, publish_task publishes online status first, then inventory. Both are retained and represent current state. No ordering guarantee is needed relative to non-retained events (they are not buffered).

**Return type:** `Result<(), PublishTaskError>` -- runner inspects on join.

**`event_rx` channel capacity:** The `event_rx` channel is created by the adapter (not the runner). The adapter uses `tokio::sync::mpsc::channel(capacity)` with a bounded capacity. The runner does not specify or constrain this capacity. If the adapter fills the channel (e.g., publish_task is slow), the adapter's `send().await` will backpressure naturally. The adapter is responsible for choosing an appropriate capacity for its production rate.

#### 2.2.4 runner main task (the `run()` async fn)

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
// Monitor BOTH tasks. The runner must detect eventloop_task death
// even if no adapter events are arriving (publish_task would hang).
tokio::select! {
    publish_result = &mut publish_join => {
        // publish_task exited (event_rx closed or error).
        // Proceed to offline status + cleanup.
    }
    eventloop_result = &mut eventloop_join => {
        // eventloop died unexpectedly — this is fatal.
        // Signal publish_task to stop by dropping the abort handle,
        // then return Err(RunnerError::EventLoopDied).
        publish_join.abort();
        return Err(RunnerError::EventLoopDied(eventloop_result));
    }
}

// Normal path: publish_task exited first (event_rx closed).
// Publish offline status with timeout
match timeout(5s, client.publish(status_topic, QoS1, retained=true, offline_payload)).await {
    Ok(Ok(())) => {},
    _ => warn!("failed to publish offline status, LWT will fire as fallback"),
}
client.disconnect().await;

// Grace period: let eventloop transmit the above
timeout(2s, eventloop_join).await;
// If eventloop didn't finish, abort it
eventloop_join.abort();

// Return Ok or Err based on publish_result
```

**Why select! on both handles:** If the runner only awaited `publish_join`, and the eventloop died while no adapter events were arriving, publish_task would block on `event_rx.recv()` indefinitely. The runner would hang. By selecting on both handles, the runner detects eventloop death immediately and returns `Err(RunnerError::EventLoopDied)`. The binary then logs the error and exits 1.

#### 2.2.5 Shared State Summary

| Item | Type | Writer | Reader(s) |
|---|---|---|---|
| `conn_tx` / `conn_rx` | `watch::Sender<ConnectionState>` / `watch::Receiver<ConnectionState>` | eventloop_task (`conn_tx`) | publish_task (`conn_rx`) |

No other shared mutable state exists. `AsyncClient` is cloned (rumqttc's `AsyncClient` is backed by an internal channel and is `Clone + Send`); each clone is independently owned.

`desired_inventory` is exclusively owned by publish_task -- no sharing needed.

### 2.3 Task Supervision

#### 2.3.1 Runner-Internal Task Monitoring

The runner's `run()` function monitors its two internal tasks **concurrently** via `select!`:

| Source | Monitoring method | Condition |
|---|---|---|
| publish_task exit | `select!` branch on `publish_join` | JoinHandle resolves (normal: event_rx closed) |
| eventloop_task exit | `select!` branch on `eventloop_join` | JoinHandle resolves (fatal: eventloop died unexpectedly) |

Both handles are monitored simultaneously. If eventloop_task exits first, the runner immediately returns `Err(RunnerError::EventLoopDied)` without waiting for publish_task. If publish_task exits first (normal shutdown path), the runner proceeds with offline status publish and eventloop cleanup.

The runner does NOT monitor signals. Signal handling is the binary's responsibility.

#### 2.3.2 Runner Return Values

| Event | Runner action | Return value |
|---|---|---|
| publish_task exits Ok (event_rx closed) | Publish offline status with timeout, gracefully stop eventloop | `Ok(())` |
| publish_task exits Err/panic | Log error, abort eventloop | `Err(RunnerError::PublishTaskFailed)` |
| eventloop_task exits unexpectedly (before publish_task) | Runner detects immediately via `select!` on `eventloop_join`; aborts publish_task | `Err(RunnerError::EventLoopDied)` |

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
        v  [fail: log error, exit 1]
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
    - MqttOptions with LWT: topic=`{prefix}/status`, payload=`{"v":1,"adapter_id":"...","ts":0,"online":false,"session_id":"..."}`, retain=true, QoS 1
    - AsyncClient::new(options, cap=100)
        |
        v  [fail: return Err(RunnerError::MqttInit(...))]
(2) Spawn eventloop_task
    - tokio::spawn(eventloop_run(eventloop, conn_tx))
    - Returns JoinHandle
        |
        v  [fail: impossible -- spawn does not fail]
(3) Spawn publish_task
    - tokio::spawn(publish_run(event_rx, client.clone(), conn_rx, ...))
    - Returns JoinHandle
        |
        v  [fail: impossible -- spawn does not fail]
(4) Select! on both task handles (see Section 2.2.3)
    - Awaits EITHER publish_task OR eventloop_task completion
    - Normal path: publish_task exits first (event_rx closure) -> cleanup
    - Fatal path: eventloop_task exits first -> return Err immediately
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
(2) publish_task exits loop, returns Ok(())
    |
    v
(3) Offline status publish (with 5s timeout)
    - runner publishes retained status: {"v":1,"adapter_id":"...","ts":<real_unix_ms>,"online":false,"session_id":"..."}
    - Topic: {prefix}/status, QoS 1, retain=true
    - This overwrites the LWT's ts=0 with a real timestamp
    - If timeout fires: warn log, proceed (LWT serves as fallback)
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
- `Ok(())` = event_rx closed cleanly, offline status published (or timed out, best-effort).
- `Err(RunnerError)` = MQTT fatal error at startup, or eventloop/publish_task panicked.

**What happens to eventloop_task when runner returns?** The runner aborts it in step 6. The binary does not need to manage it.

#### Timeout Budget

| Phase | Owner | Max duration |
|---|---|---|
| Adapter shutdown | Binary | 5 seconds |
| Offline status publish | Runner | 5 seconds (timeout) |
| Disconnect + eventloop grace | Runner | 2 seconds |
| Eventloop abort | Runner | ~0 (immediate) |
| **Total maximum** | | **5 + 7 = 12 seconds** |

Note: The binary's 5s adapter timeout runs concurrently with (not before) the runner's shutdown sequence. In practice, once the adapter drops event_tx, the runner begins shutdown immediately, so the actual wall time is typically 5s + small overlap.

#### 2.5.3 2nd Signal During Shutdown

If a second SIGINT or SIGTERM arrives while the binary's shutdown sequence is in progress, the binary calls `std::process::exit(1)` immediately. This bypasses the offline status publish and eventloop grace period. The LWT (with ts=0) will inform subscribers of the ungraceful exit.

#### 2.5.4 Adapter Shutdown Hangs

If `adapter_handle.shutdown()` does not complete within 5 seconds, the binary:
1. Logs a warning: "adapter shutdown timed out after 5s"
2. Aborts the runner task.
3. Exits with code 1.

The LWT fires after the broker's keepalive timeout (default 30s), providing eventual offline notification.

### 2.6 Exit Code Contract

| Scenario | Exit code | Rationale |
|---|---|---|
| Config validation failure | 1 | Cannot run with invalid config |
| Adapter start failure | 1 | No data source |
| Runner returns Err (MQTT init failure) | 1 | Cannot connect to broker |
| Runner returns Err (task panic/crash) | 1 | Critical infrastructure gone |
| Signal + runner returns Ok (clean shutdown) | 0 | Graceful shutdown |
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
- **Action:** `conn_tx.send(Disconnected)`. rumqttc reconnects automatically with exponential backoff.
- **publish_task behavior:** Stops publishing non-retained events (drops with warn log). Continues tracking inventory in `desired_inventory`. Waits on `conn_rx.changed()` for reconnection.
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
| eventloop_task exit (unexpected) | Runner detects via `select!` on `eventloop_join` (immediate) | Runner returns Err, binary logs + exits 1 |
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
  -> publish_task exits loop
  -> runner publishes offline status (with timeout), cleans up eventloop
  -> runner returns Ok(())
  -> binary checks: shutdown_initiated == true, runner Ok -> exit 0
```

#### 2.8.3 Adapter Crash Path

```
adapter panics / drops event_tx unexpectedly
  -> event_rx.recv() returns None in runner's publish_task
  -> publish_task exits loop
  -> runner publishes offline status (with timeout), cleans up eventloop
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
/// it publishes offline status and returns.
///
/// The runner does NOT handle signals or own the adapter. The caller (binary)
/// is responsible for:
/// - Installing signal handlers
/// - Calling adapter shutdown (which closes event_rx)
/// - Interpreting the return value for exit code decisions
///
/// Returns Ok(()) on clean event_rx closure.
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

The connection state is tracked via a `tokio::sync::watch` channel (see Section 2.2.1). The eventloop_task sends `ConnectionState::Connected` on ConnAck and `ConnectionState::Disconnected` on error/disconnect. The publish_task reads the latest state via `conn_rx.borrow()` and receives change notifications via `conn_rx.changed()`. Because `watch` is level-triggered, the publish_task always sees the latest state -- there is no race between state update and notification delivery.

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

    Disconnected [label="Disconnected\n(watch=Disconnected)"];
    Connected [label="Connected\n(watch=Connected)"];

    Disconnected -> Disconnected [label="poll() -> Err\n(backoff sleep)"];
    Disconnected -> Connected [label="poll() -> ConnAck\n-> conn_tx.send(Connected)\n-> reset attempt=0"];
    Connected -> Connected [label="poll() -> Ok(non-ConnAck)\n(PUBACK, PINGRESP, etc.)"];
    Connected -> Disconnected [label="poll() -> Err\n-> conn_tx.send(Disconnected)\n-> increment attempt\n-> backoff sleep"];
}
```

### 3.4 ConnAck Processing (identical for initial and reconnect)

On every `ConnAck`, the following sequence executes **in order**:

**eventloop_task** (steps 1-2):

1. `conn_tx.send(Connected)`
2. `reconnect_attempt = 0`

**eventloop_task does NOT call `client.publish()`.** See Section 3.4.1 for deadlock rationale.

**publish_task** (steps 3-5, after `conn_rx.changed()` delivers `Connected`):

3. **Publish online status** -- `timeout(5s, encode_status(adapter_id, true, now_ms(), session_id))` to `iotkit/v1/{adapter_id}/status`, QoS 1, retained. Timeout -> warn + skip.
4. **Inventory reconcile** -- for each entry in `desired_inventory`, publish retained inventory with current `session_id` + fresh `ts` (see Section 3.8). Each publish has a 5-second timeout. Failures are warned and skipped; the entry will be retried on the next ConnAck.
5. **Resume live event processing** -- the `select!` loop continues normally. Incoming events from `event_rx` are published immediately (while connected).

All reconcile steps run synchronously within the `conn_rx.changed()` branch of the `select!` loop. Reconcile is bounded by device count (not buffer size). For typical deployments (< 100 devices), reconcile takes < 100ms.

#### 3.4.1 Deadlock Prevention: eventloop_task Must Not Publish

**Problem:** If eventloop_task calls `AsyncClient::publish().await` inside the ConnAck handler, and rumqttc's internal channel is full (capacity 100), the `publish()` call blocks waiting for the eventloop to drain the channel. But eventloop IS the blocked task — deadlock.

**Solution:** eventloop_task MUST NOT call `client.publish()`. It ONLY:
1. Sends connection state via `conn_tx.send(Connected)` or `conn_tx.send(Disconnected)`.

All publish operations (online status, inventory reconcile, live events) happen in publish_task. Since publish_task and eventloop_task are separate tokio tasks, `publish_task`'s `publish().await` enqueues to the channel while eventloop_task independently polls and drains it. No deadlock is possible.

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

- The binary receives SIGTERM/SIGINT, shuts down the adapter, which closes `event_rx`, causing the runner to publish offline status and exit.
- A fatal configuration error prevents even TCP connect attempts (invalid hostname -- rumqttc will still retry).

During extended disconnection:
- The adapter continues producing events into `event_rx`.
- The publish_task continues consuming from `event_rx`, tracking inventory locally in `desired_inventory`, and dropping non-retained events (with warn log).
- The eventloop pump task continues calling `poll()` with backoff sleeps.

### 3.7 Publish Policy

#### 3.7.1 Definition of "Publish Succeeded"

`AsyncClient::publish().await` returning `Ok(())` means the message has been **enqueued into rumqttc's internal bounded channel** (capacity: 100, set at `AsyncClient::new(opts, 100)`). It does **not** mean:

- The message has been written to the TCP socket.
- The message has been received by the broker.
- A `PUBACK` has been received (for QoS 1).

This is a fundamental constraint of rumqttc's API. The runner **must not** treat enqueue success as delivery confirmation for any correctness-critical operation.

**rumqttc backpressure model:** `AsyncClient::publish().await` **blocks** (does not return an error) when rumqttc's internal channel is full (capacity 100). The eventloop_task must be polling to drain this channel. If the eventloop is healthy, blocking resolves quickly. If the eventloop is stuck, the publish call blocks indefinitely.

**Mandatory publish timeout:** ALL `client.publish()` calls in publish_task and the runner MUST be wrapped with a 5-second timeout:

```rust
match tokio::time::timeout(Duration::from_secs(5), client.publish(...)).await {
    Ok(Ok(())) => { /* enqueued successfully */ }
    Ok(Err(e)) => { warn!("publish error: {e}"); /* skip */ }
    Err(_) => { warn!("publish timed out after 5s"); /* skip */ }
}
```

Timeout or error -> warn + skip (not fatal). For retained operations, the next ConnAck will reconcile. For non-retained events, the data is lost (accepted).

#### 3.7.2 Disconnect Policy by Event Class

Events are classified into two categories based on their MQTT retain semantics and recoverability:

| Event class | Events | Retained? | Disconnect policy | Rationale |
|-------------|--------|-----------|-------------------|-----------|
| **Inventory/Status (recoverable)** | `DeviceDiscovered`, `DeviceLost`, status online/offline | Yes | MUST NOT lose state. Tracked in `desired_inventory` (HashMap). Re-published on every ConnAck. | These represent current device state. Local tracking makes them fully recoverable. |
| **Transient (non-recoverable)** | `SensorData` (telemetry), `AdapterError`, non-retained copies of discovery/loss events | No | **Drop immediately.** Emit `warn!` log. No buffer. | Telemetry is time-series data; stale readings have diminishing value. Dropping eliminates buffer/replay complexity entirely. |

**No outbound buffer exists.** This is a deliberate simplification. The previous design included a bounded `VecDeque<PublishItem>` buffer with replay, fairness, and drain logic. This has been removed because:

1. Retained state (inventory, status) is fully recoverable via `desired_inventory` + ConnAck reconcile.
2. Non-retained events have diminishing value when stale.
3. The buffer was the source of nearly all complexity: replay fairness, ordering, backpressure, reconcile ordering race, drain phase.
4. Quantitative impact: 10 sensors x 1Hz x 30s disconnect = 300 readings lost. The gateway has timeseries history for longer analysis. Brief disconnects (100ms) lose only a few readings.

#### 3.7.3 What Happens When Disconnected

When disconnected, for each incoming `AdapterEvent`:

1. **Always:** `track_inventory(&event)` -- updates `desired_inventory` HashMap for `DeviceDiscovered` and `DeviceLost` events. No-op for other events.
2. **Non-retained events:** Drop with `warn!` log. No buffering.
3. **Retained inventory updates:** Already tracked in step 1. Will be published on next ConnAck.

Discovery/loss events are NOT published to their non-retained topics during disconnect. Only the retained inventory state in `desired_inventory` is preserved. On reconnect, inventory is reconciled but the non-retained discovery/loss notifications are not replayed (subscribers get the current inventory state via retained topics, which is sufficient).

#### 3.7.4 ConnAck Reconcile Order

On `conn_rx.changed()` delivering `Connected`, the publish_task executes in strict order:

```
(1) Online status   -- publish_task publishes encode_status(..., online=true, ...) (retained, with session_id)
(2) Inventory reconcile -- for each entry in desired_inventory: publish retained inventory (with session_id)
(3) Resume           -- select! loop continues, events published normally
```

All steps execute synchronously within the `conn_rx.changed()` branch. Each publish call uses a 5-second timeout. Reconcile is fail-fast: the first publish failure (timeout or error) stops reconcile immediately, logs the count of remaining unreconciled entries, and returns control to the `select!` loop. All entries are retried on the next ConnAck.

### 3.8 Retained Inventory Semantics

#### 3.8.1 Data Model — Single `desired_inventory`

There is ONE inventory model. No separate `pending_retained_ops`. All inventory state lives in a single HashMap:

```rust
/// device_key (String) -> Some(identity_data) for active, None for tombstone
desired_inventory: HashMap<String, Option<InventoryData>>
```

Where `InventoryData` is a struct containing the identity fields needed to re-encode the inventory payload at publish time:

```rust
struct InventoryData {
    device_key: String,
    identity: SensorIdentity,  // manufacturer, ic_part_number, sensor_type, connection
    first_seen_at: i64,        // Unix ms timestamp; set at discovery time, never changes
}
```

**Why store structured data instead of pre-encoded bytes:** The `session_id` and `ts` fields must be set at publish time, not at event-receipt time. If `desired_inventory` stored pre-encoded `Vec<u8>`, `republish_all` would publish payloads with stale timestamps and (after a restart) wrong `session_id`. By storing the identity data, encoding happens at publish time with the current `session_id` (runner-level constant) and a fresh `ts = now_ms()`.

This HashMap is the **sole source of truth** for device inventory. It is exclusively owned by publish_task (no sharing, no Mutex). The broker's retained message store is treated as a cache that is unconditionally overwritten on every reconnect.

On ConnAck, replay **everything** in `desired_inventory`. This is simple and correct.

| `desired_inventory` value | Meaning | MQTT action on publish/reconcile |
|---------------------------|---------|----------------------------------|
| `Some(data)` | Device is active | Encode `data` with current `session_id` + `ts = now_ms()`, publish to `inventory/{device_key}`, QoS 1, **retained** |
| `None` | Device was lost (tombstone) | Publish **empty payload** (`Vec::new()`) to `inventory/{device_key}`, QoS 1, **retained** |
| Key absent | Device never seen or process restarted | No action |

#### 3.8.2 Event Tracking

**`DeviceDiscovered`:**

1. Extract identity data from the event into an `InventoryData` struct. Set `first_seen_at = now_ms()` at this moment (first discovery). If the device was previously in `desired_inventory` as `Some(data)` (re-discovery), preserve the original `data.first_seen_at` rather than resetting it to `now_ms()`.
2. `desired_inventory.insert(device_key_str, Some(inventory_data))`.
3. If connected: encode with current `session_id` + `ts = now_ms()`, publish to `iotkit/v1/{adapter_id}/inventory/{device_key}`, QoS 1, retained.

**`DeviceLost`:**

1. `desired_inventory.insert(device_key_str, None)` -- overwrites any previous `Some(data)`.
2. If connected: publish **empty bytes** to `iotkit/v1/{adapter_id}/inventory/{device_key}`, QoS 1, retained. This clears the broker's retained message for that topic.

**All other events:** No inventory tracking. `track_event` returns `false`.

#### 3.8.3 Reconnect Reconciliation

On every `ConnAck`, the publish_task iterates **all entries** in `desired_inventory`:

```
for (device_key_str, maybe_data) in &desired_inventory:
    topic = inventory_topic(adapter_id, device_key_str)
    match maybe_data:
        Some(data) =>
            payload = encode_inventory(adapter_id, data, session_id, now_ms())
            client.publish(topic, QoS1, retained=true, payload).await
        None =>
            client.publish(topic, QoS1, retained=true, Vec::new()).await
```

This is an **unconditional full overwrite** of the broker's retained inventory state. There is no diffing, no checking what the broker currently holds. Every active device gets a freshly-encoded payload (with the current `session_id` and `ts = now_ms()`). Every tombstone gets an empty retained publish.

**Why unconditional overwrite:** With `clean_session=true`, the runner cannot know which QoS 1 publishes were actually delivered before the previous disconnection. A retained message may have been enqueued to rumqttc but never transmitted. The only safe strategy is to republish everything.

**Why re-encode on every reconcile:** The `session_id` is constant per process, but `ts` must reflect the actual publish time. Re-encoding ensures subscribers see a fresh timestamp on every reconnect, making it easy to identify the most recent reconciliation.

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
- **Cleared on process restart.** When the adapter process starts fresh, `desired_inventory` is empty. A new `session_id` is generated. The adapter will re-discover devices, populating `desired_inventory` with fresh `Some(data)` entries. Devices that no longer exist simply won't appear. Subscribers use the new `session_id` to discard stale inventory from the previous session.

**Why keep tombstones forever:** A tombstone publish (`empty retained`) may have been enqueued to rumqttc but never delivered (the connection dropped before TCP write). Without per-message delivery confirmation, the only safe strategy is to re-send tombstones on every reconnect.

#### 3.8.6 Graceful Shutdown Inventory Behavior

On graceful shutdown (adapter closes `event_rx`, publish_task exits):

1. **Inventory is NOT cleared.** No tombstone publishes for active devices. Retained inventory messages remain on the broker.
2. **Only status changes:** Offline status is published (`encode_status(adapter_id, false, now_ms(), session_id)`).
3. Devices may still be visible to consumers via retained inventory topics.

**Normative rule for consumers:** When `status=offline`, inventory represents last-known state. Consumers SHOULD display devices as "last seen". Consumers MUST NOT assume devices are currently reachable when the adapter is offline.

Consumers use `first_seen_at` for device freshness (when was this device originally discovered?) and `ts` for message freshness (when was this inventory message last published?).

**Rationale:** In a multi-adapter deployment, other adapters may be publishing to the same broker. Clearing inventory on shutdown would create a false "all devices gone" signal. The offline status is sufficient for consumers to know this adapter is no longer active.

#### 3.8.7 Crash (Ungraceful Shutdown)

On crash or SIGKILL:

1. **LWT fires:** Broker publishes offline status with `ts=0` (timestamp unknown at LWT registration time). The LWT payload is `encode_status(adapter_id, false, 0, session_id)`. The `session_id` is the one from the current process -- this is correct because the LWT fires for the session that registered it.
2. **Inventory stays stale:** Retained inventory messages remain on the broker with the last-known payloads. No tombstones are published.
3. **On restart:** The new process starts with empty `desired_inventory`. As the adapter re-discovers devices, `republish_all` on the first `ConnAck` publishes only the currently-active devices. Devices that no longer exist will have stale retained messages on the broker **until the broker retains them indefinitely or another mechanism clears them**.

**Stale inventory cleanup after restart uses `session_id` (Section 1.3.7).** The adapter runner includes a `session_id` in every retained message (status + inventory). On process restart, a new `session_id` is generated. Subscribers use this to definitively classify inventory as current or stale:

The expected protocol:
1. Subscriber receives `status` message with `online: true`. Notes the `session_id`.
2. All `inventory/{device_key}` messages with matching `session_id` are current.
3. All `inventory/{device_key}` messages with a different or missing `session_id` are stale. The subscriber removes them from its local state.
4. On MQTT reconnect (same process), the `session_id` does not change. Inventory reconcile re-publishes all entries with the same `session_id` and fresh timestamps. Subscribers see refreshed timestamps but the same session, confirming continuity.

**Why session_id instead of a timing window:** A fixed staleness window (e.g. 30 seconds) is fragile -- it breaks if inventory reconcile is delayed by a slow broker, many devices, or network congestion. The `session_id` provides a definitive current/stale classification with zero timing assumptions. The cost is one additional 32-byte field in retained messages only (non-retained messages are unaffected).

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

- When connected: publish with 5-second timeout. If `Ok`, considered "best-effort sent". No retry. If timeout or error, logged and dropped.
- When disconnected: dropped immediately with `warn!` log. No buffer.
- There is **no application-level acknowledgment** for transient events. Data loss is possible and accepted.

#### 3.9.4 Graceful Offline Publish -- Eventloop Grace Period

On event_rx closure, the runner (not the binary) publishes offline status and gracefully stops:

```rust
// Runner's cleanup after publish_task exits:
// Publish offline status (with timeout)
match timeout(Duration::from_secs(5), client.publish(status_topic, QoS1, retained=true, offline_payload)).await {
    Ok(Ok(())) => {},
    _ => warn!("failed to publish offline status, LWT will fire as fallback"),
}
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
| `poll()` returns `ConnAck` | Disconnected | `conn_tx.send(Connected)`, reset attempt | Connected | publish_task publishes online status + reconciles inventory |
| `poll()` returns `ConnAck` | Connected | `conn_tx.send(Connected)`, reset attempt | Connected | Duplicate ConnAck (unusual but safe); full reconcile re-runs |
| `poll()` returns `Err(ConnectionError)` | Connected | `conn_tx.send(Disconnected)`, increment attempt, sleep(backoff) | Disconnected | `warn!` log; publish_task starts dropping non-retained events |
| `poll()` returns `Err(Timeout)` | Connected | `conn_tx.send(Disconnected)`, increment attempt, sleep(backoff) | Disconnected | Same as ConnectionError |
| `poll()` returns `Err` | Disconnected | increment attempt, sleep(backoff) | Disconnected | `warn!` log; backoff grows |
| `poll()` returns `Ok(PubAck)` | Connected | no-op | Connected | rumqttc retires internal pending |
| `poll()` returns `Ok(PingResp)` | Connected | no-op | Connected | Keepalive confirmed |
| Broker sends DISCONNECT | Connected | Next `poll()` returns Err | Disconnected | Handled via Err path |
| TCP RST from broker | Connected | Next `poll()` returns Err | Disconnected | Handled via Err path |
| DNS resolution fails | Disconnected | `poll()` returns Err | Disconnected | Backoff continues |
| `event_rx` closed (adapter stopped by binary) | Any | publish_task exits loop; runner publishes offline status (5s timeout); 2s grace; eventloop aborted; runner returns Ok | Terminated | Runner exits cleanly; binary decides exit code |

#### 3.10.2 Publish

| Trigger | Current state | Action | Next state | Observable effect |
|---------|--------------|--------|------------|-------------------|
| `AdapterEvent` received | Connected | `track_inventory` + publish with 5s timeout (inventory retained w/ session_id + non-retained) | Connected | Event published to broker (enqueued) |
| `AdapterEvent` received | Disconnected | `track_inventory` + drop non-retained with `warn!` | Disconnected | Inventory tracked locally; non-retained events lost |
| `conn_rx.changed()` -> Connected | publish_task | Publish online status (w/ session_id, 5s timeout); for each `desired_inventory` entry: publish retained (5s timeout each) | Connected | Online status published; inventory reconciled |
| `client.publish` timeout/error during reconcile | Reconciling | `warn!` with remaining count + stop reconcile immediately | Connected | All unreconciled entries retried on next ConnAck |
| `client.publish` timeout/error for live event | Connected | Log `warn!`, drop the single event | Connected | Event lost; non-retained events are best-effort |
| `event_rx` closed | Any | Exit publish_task loop | Terminated | Publish_task returns |

#### 3.10.3 Inventory

| Trigger | State | Action | Result | Observable effect |
|---------|-------|--------|--------|-------------------|
| `DeviceDiscovered` while connected | desired_inventory[k] = any | Insert `Some(data)` + encode w/ session_id + retained publish | Broker has current inventory | Device visible to consumers |
| `DeviceDiscovered` while disconnected | desired_inventory[k] = any | Insert `Some(data)` only | Local tracking updated | No broker publish; reconciled on ConnAck |
| `DeviceLost` while connected | desired_inventory[k] = Some | Insert `None` + empty retained publish | Broker inventory cleared | Device no longer visible |
| `DeviceLost` while disconnected | desired_inventory[k] = Some | Insert `None` only | Local tracking updated | Tombstone sent on ConnAck |
| `DeviceLost` for unknown device | desired_inventory[k] absent | Insert `None` | Tombstone created | Defensive; empty retained on ConnAck clears any stale data |
| Reconnect (ConnAck) | N active, M tombstones | Publish all N as retained + all M as empty retained | Broker state = local state | Full reconcile; `info!` log with counts |
| `republish_all` publish fails for one device | Iterating desired_inventory | `warn!` log with remaining count, stop reconcile immediately | Fail-fast reconcile | All unreconciled entries retried on next ConnAck |
| Graceful shutdown | N active devices | Publish offline status only; inventory unchanged | Broker retains last inventory | Consumers see offline status + stale inventory |
| Crash / SIGKILL | N active devices | LWT publishes offline (ts=0); inventory unchanged | Broker retains last inventory | Consumers see offline (ts=0) + stale inventory |
| Process restart after crash | Empty desired_inventory | Re-discover devices; first ConnAck reconciles | Broker gets fresh inventory | Previously-lost devices have stale retained (known limitation) |

#### 3.10.4 Delivery

| Trigger | Action | Outcome | Recovery |
|---------|--------|---------|----------|
| `client.publish().await` returns `Ok` | Message enqueued to rumqttc channel | May or may not reach broker | For retained ops: replayed on next ConnAck. For transient: no retry. |
| `client.publish().await` times out (5s) | rumqttc internal channel full or eventloop stalled | Message not enqueued | For live events: logged + dropped. For reconcile: warn + skip, retried on next ConnAck. |
| `client.publish().await` returns `Err(ClientError::TrySend)` | rumqttc internal channel closed | EventLoop has been dropped/aborted | Fatal -- process is shutting down. |
| Connection drops after enqueue, before TCP write | eventloop detects on next poll | QoS 1 messages in rumqttc's internal pending queue are lost (clean_session=true, no broker-side session) | Retained ops: reconciled on next ConnAck from `desired_inventory`. Transient: lost (no buffer). |
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
        label="Eventloop Pump Task (NO publish calls)";
        style=dashed;
        EL_Poll [label="poll()"];
        EL_ConnAck [label="ConnAck received\n1. conn_tx.send(Connected)\n2. reset attempt\n(NO publish here)"];
        EL_Err [label="Error received\n1. conn_tx.send(Disconnected)\n2. increment attempt\n3. sleep(backoff)"];
        EL_Ok [label="Other Ok\n(no-op)"];

        EL_Poll -> EL_ConnAck [label="ConnAck"];
        EL_Poll -> EL_Err [label="Err"];
        EL_Poll -> EL_Ok [label="PubAck/PingResp/..."];
        EL_ConnAck -> EL_Poll;
        EL_Err -> EL_Poll;
        EL_Ok -> EL_Poll;
    }

    subgraph cluster_publish {
        label="Publish Task (no outbound buffer)";
        style=dashed;
        PT_Select [label="select! {\n  event_rx.recv()\n  conn_rx.changed()\n}"];
        PT_Event_Connected [label="Connected path:\n1. track_inventory\n2. publish event (5s timeout)"];
        PT_Event_Disconnected [label="Disconnected path:\n1. track_inventory\n2. drop non-retained (warn)"];
        PT_Reconnect [label="ConnAck path:\n1. publish online status (5s timeout)\n2. reconcile all inventory (5s timeout each)"];
        PT_Exit [label="event_rx closed\n-> exit"];

        PT_Select -> PT_Event_Connected [label="event + connected"];
        PT_Select -> PT_Event_Disconnected [label="event + disconnected"];
        PT_Select -> PT_Reconnect [label="conn_rx -> Connected"];
        PT_Select -> PT_Exit [label="recv() = None"];
        PT_Event_Connected -> PT_Select;
        PT_Event_Disconnected -> PT_Select;
        PT_Reconnect -> PT_Select;
    }

    subgraph cluster_shutdown {
        label="Runner Shutdown Sequence\n(triggered by event_rx closure)";
        style=dashed;
        S1 [label="event_rx closed\n(adapter stopped by binary)"];
        S2 [label="Publish offline status\n(5s timeout)"];
        S3 [label="client.disconnect()\n(enqueue)"];
        S4 [label="Eventloop grace (2s)"];
        S5 [label="Abort eventloop"];
        S6 [label="Runner returns Ok(())"];

        S1 -> S2 -> S3 -> S4 -> S5 -> S6;
    }
}
```

### 3.12 Constants Summary

| Constant | Value | Location | Configurable? |
|----------|-------|----------|---------------|
| rumqttc channel capacity | 100 | `mqtt_client.rs` (`AsyncClient::new(opts, 100)`) | No (compile-time) |
| Publish timeout | 5000 ms | `publish_loop.rs` | No (compile-time) |
| Backoff base | 1000 ms | `lib.rs` (`backoff_with_jitter`) | No (compile-time) |
| Backoff max | 30000 ms | `lib.rs` (`backoff_with_jitter`) | No (compile-time) |
| Backoff jitter | +/- 30% | `lib.rs` (`backoff_with_jitter`) | No (compile-time) |
| Graceful shutdown grace period | 2000 ms | `lib.rs` (`run`) | No (compile-time) |
| Keepalive | 30 s (default) | `mqtt_client.rs` | Yes (`MqttConfig.keepalive_secs`) |
| `clean_session` | `true` | rumqttc default | No |
| QoS | `AtLeastOnce` (1) | All publishes | No |
| LWT timestamp | `0` (unknown) | `mqtt_client.rs` | No |
| `CLIENT_ID_WARN_LEN` | 128 characters | `config.rs` | No (compile-time) |

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

Config validation uses a **three-phase approach**. All three phases run before any I/O starts (no MQTT connection, no I2C bus access). No config error can occur after startup begins.

- **Phase 1: TOML parse** (`toml::from_str`) -- fails fast on the first syntax error, type mismatch, or missing required field. serde does not support collecting multiple parse errors. If Phase 1 fails, a single error message is printed and the process exits with code 1.
- **Phase 2: Cross-field validation** (`Config::validate()`) -- runs only after Phase 1 succeeds. This phase CAN collect multiple errors (e.g., `mqtt://` + `ca_path` present AND empty `adapter_id` AND `keepalive_secs = 0`). All errors are collected and printed to stderr before exiting with code 1.
- **Phase 3: Adapter/driver validation** (`rpi_local_adapter::validate(&config)`) -- runs only after Phase 2 succeeds. This phase validates driver-specific constraints that cannot be expressed as cross-field rules (e.g., OPT3001 minimum poll interval of 800ms, MCP9600 minimum conversion time). If any driver-level validation fails, errors are collected and printed to stderr before exiting with code 1.

The process exits with code 1 after printing all errors from whichever phase fails.

**Pre-flight guarantee:** All three validation phases complete before the adapter starts, before the MQTT client connects, and before any hardware I/O occurs. If the process starts its main loop, the config is fully validated. Runtime config errors are impossible by construction (barring external state changes like a TLS certificate expiring or a broker going offline, which are not config errors).

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
| `broker_url` contains a path other than empty or `"/"` | `config error: mqtt.broker_url: must not contain path, query, or fragment components` |
| `broker_url` contains a query string | `config error: mqtt.broker_url: must not contain path, query, or fragment components` |
| `broker_url` contains a fragment | `config error: mqtt.broker_url: must not contain path, query, or fragment components` |

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

**Length warning (not an error):** After deriving or accepting the final `client_id`, if `client_id.len() > 128`, emit `warn!("MQTT client_id exceeds 128 characters ({len}); some brokers may reject this")`. This is a warning, not a validation error -- the process continues normally.

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

**Portability note (MQTT 3.1.1 compliance):** The MQTT 3.1.1 specification (Section 3.1.3.1) states that conforming brokers MUST accept client IDs of 1-23 characters containing only `[0-9a-zA-Z]`. Brokers MAY accept longer IDs and IDs containing other characters (e.g. `%`). The default derived `client_id` (e.g. `iotkit-rpi%2Dlocal%3Adefault`, 29 characters) exceeds the 23-character minimum and contains `%` characters. This works with Mosquitto, HiveMQ, AWS IoT, and other modern brokers that accept extended client IDs. For strict MQTT 3.1.1 compliance with a broker that enforces the 23-character alphanumeric limit, operators MUST set `mqtt.client_id` explicitly to a compliant value.

**Length warning:** If the final `client_id` (derived or explicit) exceeds 128 characters, the binary logs a `warn!` at startup: `"MQTT client_id exceeds 128 characters ({len}); some brokers may reject this"`. There is no hard rejection -- modern brokers handle long IDs, and the warning alerts operators to potential compatibility issues.

### 4.4 Config Path Resolution

The binary accepts an optional `--config <path>` CLI argument parsed as `Option<PathBuf>`.

#### 4.4.1 Explicit Path (`--config` provided)

Use the provided path exactly. Do not canonicalize or search for alternatives.

If the file does not exist or is not readable:
```
error: config file not found: "<path>"
```
Exit with code 1.

#### 4.4.2 Default Search (`--config` omitted)

Try the following paths in order:

1. `./iotkit-rpi-local.toml` (relative to the process working directory)
2. `/etc/iotkit/iotkit-rpi-local.toml`

For each path: attempt to open the file. If it opens successfully, use it and stop searching. If the open fails with `NotFound`, continue to the next candidate. Any other I/O error (permission denied, etc.) is treated as fatal:
```
error: failed to read config file "<path>": <os-error>
```
Exit with code 1.

If neither candidate exists:
```
error: no config file found; tried:
  ./iotkit-rpi-local.toml
  /etc/iotkit/iotkit-rpi-local.toml
hint: use --config <path> to specify a config file explicitly
```
Exit with code 1.

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

    // Reject path, query, and fragment components.
    // MQTT broker URLs should only have scheme, host, and optional port.
    let path = parsed.path();
    if path != "" && path != "/" {
        return Err(ConfigError::InvalidComponents(raw.to_string()));
    }
    if parsed.query().is_some() {
        return Err(ConfigError::InvalidComponents(raw.to_string()));
    }
    if parsed.fragment().is_some() {
        return Err(ConfigError::InvalidComponents(raw.to_string()));
    }

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

Recommended naming convention using systemd template units with **numeric instance names** matching the I2C bus number:

**Unit file:** `/etc/systemd/system/iotkit-rpi-local@.service`

Replace the `ExecStart` and `DeviceAllow` lines with:

```ini
ExecStart=/opt/iotkit/bin/iotkit-rpi-local --config /opt/iotkit/etc/iotkit-rpi-local-%i.toml
DeviceAllow=/dev/i2c-%i rw
```

The `%i` specifier is expanded by systemd to the instance name. Use **numeric instance names** that match the I2C bus number directly: `@1` for `/dev/i2c-1`, `@3` for `/dev/i2c-3`. This ensures `DeviceAllow=/dev/i2c-%i rw` expands correctly to `/dev/i2c-1 rw`, `/dev/i2c-3 rw`.

**Config files:**

```
/opt/iotkit/etc/iotkit-rpi-local-1.toml   # adapter_id = "rpi-local:1", bus_path = "/dev/i2c-1"
/opt/iotkit/etc/iotkit-rpi-local-3.toml   # adapter_id = "rpi-local:3", bus_path = "/dev/i2c-3"
```

**Enable and start:**

```bash
systemctl enable iotkit-rpi-local@1.service
systemctl enable iotkit-rpi-local@3.service
systemctl start  iotkit-rpi-local@1.service
systemctl start  iotkit-rpi-local@3.service
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
- **Status encode/decode round-trip:** `encode_status` -> `decode_status` for online=true, online=false, ts=0 (LWT). Verify returned `(adapter_id, online, ts, session_id)` tuple matches input.
- **Status via decode_event:** Passing a status payload to `decode_event` -> `DecodeError::InvalidPayload`.
- **DeviceConfig encoding:** `encode_event` with `DeviceConfig` -> `EncodeError::UnsupportedEvent`.
- **Unknown fields ignored:** Envelope with extra fields decodes successfully (forward compatibility).
- **ConnectionKind as_str/from_str symmetry:** `ConnectionKind::from_str(k.as_str()) == k` for all variants (including `Other`).
- **ConnectionKind from_str normalization:** `ConnectionKind::from_str("i2c")` returns `I2c` (not `Other("i2c")`). Known strings are always normalized to their typed variants.
- **Inventory payload does NOT equal discovery payload:** `encode_event` for `DeviceDiscovered` produces a payload without `session_id`; `encode_inventory` produces a payload with `session_id`. The two are structurally different.
- **Inventory payload includes session_id:** `encode_inventory` with a session_id -> `decode_inventory` -> `session_id` field matches.
- **Inventory decode returns session_id:** `decode_inventory` returns `(adapter_id, event, session_id, first_seen_at)` tuple; `session_id` matches the encoded value.
- **Inventory decode returns first_seen_at:** `decode_inventory` returns the encoded `first_seen_at` value unchanged.
- **Inventory first_seen_at preserved across reconnect:** On re-discovery of an already-tracked device, `desired_inventory` retains the original `first_seen_at`; reconcile payload includes the original value (not `now_ms()`).
- **Session_id in status payload:** `encode_status` with a session_id -> decode -> `session_id` field matches for online, offline, and LWT variants.

### 6.2 `iotkit-adapter-runner` Tests

- **Adapter task exit -> runner exits:** Adapter drops `event_tx` unexpectedly -> runner publishes offline (with timeout), returns Ok. (Binary decides exit code based on shutdown_initiated.)
- **Disconnect + DeviceDiscovered -> inventory tracking:** Device discovered while disconnected -> `desired_inventory` updated -> on ConnAck, retained publish occurs.
- **Disconnect + DeviceLost -> tombstone:** Device lost while disconnected -> tombstone recorded -> on ConnAck, empty retained publish.
- **Disconnect drops telemetry:** Telemetry events while disconnected -> dropped with warn log, not buffered.
- **Disconnect drops non-retained discovery/loss notifications:** Non-retained copies of discovery/loss events are dropped during disconnect; only retained inventory is tracked.
- **ConnAck -> full inventory republish:** After ConnAck, all entries in `desired_inventory` are published as retained with current `session_id`.
- **Inventory republish re-encodes with fresh ts:** After ConnAck, inventory payloads have `ts` reflecting the reconnect time, not the original discovery time.
- **Session_id consistency:** All retained messages (status + inventory) from the same process share the same `session_id`.
- **Reconcile fail-fast:** If one inventory publish times out during reconcile, reconcile stops immediately (warn log with remaining count); remaining entries are NOT published in this round. Next ConnAck retries everything from scratch.
- **Publish timeout:** A publish that blocks for > 5s is timed out and skipped (warn log).
- **Graceful shutdown sequence:** Binary receives signal -> adapter stopped (closes event_rx) -> runner publishes offline status (with timeout) -> runner returns Ok -> binary exits 0.
- **Offline status timestamp:** Graceful offline status has `ts > 0`. LWT has `ts = 0`. Both include `session_id`.
- **2nd signal -> immediate exit:** Second signal during shutdown -> binary calls `std::process::exit(1)`.
- **publish_task panic -> runner returns Err:** Simulate publish_task panicking -> runner returns Err(RunnerError::PublishTaskFailed).
- **eventloop_task unexpected exit -> runner returns Err:** Simulate eventloop_task returning unexpectedly -> runner returns Err(RunnerError::EventLoopDied).
- **Backoff calculation:** Verify exponential growth with jitter: attempt 0 -> ~1s, attempt 5 -> ~32s (capped at 30s), jitter within +/-30%.
- **Reconnect counter reset:** ConnAck resets attempt counter to 0.
- **Device lost then rediscovered while disconnected:** `desired_inventory` reflects latest state (rediscovered); intermediate tombstone not published.
- **watch channel rapid state change:** eventloop sends Connected then immediately Disconnected -> publish_task sees Disconnected, does not reconcile.

### 6.3 `iotkit-rpi-local` Config Tests

- **Valid TOML parse:** Well-formed config -> `ValidatedConfig` with all fields populated.
- **Empty adapter_id:** -> `config error: adapter_id: must not be empty`.
- **Empty broker_url:** -> `config error: mqtt.broker_url: must not be empty`.
- **Invalid scheme:** `tcp://localhost` -> config error with scheme message.
- **Missing host:** `mqtt://` -> `config error: mqtt.broker_url: host must not be empty`.
- **broker_url with path:** `mqtt://localhost/some/path` -> `config error: mqtt.broker_url: must not contain path, query, or fragment components`.
- **broker_url with query:** `mqtt://localhost?key=val` -> config error about path/query/fragment.
- **broker_url with fragment:** `mqtt://localhost#frag` -> config error about path/query/fragment.
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
- **Long client_id warning:** `client_id` with > 128 characters -> `warn!` log emitted at startup, but config validation succeeds.
- **Default port resolution:** `mqtt://localhost` -> port 1883. `mqtts://localhost` -> port 8883.
- **Phase 1 fail-fast:** Config with missing required field (e.g., `adapter_id` absent) -> single serde parse error, process exits.
- **Phase 2 collect-all-errors:** Config that parses successfully but has multiple cross-field validation errors (e.g., `mqtt://` + `ca_path` AND `keepalive_secs = 0`) -> all validation errors reported in single output.

---

## 7. Out of Scope

The following items are explicitly excluded from Phase 1A:

- **Inbound MQTT commands** -- No subscribe-side logic. The runner is publish-only. Command handling (e.g. `DeviceConfig` writes, actuator control) is deferred to Phase 2 command bridge (#46).
- **Hot reload** -- Config changes require a process restart. There is no file watcher, no SIGHUP handler, and no runtime reconfiguration mechanism.
- **Health API endpoints** -- No HTTP health check endpoint. Liveness is observed via the retained `status` MQTT topic. A future version may expose a local socket for systemd `Type=notify` or an HTTP `/healthz` endpoint.
- **Outbound buffer / local disk buffer** -- Non-retained events are dropped during MQTT disconnect. There is no in-memory buffer or local disk persistence. Retained state (inventory) is tracked in `desired_inventory` and reconciled on every ConnAck. Local disk persistence (e.g. SQLite WAL, append-only log) is deferred due to SD card write amplification concerns on Raspberry Pi.
- **MQTT v5** -- The runner uses MQTT v3.1.1 exclusively. MQTT v5 features (user properties, shared subscriptions, topic aliases, message expiry) are not used. rumqttc supports MQTT v5 but enabling it would require protocol-level testing against all target brokers.
- **Gateway MQTT subscriber** (#45) -- Phase 2.
- **DeviceKey bus identity change** (#33) -- Phase 2. Current `i2c:0x{addr}:{suffix}` format is preserved.
- **Transform layer** (#43, #44) -- Phase 1B.
- **BravePI standalone adapter** (#46) -- Phase 2.
- **Auto-detection** (#35) -- Phase 2.

---

## 8. Design Tradeoffs and Predicted Concerns

### 8.1 No Outbound Buffer — Telemetry Loss During Disconnect

**Concern:** Without a buffer, telemetry during MQTT disconnect is permanently lost.

**Quantitative impact:** 10 sensors x 1Hz x 30-second disconnect = 300 readings lost. For a 100ms disconnect (e.g., brief network glitch), only ~1 reading per sensor is lost.

**Why this is acceptable:**
1. The gateway maintains timeseries history. A 30-second gap in a continuous stream is tolerable for monitoring and alerting use cases.
2. Telemetry is time-series data with diminishing value when stale. A 30-second-old temperature reading is less useful than a fresh one.
3. The adapter has idempotent write semantics on the gateway side (ON CONFLICT DO NOTHING), so duplicate delivery is harmless but stale delivery adds no value.
4. The eliminated complexity is significant: outbound buffer management, replay fairness, drain phase, backpressure coordination, reconcile ordering races.

**If buffer is needed in the future:** Add it as a separate `BufferedPublisher` wrapper around the publish path. The current design's clean separation (track inventory vs. publish immediately) makes this straightforward to add without restructuring the core state machine.

### 8.2 Brief Disconnects (100ms)

**Concern:** What if the disconnect is very brief (100ms)? Is dropping telemetry wasteful?

**Answer:** Still drop. The reconnect reconcile happens on ConnAck regardless. A few telemetry points lost during a 100ms glitch is acceptable. The alternative (buffering even for brief disconnects) reintroduces all the buffer complexity for marginal gain.

### 8.3 watch Channel Rapid State Changes

**Concern:** What if eventloop sends `Connected` then immediately `Disconnected` before publish_task reads?

**Answer:** `watch` delivers the **latest** state. The publish_task sees `Disconnected` and does not reconcile. This is correct behavior — the connection was too brief to be useful. The next stable `Connected` state will trigger reconcile normally.

### 8.4 Shutdown Without Drain Phase

**Concern:** Without a drain phase, what happens to in-flight events at shutdown?

**Shutdown sequence:**
1. Binary stops adapter -> `event_rx` closes.
2. publish_task breaks out of the event loop (no more events to process).
3. Runner publishes offline status with 5-second timeout.
4. Grace period (2s) for eventloop to flush.
5. Return `Ok(())`.

There is nothing to drain because there is no buffer. Events that were already enqueued to rumqttc's internal channel (capacity 100) will be flushed during the grace period if the connection is active. Events that were dropped during disconnect are already lost — no drain would recover them.
