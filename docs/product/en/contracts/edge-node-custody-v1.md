---
type: Contract
title: "Edge Node custody contract v1"
description: "Defines the complete MQTT custody transfer, activation, record families, acknowledgement, retry, and authentication contract."
language: en
translation_key: contracts.edge-node-custody-v1
status: stable
revision: 5
---

# Edge Node custody contract v1 (R10 exit)

Status: Approved MQTT v1 target contract. The records/descriptors/accepted-through custody path is
implemented, including Edge Node activation and publication admission. The older HTTPS
publisher is retained only as transitional code and is not started by the Edge Node composition root.

This contract defines how canonical records leave one Edge Node and when that Edge Node may transfer
custody. Application meaning such as production, OEE, process, alarm text, or Pinikiet state is not
part of this contract.

## Roles

- **Edge Node publisher** reads the durable outbox, publishes bounded batches, retries, and owns the
  local delivery cursor.
- **MQTT Broker** transports QoS 1 messages. Its PUBACK confirms Broker receipt only.
- **IoTKit Edge** durably accepts canonical records, advances the contiguous accepted-through
  cursor, then publishes the application custody acknowledgement. It also provides direct raw query
  today and is the Edge-scoped semantic and application export boundary. Semantic projection
  and exporter failure never weaken or roll back raw custody acceptance.
- **Application consumer** such as Pinikiet receives a separately mapped Output Adapter contract.
  It does not consume this raw custody stream, and its business result does not authorize Edge Node purge.

## Topics

```text
iotkit/v1/edge-nodes/{edge_node_id}/records
iotkit/v1/edge-nodes/{edge_node_id}/accepted-through
iotkit/v1/edge-nodes/{edge_node_id}/descriptors
iotkit/v1/edge-nodes/{edge_node_id}/activation/request
iotkit/v1/edge-nodes/{edge_node_id}/activation/result
```

`records` and `accepted-through` use QoS 1 and MUST NOT be retained. `descriptors` uses QoS 1 and
MUST be retained; it is a complete current-state replica, not a custody stream. `activation/request`
and `activation/result` use QoS 1 and MUST NOT be retained. IoTKit Edge durably retries an activation
request until the correlated application result is committed; MQTT PUBACK is never activation
completion. ACLs restrict each Edge Node to publishing its own records/descriptors/activation result
and subscribing to its own acknowledgement/activation request. Application-specific topics are
outside R10.

## Edge Node activation and publication admission

Broker enrollment and Edge Node activation are separate.

- **Broker enrollment** gives an Edge Node its connection profile, static credential, and exact
  topic ACL. It is an installation operation on the Broker and Edge Node hosts.
- **Edge Node activation** is an authenticated IoTKit Console operation that authorizes one exact
  `(edge_node_id, ledger_epoch)` incarnation to begin a new IoTKit Edge custody stream.

A Broker-enrolled but inactive Edge Node publishes its descriptor and receives activation requests. It
MUST NOT publish records. It may durably keep normalized pre-activation readings for local
commissioning preview, but MUST NOT assign them a `pub_seq` or insert any record family into the
publication outbox. Those readings are not R10 canonical publication records and are never eligible
for later replay to IoTKit Edge.

An administrator activation transaction durably records `edge_id`, a unique `activation_id`, the
exact Edge Node and ledger epoch, actor audit, and a retryable command outbox before publishing:

```json
{
  "schema_version": 1,
  "activation_id": "act-0123456789abcdef0123456789abcdef",
  "edge_id": "edge-0123456789abcdef0123456789abcdef",
  "edge_node_id": "edge-node-01",
  "expected_ledger_epoch": "01J...",
  "grant_revision": 1,
  "issued_at": 1720000000000
}
```

The Edge Node applies a request in the same SQLite write serialization used by collection. It validates
the exact identity and epoch, verifies that the publication log and its allocation sequence have
never been used, freezes the pre-activation `readings.seq` boundary once, persists the activation
receipt, and opens publication admission for future transactions. Every publication enqueue path,
including measurement, annotation, epoch start, commissioning smoke, and later quarantine release,
MUST use this durable admission gate. Replaying the same activation ID returns the same result and
never recomputes the boundary. A different activation ID for an active Edge Node is rejected.

