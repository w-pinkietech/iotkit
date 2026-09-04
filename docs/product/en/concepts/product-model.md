---
type: Concept
title: "IoTKit product model"
description: "Defines the complete product scope, component responsibilities, authority flow, deployment choices, and extension boundaries."
language: en
translation_key: concepts.product-model
status: stable
revision: 6
---

# IoTKit product model

Status: Current product concept.

IoTKit is the reusable layer below factory and business applications. It collects
sensor observations, preserves them through failures, lets an operator assign
generic meaning, and exports versioned application-specific messages. It does not
own products, work orders, processes, OEE, business alarms, or a factory hierarchy.

## Components

| Component | Responsibility | Does not own |
|---|---|---|
| Device | Measures or detects a physical condition | IoTKit durability or business meaning |
| Input Adapter | Translates a vendor/protocol-specific input into the generic Edge Node ingest boundary | Storage, retry policy, custody, semantic rules, or external output |
| Authenticated HTTP ingest | Accepts bounded contract-native device envelopes without an in-process Input Adapter | Device firmware, vendor decoding, or business meaning |
| IoTKit Edge Node | Collects, normalizes, durably buffers, and retries records close to devices | Cross-Edge aggregation or business logic |
| Internal MQTT Broker | Transports Edge Node records and control messages | Durable application custody or data authority |
| IoTKit Edge | Accepts raw custody, discovers and activates Edge Node incarnations, stores device/signal descriptor replicas, owns Edge-scoped display/semantic/output settings, serves the Console, and owns durable output delivery | Edge Node device identity/inventory/desired-configuration authority, factory/business masters, or application workflows |
| Output Adapter | Deterministically converts a generic semantic observation into one external application's topic and payload | Broker credentials, retry scheduling, durable outbox state, or semantic evaluation |
| External application | Uses IoTKit observations for its own domain | IoTKit raw custody and Edge Node purge authority |

An IoTKit Edge can manage multiple Edge Nodes. IoTKit itself does not require a
factory concept; an `edge_id` identifies one IoTKit Edge scope. Aggregation across
multiple `edge_id` values belongs to an optional fleet or application layer.

## Data and authority flow

1. A vendor/protocol device reports through an in-process Input Adapter, or a
   contract-native device sends an authenticated HTTP envelope. Both paths converge
   at the Edge Node collector.
2. The Edge Node resolves stable IoTKit identity and stores the observation.
   Only an active Edge Node incarnation and a non-quarantined record that passes
   publication admission receive publication state in the same transaction and are
   published through the internal Broker. Pre-activation readings remain local and
   are never replayed into the custody stream. Quarantined readings have no outbox
   while quarantined; a later release may enqueue them only through the durable
   activation and publication-admission gate defined by the custody contract.
3. IoTKit Edge atomically stores the validated raw record and contiguous cursor in
   its selected authoritative storage profile.
4. Only IoTKit Edge's application-level `accepted-through` transfers custody and
   makes the corresponding Edge Node records purge-eligible. MQTT PUBACK alone does
   not do this.
5. IoTKit Edge evaluates operator-defined generic semantic rules without changing
   the accepted raw record.
6. A selected Output Adapter creates the exact external MQTT topic and payload. Its
   delivery lifecycle is independent of raw custody acceptance.

## Deployment choices

Edge Node uses local SQLite as its durable buffer. IoTKit Edge selects exactly one
authoritative storage profile:

- `embedded`: SQLite on the IoTKit Edge host, suited to standalone and lower-
  concurrency deployments within a measured capacity envelope;
- `postgres`: PostgreSQL, suited to deployments that need a separate database host,
  higher concurrency, or a larger measured capacity envelope.

Both profiles implement the same product contract and are valid for production
inside a verified capacity envelope. They are not feature or reliability tiers;
deployment measurements determine which profile is appropriate.

IoTKit Edge never dual-writes both profiles and never silently falls back to an
empty backend. Moving from SQLite to PostgreSQL is an explicit offline migration
with identity, cursor, and outbox verification.

The Brokers may be co-located with IoTKit Edge or run on separate hosts. Hostname,
network, certificate, and credential provisioning are deployment responsibilities;
the Console selects configured Output Adapters but does not provision Broker
infrastructure.

## Extension boundaries

- Add a sensor or vendor protocol through an Input Adapter and, when useful, a
  reusable driver.
- Or implement the authenticated HTTP ingest contract directly on a capable
  contract-native device.
- Add a destination application through an Output Adapter.
- Keep vendor-specific and application-specific identifiers inside those adapters.
- Add a wire field or record family only through a versioned contract change with
  cross-language conformance fixtures.

BravePI and Pinikiet are the first verified integrations. Neither defines IoTKit's
generic core model.

## Observation model (device-local redesign)

