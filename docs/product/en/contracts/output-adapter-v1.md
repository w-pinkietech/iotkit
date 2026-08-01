---
type: Contract
title: "IoTKit Output Adapter contract v1"
description: "Defines generic observations, route configuration, transformation, MQTT publications, and application bindings."
language: en
translation_key: contracts.output-adapter-v1
status: stable
revision: 4
---

# IoTKit Output Adapter contract v1

Status: Implemented 2026-07-19.

## 1. Purpose

An Output Adapter is the boundary that converts generic semantic data established by IoTKit Edge into an external-application-specific MQTT contract. Pinikiet is the first implementation, but the generic contract knows none of its topics, purpose names, or payload fields.

```text
stored raw observation
  -> IoTKit generic semantic rule
  -> Generic Output Observation
  -> versioned Output Adapter + route configuration
  -> exact MQTT publication
  -> IoTKit Edge durable outbox / delivery layer
```

A Sensor/Input Adapter brings values from physical equipment into IoTKit. An Output Adapter sends semantic data from IoTKit to an external application. They do not share one adapter lifecycle or configuration shape.

## 2. Contract boundary

An Output Adapter owns:

- a stable Adapter ID and configuration schema version;
- supported combinations of generic Observation kind and external mode;
- syntax, value, and compatibility validation for route configuration;
- deterministic conversion from a Generic Output Observation to MQTT topic and payload;
- one MQTT publication including QoS and retain.

It does not own:

- semantic-rule evaluation, thresholds, debounce, or accumulation;
- Broker endpoint, TLS, certificates, credentials, or client ID;
- MQTT connection, publish, PUBACK, retry, or backoff;
- SQLite, outbox state, delivery state, or audit;
- IoTKit Edge accounts, roles, or Console;
- business masters, processes, production results, or OEE.

The Adapter is a pure in-process transformation. It must not access storage, clock, network, environment, or secrets. The same route configuration and Observation return the same publication byte for byte.

## 3. Adapter identity and capabilities

Every Adapter returns the `Descriptor` defined by
`iotkit-output-adapter-api`. Its current public Rust shape is:

```rust
pub struct Mode {
    pub key: &'static str,
    pub display_name: &'static str,
    pub accepts: &'static [ObservationKind],
}

pub struct Descriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub config_schema_version: u32,
    pub modes: &'static [Mode],
}
```

- `id` is a stable lowercase ASCII ID identifying the external application, transport, and major contract. Initial IDs are `iotkit.mqtt-json.v1` and `pinikiet.mqtt.v1`.
- `config_schema_version` is the exact route configuration JSON version.
- `Mode::key` is an external-application purpose, not a generic kind.
- `Mode::accepts` is the closed set of generic kinds transformable to that mode.
- Mode keys do not repeat inside one Adapter.

The descriptor may populate Console choices, but `validate_config` remains the
final authority for persistence.

## 4. Generic Output Observation

```rust
pub enum ObservationKind {
    Numeric,
    Boolean,
    CumulativeValue,
    Alarm,
}

pub enum ObservationValue {
    Numeric(f64),
    Boolean(bool),
    CumulativeValue(u64),
    Alarm { active: bool, reading: Option<f64> },
}

pub struct Observation {
    observation_id: String,
    series_id: String,
    sequence: u64,
    observed_at: i64,
    value: ObservationValue,
}
```

The fields are private. Authors receive validated values and read them through
`observation_id()`, `series_id()`, `sequence()`, `observed_at()`, `kind()`,
`value()`, and `reading()`. `Observation::new` is the validating constructor.

| Kind | Value | Meaning |
|---|---|---|
| `numeric` | finite JSON number | calibrated or transformed number |
| `boolean` | JSON boolean | generic on/off state |
| `cumulative_value` | non-negative JSON integer | accumulated value since an origin |
| `alarm` | JSON boolean | `true` raised, `false` cleared |