```json
{
  "schema_version": 1,
  "activation_id": "act-0123456789abcdef0123456789abcdef",
  "edge_id": "edge-0123456789abcdef0123456789abcdef",
  "edge_node_id": "edge-node-01",
  "ledger_epoch": "01J...",
  "status": "applied",
  "discard_through_reading_seq": 842,
  "first_publication_seq": 1,
  "applied_at": 1720000001000
}
```

The frozen prefix becomes immediately ineligible for normal query and publication. Physical row
deletion is restartable, bounded Edge Node-local cleanup and is not an activation completion condition.
It never changes the boundary. `accepted-through` remains the only authority to advance or purge
the post-activation official publication outbox.

IoTKit Edge states are `discovered`, `activating`, `active`, and `recovery_hold`. It accepts records only
after committing the matching activation result and entering `active`. Activation state validation,
exact epoch validation, raw insertion, fingerprint verification, and cursor advance belong to the
same custody transaction. A record received before activation completion is stored nowhere and
receives no acknowledgement; normal Edge Node replay converges after IoTKit Edge becomes active.

## Descriptor snapshot

The descriptor topic carries schema version 2 complete snapshots of Edge Node-owned device and signal
metadata. Other descriptor schema versions are rejected; there is no pre-release schema 1
compatibility path. Schema version 2 includes the optional device-level `model_id`. Edge Node publishes
after every MQTT connection and whenever the persisted descriptor revision changes. The encoded
snapshot is limited to 1 MiB and is rejected rather than truncated.

`model_id` is an opaque, stable software catalog identifier for an explicitly persisted device
model. It is not a display label, device identity component, or semantic classification. It is
absent for unknown and non-modelled devices. When present it is 1–64 ASCII bytes matching
`[a-z][a-z0-9]*(?:[-_.][a-z0-9]+)*`. IoTKit Edge may display it but MUST NOT branch semantic mapping,
grouping, or authorization on its value.

The snapshot also contains stable `system_id`/`series_key`, an optional non-authoritative display
identifier, device state, measurement key, channel, variant, canonical unit, and value type. It
never contains hardware/provider identifiers, adapter type or instance identifiers, physical
locators, configured sources, credentials, or adapter payloads. IoTKit Edge validates the composite
series identity and durably replicates the snapshot. Lower revisions in one ledger epoch are
ignored; equal revisions with different content are conflicts. Persisted model binding changes
advance `descriptor_revision`, so different model content is never published under the same
revision.

### Identity across restart and replacement

Physical sameness is not an identity claim. The descriptor deliberately omits hardware/provider
IDs and physical locators, so IoTKit Edge never guesses that two signals are the same.

| Situation | Edge Node-owned identity | IoTKit Edge result |
| --- | --- | --- |
| Process or host restart with the same database | The same `edge_node_id`, `system_id`, and `series_key` continue. | Existing `device_ref`, `signal_ref`, profiles, rules, output bindings, and history continue. No sensor reconfiguration is required. |
| Authorized recovery from an authenticated Edge Node backup | The restored `edge_node_id`, `system_id`, and `series_key` continue under the permitted new ledger epoch. | Existing refs and Edge-owned settings continue after recovery activation. The recovery contract, not physical matching, authorizes continuity. |
| Clean replacement without a usable backup, or after the operator abandons a failed recovery | A fresh database MUST use a new `edge_node_id` and creates new ledger-owned device and series identities. | The descriptor creates new `device_ref` and `signal_ref` inventory with no inherited profile, rule, calibration, or output binding. The old inventory, settings, and history remain attached to the old identity. |
| Confirmed identity-bearing device hardware replacement while the Edge Node ledger survives | The typed replacement operation preserves `system_id` and its existing series identities while changing only the hardware binding. | Existing refs, settings, and history continue. This is not Edge Node host recovery. |

A clean replacement may attach the same physical sensor and report the same measurement type; it
is still a new signal. IoTKit Edge does not merge history or copy configuration automatically.
Copying selected settings, if later supported, is a separate explicit operation and never an
identity merge. Continuation of IoTKit Edge-owned refs and settings assumes its authoritative
database remains available or is separately restored through the IoTKit Edge recovery procedure.

