---
type: Contract
title: "IoTKit MQTT Output Adapter contract v1"
description: "Defines the topics, payloads, delivery, and consumer obligations when an IoTKit device publishes Observations and status to a standard MQTT Broker."
language: en
translation_key: contracts.mqtt-output-adapter-v1
status: draft
revision: 1
---

# IoTKit MQTT Output Adapter contract v1

Status: contract fixed. Publishing from the IoTKit process is implemented in the child issues of [#232](https://github.com/w-pinkietech/iotkit/issues/232).

## 1. Purpose

An IoTKit device converts sensor input into Observations through calibration, thresholding, hysteresis, debounce, and accumulated counting, and publishes them to a standard MQTT Broker with the topics and payloads in this contract.
Business applications such as Pinkiet subscribe to the Broker and map Observations into their own domain.

```text
sensor -> Input Adapter -> pipeline -> MQTT Output Adapter v1 -> MQTT Broker -> consumer
         |<-------------------- IoTKit (one instance per device) ------------------------>|
```

This is a per-protocol contract, not a per-application one.
Business vocabulary such as production, alarm, or Gantt appears in neither topic nor payload.
The Observation model itself (kind, series, sequence, timestamp) lives in the [product model](../concepts/product-model.md); this contract maps it onto MQTT.

## 2. Identifiers

edge-node-id and pipeline-id match the following regular expression and are 1 to 64 bytes of UTF-8.

```text
^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$
```

- edge-node-id identifies one IoTKit device and is given in the start-up configuration. It is unique within one Broker namespace.
- pipeline-id identifies one processing pipeline inside the device. It is a configuration identifier, not a physical sensor identifier.

In the first version one Broker namespace is assigned to one consumer-side site. Topics carry no enterprise, site, or factory identifier.

## 3. Observation topic

```text
iotkit/v1/edge-node/{edge-node-id}/observation/{pipeline-id}/{kind-key}
```

kind-key is one of `measurement`, `accumulated-count`, or `state` and equals the pipeline's kind.
One pipeline has one topic.

Examples:

```text
iotkit/v1/edge-node/rpi1/observation/press-01-temperature/measurement
iotkit/v1/edge-node/rpi1/observation/press-01-cycle-count/accumulated-count
iotkit/v1/edge-node/rpi1/observation/press-01-temperature-high/state
```

Consumers subscribe to `iotkit/v1/edge-node/+/observation/+/+` and read edge-node-id, pipeline-id, and kind from the topic segments.

## 4. Observation payload

```json
{
  "series_id": "018f5f83-7a2b-7c61-a729-6af238f558e0",
  "sequence": 42,
  "timestamp": 1784190000123,
  "value": 1250
}
```

| Field | Type | Meaning |
|---|---|---|
| `series_id` | string (1 to 64 bytes of UTF-8) | One continuous generation of the same pipeline output. An opaque string compared only for equality; consumers do not validate it as a UUID or check a version |
| `sequence` | integer (1 to 2^53−1) | Starts at 1 within a series and increases by 1 per publication. Retransmissions reuse the value |
| `timestamp` | integer (Unix epoch ms) | The real time at which IoTKit received the input that settled this output. For a debounced transition, the receipt time of the input that completed the debounce |
| `value` | by kind | See below |

| kind-key | `value` | Notes |
|---|---|---|
| `measurement` | JSON number | Integer or fraction. The unit is not in the payload; it is pipeline configuration and consumer registration |
| `accumulated-count` | integer ≥ 0 (≤ 2^53−1) | The count computed by the pipeline. Monotonic within a series |
| `state` | boolean | The current state after thresholding, hysteresis, and debounce |

These four fields are the whole payload; no others are added.
IoTKit sends the payload as compact JSON with keys in this order (`{"series_id":…,"sequence":…,"timestamp":…,"value":…}`). Producer conformance compares these bytes with the fixtures.
Consumers parse the payload as JSON and must not depend on key order or whitespace.

`sequence` is monotonic; `timestamp` can move backwards after a clock correction on the device. Ordering and de-duplication use `sequence`.
Devices require NTP synchronization.

## 5. Series rules

- Changing a display name, a tuning field such as a threshold or debounce, the MQTT Broker configuration, a normal process restart, or an MQTT reconnect does not change the series.
- Changing a structural field of the pipeline (kind, input, trigger, unit), an explicit reset of the accumulated counter, importing pipeline definitions, loss of SQLite state, or any other break in continuity starts a new series.
- A new `accumulated-count` series publishes `sequence = 1, value = 0` under the new series_id immediately at its start. The value the Broker retains therefore always represents the current series, and a consumer does not miss the first increment.
- `state` and `measurement` publish their initial value on the first input.
- When an `accumulated-count` reaches 2^53−1, IoTKit stops counting and shows a device error. There is no automatic rollover.

## 6. Delivery and responsibility boundary

- MQTT 3.1.1 with `clean_session = true`. IoTKit's outbox is the single source of truth; after a reconnect it resends from the outbox.
- Every Observation is published with QoS 1 and retain. Retain is not history; it makes the Broker hold the latest value per topic.
- One publication is in flight at a time and the outbox is sent in insertion order, so a duplicate can only be a retransmission of the immediately preceding publication.
- PUBACK is the boundary of IoTKit's delivery responsibility. After PUBACK, IoTKit deletes the publication from its outbox. Storage or business processing on the consumer side is not guaranteed. There is no application-level ACK topic.
- Immediately after a reconnect IoTKit publishes status first, then resends the outbox.

Publications made while the Broker is down stay in IoTKit's outbox and arrive with the same topic and payload after recovery.
Input that IoTKit cannot persist is discarded and status becomes `degraded` (section 7).

## 7. Status topic

```text
iotkit/v1/edge-node/{edge-node-id}/status
```

Payload:

```json
{
  "timestamp": 1784190000123,
  "value": "online"
}
```

`value` has three values.

| value | Meaning | Effect on Observations |
|---|---|---|
| `online` | Connected to MQTT and persisting input | None |
| `degraded` | Connected to MQTT and still sending the persisted outbox, but new input cannot be persisted and is discarded | Increments of `accumulated-count` in this interval are lost and never arrive. Consumers record the interval as a gap |
| `offline` | Not connected to MQTT | Observations in this interval are preserved in the outbox and arrive after recovery |

- A heartbeat is the periodic `online` or `degraded`. IoTKit publishes it immediately after connecting and then every `heartbeat_interval` (default 60 s, 5 s to 1 h) with QoS 1 and retain.
- A change between `online` and `degraded` is published immediately, without waiting for the interval.
- At graceful shutdown IoTKit publishes `offline` with a timestamp, QoS 1 and retain, waits up to 2 s for PUBACK, and disconnects.
- On abnormal disconnect the Broker publishes IoTKit's Will (QoS 1, retain). `timestamp` is `null` because IoTKit did not observe the disconnect time.

```json
{
  "timestamp": null,
  "value": "offline"
}
```

## 8. Pipeline deletion

When a pipeline is deleted, IoTKit publishes a zero-length payload with retain to its Observation topic.
The Broker clears the retained value, and consumers that subscribe later receive nothing.
Consumers already subscribed receive the zero-length payload. It is not malformed JSON; it is the settled fact that this input is no longer available.

## 9. Consumer obligations and assumptions

- Keep the highest received `sequence` per `series_id` and discard anything at or below it as a duplicate. When `series_id` changes, drop the stored maximum and accept the new series. A series change is a baseline update, not an anomaly.
- A gap in `sequence` is not an anomaly. Retain keeps only the latest value, so intermediate values published while the consumer was disconnected do not arrive. `accumulated-count` is cumulative, so the latest value is enough to catch up. A consumer that needs every Observation uses a persistent session with the Broker.
- Treat the retained latest value delivered right after subscribing as the initial value. The first `accumulated-count` received is a baseline; do not add its total to business results.
- Treat a decrease of `accumulated-count` within one series as an anomaly; do not silently replace the baseline.
- Treat a zero-length payload as pipeline deletion (section 8).
- Record `degraded` intervals as gaps and distinguish them from `offline` (section 7).
- The difference between a heartbeat's `timestamp` and its receipt time reveals device clock drift. Do not discard Observations on time difference alone.

## 10. Contract artifacts

| Artifact | Path |
|---|---|
| Observation payload JSON Schema | `testdata/observation/v1/observation.schema.json` |
| Status payload JSON Schema | `testdata/observation/v1/status.schema.json` |
| Fixture format | `testdata/observation/v1/fixture.schema.json` |
| Publication fixtures (topic, QoS, retain, payload bytes) | `testdata/observation/v1/*.json` |
| Cases that must be rejected | `testdata/observation/v1/invalid/*.json` |
| Schema and canonical-form check | `node scripts/check-observation-fixtures.mjs` |
| Consumer-side check (publish to a Broker, verify at the subscriber) | `scripts/test-observation-consumer.sh` |

The fixtures are the reference for both producer and consumer. IoTKit's conformance test compares the topics and bytes it publishes with them; consumer conformance tests and simulators generate payloads from them.
A disagreement between document, schema, fixtures, or checks is a contract defect; none is silently adjusted to another.

## 11. Open items

- The publish rate of `measurement` (every input, on change only, or a minimum interval). Decided in a #232 child issue, followed by a revision bump of this contract.
- Whether the zero-length deletion payload goes through the outbox so that it is guaranteed to arrive when the pipeline is deleted while the Broker is unreachable. Also decided in a child issue.