`production`, `onoff`, and `gantt_chart` are application purposes and never become generic kinds. Even if an internal implementation is named `cumulative_counter`, this boundary uses the agreed `cumulative_value`.

`observation_id` and `series_id` are lowercase canonical UUIDs, `sequence` is monotonic and at least 1, and `observed_at` is Unix epoch milliseconds. The Adapter never recreates identity, time, or value. `reading` is an optional finite number captured during alarm evaluation and is never guessed. Edge Node ledger epoch, publication sequence, IoTKit Edge raw-row ID, and custody cursor do not cross this application boundary.

## 5. Versioned route configuration

Configuration is one UTF-8 JSON object with required `schema_version`. An Adapter rejects unknown fields, unknown versions, multiple JSON values, and trailing garbage. It never interprets an old configuration through implicit defaults or field guessing.

```rust
pub trait OutputAdapter: Send + Sync {
    fn descriptor(&self) -> &'static Descriptor;

    fn validate_config(
        &self,
        config: &serde_json::value::RawValue,
        kind: ObservationKind,
    ) -> Result<(), AdapterError>;

    fn transform(
        &self,
        config: &serde_json::value::RawValue,
        observation: &Observation,
    ) -> Result<MqttPublication, AdapterError>;
}

pub trait ProfilePolicy: Send + Sync {
    fn setup(&self) -> &'static ProfileSetup;
    fn identity_policy(&self) -> IdentityPolicy;
    fn propose(
        &self,
        request: &ProfileRequest<'_>,
    ) -> Result<Vec<RouteProposal>, AdapterError>;
}
```

`validate_config` checks together:

- JSON conforms to the selected Adapter schema;
- the requested mode exists;
- source kind and mode are compatible;
- external identities used in topics have a safe closed syntax;
- values satisfy the external contract.

Configuration changes create a future-only route revision and never implicitly backfill old Observations. Stopping a route stops new transformation but does not silently delete already durable outbox publications; existing delivery drains normally. The Console treats route active/stopped separately from MQTT pending/stalled.

Route configuration contains only non-secret transformation settings visible to viewers. Broker credentials, CA, private key, and token belong to deployment connection profiles.

## 6. Registry and route persistence

IoTKit Edge registers built-in Adapters in a compile-time registry. V1 has no runtime plugin discovery. Duplicate or invalid descriptors and unknown Adapter IDs are rejected when a route is created.

An Adapter is a trusted Rust crate implementing `OutputAdapter` and
`ProfilePolicy`. Authors depend on `iotkit-output-adapter-api`, prove behavior
with `iotkit-output-adapter-testkit`, then add the crate to the workspace and
the single static production registry. Provider-specific transformations do not
modify core storage, MQTT, HTTP, or Console code.