In the redesign of [#232](https://github.com/w-pinkietech/iotkit/issues/232), IoTKit runs as one instance per device, converts sensor input into Observations inside the device, and publishes them to a standard MQTT Broker. The central IoTKit Edge and the per-application Output Adapters described in the sections above are deleted when that redesign completes. This section defines the Observation model that remains, independent of any protocol. Its mapping onto MQTT is in the [MQTT Output Adapter contract v1](../contracts/mqtt-output-adapter-v1.md).

An Observation is one value produced by one processing pipeline inside the device. A pipeline converts Input Adapter output through calibration, thresholding, hysteresis, debounce, and accumulated counting, and is identified by a pipeline-id. One pipeline has one output; several pipelines may read the same input.

The kinds are fixed to three. Business meaning such as production, alarm, or Gantt is not part of IoTKit; the receiving side assigns it.

| kind | value | Unit |
|---|---|---|
| measurement | A numeric value measured at one point in time | Pipeline configuration; not in the payload |
| accumulated-count | A cumulative integer ≥ 0 computed by the pipeline | Treated as a count |
| state | A boolean current state | None |

Continuity, order, and time are expressed by four fields.

- **series**: one continuous generation of the same pipeline output. It does not change on display-name or threshold tuning, Broker configuration, process restart, or reconnect. It changes on structural edits (kind, input, trigger, unit), an explicit reset, importing definitions, or loss of state. A new accumulated-count series starts at `value = 0` and publishes that first value immediately.
- **sequence**: an integer starting at 1 within a series and increasing by 1 per publication. Ordering and de-duplication use it.
- **uptime**: the time elapsed from the boot of the device to the receipt of the input that settled the output. It comes from a monotonic clock, so within one boot the difference between two Observations equals the real elapsed time. It resets on reboot without changing the series. Cycle times and gap lengths are measured with it.
- **unix epoch time**: the wall-clock time at which that input was received. Present only while the device can vouch for its clock (for example after NTP synchronization); otherwise unknown. It stays unknown right after boot on devices without an RTC and on sites without NTP. Used for calendar placement, never for ordering.

Observations are not stored long-term on the device. The device keeps only evaluation state, the current accumulated value or state, the series, the next sequence, and unsent publications; history belongs to the receiving application.

## Configuration ownership (device-local redesign)

After the redesign, the device splits its configuration by whether a change requires a process restart. Settings that require a restart live in a TOML file; settings that can change while the process runs live in SQLite.

| Owner | Items | Takes effect |
|---|---|---|
| TOML | edge-node-id, MQTT Broker connection, database path, status heartbeat interval, pipeline definition export path, Console API bind, Input Adapter instances | Process restart |
| SQLite (edited from the Console) | Pipeline definitions | Immediately |
| SQLite (state) | Evaluation state, accumulated value or state, series, next sequence, unsent publications, hash of the pipeline definitions | — |

The TOML tables are as follows. The edge-node-id is the stable identifier of the device; it is required and has no implicit default such as the hostname. Both edge-node-id and pipeline-id follow the identifier rules in the [MQTT Output Adapter contract v1](../contracts/mqtt-output-adapter-v1.md); a violating value is a startup error.

~~~toml
[edge_node]
id = "rpi1"                       # required; unique within the Broker namespace
db_path = "/var/lib/iotkit/iotkit.db"

[output.mqtt]
enabled = true
host = "mqtt.example"
port = 8883
password_file = "/run/secrets/iotkit-mqtt-password"
trust_mode = "bundle_only"        # system_roots or bundle_only
ca_file = "/etc/iotkit/broker-ca.pem"

[status]
heartbeat_interval = "60s"        # 5s to 1h; default 60s

[pipelines]
export_path = "/var/lib/iotkit/pipelines.toml"  # default: next to the database

[api]
enabled = true
bind = "0.0.0.0:8443"

[adapters.instances.<name>]
# Input Adapter instances; see the Input Adapter contract v1
~~~

`pipelines.export_path` is a backup derived from the database and written after every committed change to the pipeline definitions. It is not read at startup; restoring from it is an explicit import operation.

## Pipeline definition (device-local redesign)

A pipeline is the unit that converts one Input Adapter output into one Observation. One pipeline has one output; several pipelines may read the same input. Definitions live in SQLite and are edited from the Console and `nodectl pipeline` through typed operations (`pipeline.create` / `update` / `delete` / `reset` / `import`). Items are either **structural** (a change starts a new series) or **tuning** (the series continues).

| Item | Class | Content |
|---|---|---|
| `id` | structural | pipeline-id; follows the identifier rules |
| `kind` | structural | `measurement` / `state` / `accumulated-count`. Cannot change (delete and recreate) |
| `input` | structural | `adapter` (Input Adapter instance name), `subject` (optional device identity; any subject when omitted), `measurement_key`, `channel_index` (optional), `value_index` (default 0) |
| `trigger` | structural | Required only for `accumulated-count`. `on-transition` only in the first version |
| `unit` | structural | Required only for `measurement`; forbidden for other kinds |
| `display_name` | tuning | Display name (up to 128 characters) |
| `calibration` | tuning | `scale` (finite and non-zero, default 1.0), `offset` (finite, default 0.0) |
| `detector` | tuning | `mode` (`high-active` / `low-active`), `rise_threshold`, `fall_threshold` (`fall_threshold <= rise_threshold`), `rise_debounce_ms`, `fall_debounce_ms` (0 to 300,000). Forbidden for `measurement`, required for the other kinds |

A series starts by these rules.

- The normalized hash of the structural items is stored with the state and compared with the definition's hash at startup and on every edit. A mismatch, or a missing state row, starts a new series.
- An explicit reset (Console, `nodectl pipeline reset <id>`) and `nodectl pipeline import <file>` start a new series. Import replaces every definition; pipelines absent from the file are treated as deleted.
- A new `accumulated-count` series publishes `sequence = 1, value = 0` inside the transaction that started it.
- Deleting a pipeline publishes a zero-length payload with retain to its topic, clearing the value the Broker holds.

Processing one input writes the evaluation state, the current value, the next sequence, and the outbox row in one SQLite transaction. A failed input is discarded and the evaluation state stays as it was. The number of discarded inputs, the last error, and its time are kept in memory per pipeline and shown in the Console. A pipeline whose count reached 2^53−1 discards further inputs and is shown as an error.

After every committed definition change, all definitions are written atomically to `pipelines.toml`. A failed export does not undo the change; it is shown as an error and retried on the next change.