A descriptor may discover an inactive Edge Node but never activates it. A descriptor failure never
authorizes purge, changes publication admission, or suppresses `accepted-through` for an already
active Edge Node. Edge Node and IoTKit Edge are developed and deployed together against schema version 2; incompatible
pre-release databases and retained descriptors are recreated instead of carrying compatibility
code.

## Record batch

```json
{
  "schema_version": 1,
  "edge_node_id": "edge-node-01",
  "ledger_epoch": "01J...",
  "publication_id": "edge-node-01:01J...:123:130",
  "cursor_start": 123,
  "cursor_end": 130,
  "records": []
}
```

Requirements:

- `cursor_start..cursor_end` is a non-empty contiguous publication range.
- The first and last record `pub_seq` match the range, with no gaps or duplicates inside the batch.
- Retry preserves the same publication ID, range, and record content.
- Global record identity is `(edge_node_id, ledger_epoch, pub_seq)`.
- Event time may be late or non-monotonic and is never a delivery cursor.
- Version 1 limits a batch to 256 records and 1 MiB encoded size.
- The initial publisher permits one application-unacknowledged batch at a time.
- A newly activated stream starts at `pub_seq=1`; there is no implicit or fabricated prefix.

`publication_id` is a deterministic correlation and replay identity. IoTKit Edge stores a fingerprint of
the received record content. Receiving different content for an existing global record identity is
a custody conflict; it is never last-write-wins.

## Record families

Version 1 accepts exactly `measurement`, `annotation`, and `commissioning_smoke`. Each family is
strict: missing required fields, unknown fields, unknown enum values, or an unknown family reject the
complete batch before raw storage or cursor advance. Adding a field or family requires a versioned
contract change.

### Measurement

```json
{
  "family": "measurement",
  "schema_version": 1,
  "epoch": "01J...",
  "pub_seq": 123,
  "series_key": "opaque-stable-series-key",
  "values": [21.5],
  "event_time": 1720000000000,
  "event_time_source": "device",
  "time_source": "device_ntp",
  "time_quality": "synced",
  "received_at": 1720000000123,
  "device_time": 1720000000000
}
```

`device_time` is required but may be null. `values` is non-empty and all numeric values are finite.
`time_source` is one of `device_ntp`, `device_rtc`, `edge_node`, or `edge_node_adjusted`.
`time_quality` is one of `synced`, `holdover`, or `unsynced`. `event_time_source` is one of
`device`, `edge_node_adjusted`, or `received_at` and must agree with the selected timestamp:

- `device`: `time_source` is `device_ntp` or `device_rtc`, `device_time` is present, and
  `event_time == device_time`.
- `edge_node_adjusted`: `time_source` is `edge_node_adjusted`, `device_time` is present, and
  `event_time == device_time`.
- `received_at`: `event_time == received_at`.

`series_key` is non-empty and opaque to consumers. Pinikiet or another application receives a
separate mapped Output Adapter contract rather than assigning business meaning to this raw record.

### Annotation

Version 1 retains exactly the existing `epoch_start` annotation:

```json
{
  "family": "annotation",
  "schema_version": 1,
  "epoch": "01J...",
  "pub_seq": 130,
  "subtype": "epoch_start",
  "prior_epoch": "01H..."
}
```

`prior_epoch` is required and non-empty. Annotation records share the same publication sequence as
measurements and therefore participate in the same contiguous cursor.

Additional record families require a versioned contract change. They are not added merely to mirror
an application table or MQTT topic.

### Commissioning smoke (optional)

```json
{
  "family": "commissioning_smoke",
  "schema_version": 1,
  "epoch": "01J...",
  "pub_seq": 131,
  "test_id": "smoke-0123456789abcdef0123456789abcdef"
}
```

This optional family proves the normal Edge Node outbox, MQTT, IoTKit Edge durable raw storage, and
`accepted-through` path without claiming that a physical sensor produced a measurement.
`test_id` is a freshly generated 128-bit lowercase hexadecimal identifier prefixed with `smoke-`.
The record bypasses device registration, measurement-registry, quarantine, and semantic projection;
it is never a sensor value or an application event. Consumers that do not perform commissioning may
ignore this optional family. It uses the same publication sequence and acknowledgement contract as
every other raw record and has no special topic or acknowledgement.