Author sources are the
[`iotkit-output-adapter-api`](https://github.com/w-pinkietech/iotkit-next/tree/main/edge/output-adapters/api),
[compile-tested vendor-neutral example](https://github.com/w-pinkietech/iotkit-next/tree/main/edge/output-adapters/example),
and
[shared testkit](https://github.com/w-pinkietech/iotkit-next/tree/main/edge/output-adapters/testkit).
From the repository root, begin with:

```bash
cargo test -p iotkit-output-adapter-example
cargo test -p iotkit-output-adapter-testkit
cargo test -p iotkit-edge --test output_registry
```

```text
route_id
adapter_id
config_schema_version
config_json
route_revision
active_from_observation
stopped_at_observation
```

The configuration version must match the selected descriptor. The Rust schema
is a fresh baseline and does not import routes or outbox rows from the former
implementation.

`output_routes` are execution units expanded from the IoTKit-Edge-wide `export_profile` through `profile_rule_binding`, not settings users create per rule. The profile expander derives exact route configuration from IoTKit Edge ID, versioned Adapter ID, semantic rule ID, external purpose, stable logical signal ID, and rule kind. The Adapter does not know the Edge, rule inventory, or future automatic rule additions.

Console/API operations are:

- `GET /api/v1/export-profiles`: list destinations and binding states;
- `POST /api/v1/export-profiles`: confirm continuous application to current and future rules; generic output begins immediately, while Pinikiet prepares topics and IDs;
- `PUT /api/v1/output-bindings/{binding_id}`: decide a Pinikiet boolean purpose and prepare topic/ID;
- `POST /api/v1/output-bindings/{binding_id}/start`: after external topic registration, start only beyond the saved boundary;
- `POST /api/v1/export-profiles/{profile_id}/stop`: stop new transformation at a boundary and drain existing delivery;
- `GET /api/v1/output-bindings/{binding_id}/publication`: inspect the exact topic and payload.

Diagnostic reads remain at `GET /api/v1/output-adapters` and `GET /api/v1/output-routes`. There is no API or Console operation for arbitrary individual routes, topics, source IDs, or signal IDs. Handlers never write SQL directly.

## 7. Transformation and errors

`transform` revalidates Observation and configuration, then returns exactly one
`MqttPublication`. Multiple destinations require multiple routes; v1 has no
Adapter-internal fan-out.

The public Rust error variants are:

```rust
pub enum AdapterError {
    InvalidDescriptor,
    InvalidConfiguration,
    InvalidObservation,
    UnsupportedObservation,
    InvalidPublication,
    TransformFailed,
}
```

They are written as `AdapterError::InvalidDescriptor`,
`AdapterError::InvalidConfiguration`, `AdapterError::InvalidObservation`,
`AdapterError::UnsupportedObservation`, `AdapterError::InvalidPublication`,
and `AdapterError::TransformFailed`. A pure transform has no temporary
network-error class.

On transform failure, IoTKit Edge does not enqueue an invalid message, delete the source Observation, mark it delivered, or guess another mode. It exposes the route as requiring action. It durably stores only a closed `last_transform_error_code` and timestamp: `adapter_unavailable`, `config_version_mismatch`, `invalid_observation`, or `transform_failed`. It does not copy configuration JSON, payload, credentials, or internal error strings into diagnostics.

Failure on one route does not stop another route in the same batch. The failed Observation gets no outbox row. Candidates stay oldest-first per route and interleave one at a time from the route least recently attempted. An errored route retries only its oldest untransformed Observation and stops later candidates in that batch, preventing starvation and ensuring later success cannot hide an older failure. The error clears when that oldest Observation transforms successfully.

The Console shows transformation state separately from MQTT state derived from `pending` and `published_at`. A short pending period is “delivering”; only a route whose oldest publication remains pending for five minutes becomes “possible delivery stall,” with last delivery time and count.

## 8. MQTT publication

```rust
pub struct MqttPublication {
    topic: String,
    qos: u8,
    retain: bool,
    payload: Box<serde_json::value::RawValue>,
}
```

The fields are private. `MqttPublication::new(topic, qos, retain, payload)`
validates them, and authors read an accepted publication through `topic()`,
`qos()`, `retain()`, and `payload()`.

V1 requires a non-empty exact UTF-8 topic without NUL, `+`, or `#`; exact QoS 1; and valid JSON payload. The Adapter never publishes. The delivery layer durably stores the publication in SQLite before sending it and retries the same topic/payload until PUBACK. The external contract selects retain; ordinary Observations are normally false, while a separate source-status contract may be true.

Shared fixtures live in `testdata/output/v1/` and fix Adapter ID, versioned configuration, generic Observation, expected topic, QoS, retain, and payload together. `scripts/test-edge-output.sh` verifies generic and Pinikiet routes through real Mosquitto, persistence while the Broker is down, and convergence after restart. A consumer contract gate will be added when the Pinikiet repository provides its decoder and matching shared fixture.

## 9. IoTKit MQTT JSON v1 binding

`iotkit.mqtt-json.v1` accepts all four kinds without changing their meaning. Users do not enter a topic. `source_id` is `edge_meta.edge_id`; a cryptographically random `signal_id` is issued once for `(versioned adapter_id, semantic rule_id, mode)`. The profile expander generates:

```text
iotkit/v1/sources/{source_id}/signals/{signal_id}/observations
```

Stopping and re-adding the same Adapter/rule/mode creates a new future-only binding but reuses `signal_id`. Different modes or Adapter versions receive different IDs. `series_id` identifies the semantic series and `observation_id` identifies each Observation, so binding lifecycle is not encoded by changing the signal ID.

The closed internal execution configuration is:

```json
{"schema_version":1,"topic":"iotkit/v1/sources/.../signals/.../observations"}
```

The topic is complete and at most 65,535 bytes. Templates, placeholders, and sensor-name expansion are not supported. Payload is:

```json
{
  "schema_version": 1,
  "observation_id": "00000000-0000-0000-0000-000000000000",
  "series_id": "00000000-0000-0000-0000-000000000000",
  "sequence": 1,
  "observed_at": 1720000000000,
  "kind": "numeric",
  "value": 21.5
}
```

A finite `reading` is added only when present on an alarm Observation. The Adapter does not reinterpret kind, value, identity, or time. QoS is 1 and retain is false. A company-specific system unable to consume this common contract uses a Connector outside IoTKit; v1 ships no Connector runtime, implementation, or SDK.

## 10. Pinikiet MQTT v1 binding

| Generic kind | Pinikiet mode |
|---|---|
| `cumulative_value` | `production` |
| `boolean` | `onoff` |
| `boolean` | `gantt_chart` |
| `alarm` | `alarm` |

It never guesses a Pinikiet purpose for `numeric`, and v1 has no string Observation, so it exposes no `barcode` mode. Configuration has `schema_version=1`, `source_id`, `sensor_id`, `kind`, and optional `reason`, and converts to the agreed Pinikiet MQTT Purpose-Bound Signal Contract v1.

IoTKit Edge issues `sensor_id` once per `signal_ref` as `sen-<128-bit lowercase hex>`. The `production`, `alarm`, `onoff`, and `gantt_chart` values derived from the same sensor share that ID and the exact topic `pinikiet/v1/sources/{source_id}/sensors/{sensor_id}/observations`.

Each semantic rule owns a distinct `series_id`, and `sequence` increases from 1 within that series. Kinds do not share a global sequence; `(series_id, sequence)` is the deduplication identity. A meaning-changing rule update starts a new series at sequence 1 while preserving `sensor_id` and the topic.

In Pinikiet, the topic is also an input-registration contract. Adding a profile or selecting a boolean purpose prepares but does not publish. The Console/API shows one exact topic per sensor and an example payload. After an installer registers that topic once, an explicit start operation saves an accepted cursor boundary and activates every prepared kind for that sensor. Compatible rules added later reuse the registered topic and start automatically. Observations before each rule's start boundary are never sent later.

Pinikiet source status is a separate source-level publication, not a semantic Observation route. Production bootstrap issues `edge-<32hex>` before DB creation and gives the same ID to IoTKit Edge and Broker ACL. `iotkit-edge-output-<edge-id>` may write only that source's IoTKit/Pinikiet observation and status namespaces. Starting an existing DB with a different `--edge-id` is rejected.

## 11. V1 exclusions

- Runtime shared-library plugin ABI or Console-installed binary;
- external-process, WASM, or gRPC Adapter;
- Connector implementation, SDK, or runtime;
- HTTP/Webhook output or per-Adapter Broker credentials;
- camera streams or numeric disguise for barcodes;
- calling delivery a durable application receipt when the external contract has no application acknowledgement.

A new external service first implements this in-process contract and adds contract tests. If third-party distribution or another transport becomes necessary, it is designed as a separate major contract covering security, resource limits, upgrade, and isolation.