## Application custody acknowledgement

After storing a batch, IoTKit Edge publishes:

```json
{
  "schema_version": 1,
  "edge_node_id": "edge-node-01",
  "ledger_epoch": "01J...",
  "publication_id": "edge-node-01:01J...:123:130",
  "accepted_through": 130
}
```

For a valid batch, IoTKit Edge performs one custody transaction:

1. Authenticate the Edge Node topic and validate active IoTKit Edge admission, version, identity, exact
   activated epoch, bounds, and contiguous range.
2. Insert or idempotently verify every raw canonical record.
3. Advance that Edge Node and epoch's contiguous accepted-through cursor.
4. Commit the raw records, fingerprints, and cursor atomically in the selected authoritative store
   (`embedded` SQLite or `postgres`).
5. Only after commit, publish the correlated accepted-through message.

Storage failure, ENOSPC, corruption, cancellation before commit, a gap, or a content conflict MUST NOT
produce an accepted-through acknowledgement. A lost acknowledgement causes a harmless exact replay.

Edge Node validates schema version, topic/body Edge Node identity, epoch, publication ID, monotonicity,
and that `accepted_through` does not exceed the published batch. Only then may it advance its target
cursor. MQTT PUBACK never advances this cursor and never authorizes retention purge.

The shared machine conformance cases at repository path
`testdata/egress/v1/record-family-cases.json` must produce the same accept/reject result in the
Rust Edge Node publisher and Rust IoTKit Edge decoder.

## Retry and outage behavior

- Edge Node outbox is the retry authority and remains durable until application acknowledgement.
- Edge Node activation-command outbox and Edge Node activation receipt are the retry authorities for
  activation. Broker session state is not.
- Broker receipt may release MQTT protocol inflight state but not the application sending window.
- While inactive, Edge Node continues bounded local commissioning collection without creating an R10
  publication backlog.
- If IoTKit Edge or the network is down, Edge Node continues local collection and retains unacknowledged rows.
- On reconnect, Edge Node republishes the same batch until IoTKit Edge confirms the contiguous cursor.
- IoTKit Edge exact replay verifies existing rows and republishes the already committed watermark.

## Authentication

The first implementation uses MQTT over TLS on an operator-provided IP path, anonymous access
disabled, and one static credential plus topic ACL per Edge Node. The path may be a local network, VPN,
private routed network, or another deployment-specific route; IoTKit requires no VPN product.
Secrets are stored outside Git and never appear in argv, logs, Debug output, audit detail, or query
output. D10 owns later authentication hardening.

Edge Node configuration names the Broker and a credential file; the MQTT username is always the
Edge Node's generated `edge_node_id`:

```toml
[exit.mqtt]
enabled = true
host = "mqtt.edge.example"
port = 8883
password_file = "/run/secrets/iotkit-mqtt-password"
# ca_file = "/etc/iotkit/broker-ca.pem" # optional custom CA; otherwise system roots
```

Plain MQTT requires `allow_insecure = true` and is only for local Docker testing.

## Pinikiet boundary

IoTKit publishes canonical observations. IoTKit Edge is the Edge-scoped boundary that maps
stored series to configured sensor meanings and outputs such as `production`. That mapping does not
enter R10, and downstream business success is not a custody ack. Pinikiet consumes the mapped signal
and owns business masters and logic such as products, processes, production records, OEE, alarms,
UI, and notifications.

## Deferred

Only operator-authorized hardware recovery from an encrypted backup may reactivate the
same Edge Node ID under a new ledger epoch after fencing the old credential generation,
as defined by the [Edge Node recovery contract](edge-node-recovery-v1.md). This is not
normal activation, transfer between Edge instances, or automatic clone adoption.

Outside that exact recovery case, the following are deferred: deactivation/reactivation,
IoTKit Edge transfer, Edge Node ID reuse, clone detection, automatic adoption of an
existing standalone outbox, same-epoch `stream_start_after`, terminal/gap repair
protocol, multi-Edge Node fleet operations, generic Broker fan-out, legacy HTTPS
migration, and alternative egress bindings. Ambiguous legacy or restored state enters
`recovery_hold`; it is never auto-activated or remotely cleaned.
