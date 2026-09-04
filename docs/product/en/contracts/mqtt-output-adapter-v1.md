---
type: Contract
title: "IoTKit MQTT Output Adapter contract v1"
description: "Defines the topics, payloads, delivery, and consumer obligations when an IoTKit device publishes Observations and status to a standard MQTT Broker."
language: en
translation_key: contracts.mqtt-output-adapter-v1
status: draft
revision: 2
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
The Observation model itself (kind, series, sequence, the two clocks) lives in the [product model](../concepts/product-model.md); this contract maps it onto MQTT.

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

### Topic-first type selection

The kind is not in the payload. Consumers do not consult pipeline definitions; they settle the kind from the trailing `{kind-key}` of the topic first and then decode the payload as that kind's fixed type. There is no need to decode the JSON into a generic value and discriminate afterwards.

```text
iotkit/v1/edge-node/+/observation/+/measurement        -> value: JSON number
iotkit/v1/edge-node/+/observation/+/state              -> value: boolean
iotkit/v1/edge-node/+/observation/+/accumulated-count  -> value: non-negative integer
```

Subscribing with one filter per kind gives each receive callback a fixed type. With a single filter, branch on the trailing segment and then use the same fixed types.

## 4. Observation payload

```json
{
  "series_id": "018f5f83-7a2b-7c61-a729-6af238f558e0",
  "sequence": 42,
  "uptime_ms": 8123456,
  "unix_epoch_ms": 1784190000123,
  "value": 1250
}
```

| Field | Type | Meaning |
|---|---|---|
| `series_id` | string (1 to 64 bytes of UTF-8) | One continuous generation of the same pipeline output. An opaque string compared only for equality; consumers do not validate it as a UUID or check a version |
| `sequence` | integer (1 to 2^53−1) | Starts at 1 within a series and increases by 1 per publication. Retransmissions reuse the value |
| `uptime_ms` | integer (≥ 0) | Milliseconds from the boot of the device running IoTKit to the receipt of the input that settled this output, from a monotonic clock. Always present |
| `unix_epoch_ms` | integer (Unix epoch ms) or `null` | The wall-clock time at which that input was received. An integer only while IoTKit can vouch for its clock; otherwise `null`. The key is always present |
| `value` | by kind | See below |

For a debounced transition both times refer to the input that completed the debounce.

| kind-key | `value` | Notes |
|---|---|---|
| `measurement` | JSON number | Integer or fraction. The unit is not in the payload; it is pipeline configuration and consumer registration |
| `accumulated-count` | integer ≥ 0 (≤ 2^53−1) | The count computed by the pipeline. Monotonic within a series |
| `state` | boolean | The current state after thresholding, hysteresis, and debounce |

These five fields are the whole payload; no others are added.
IoTKit sends the payload as compact JSON with keys in this order (`{"series_id":…,"sequence":…,"uptime_ms":…,"unix_epoch_ms":…,"value":…}`). Producer conformance compares these bytes with the fixtures.
Consumers parse the payload as JSON and must not depend on key order or whitespace.

### Two clocks

IoTKit distinguishes a monotonic clock from the wall clock and promises only what each can guarantee.

| | `uptime_ms` (monotonic) | `unix_epoch_ms` (wall clock) |
|---|---|---|
| Guaranteed | Monotonic and continuous while the device stays up; an IoTKit process restart does not break it. The difference between two Observations' `uptime_ms` equals the real time elapsed between them | When an integer, a wall-clock time IoTKit judged trustworthy (for example after NTP synchronization) |
| Not guaranteed | Resets to zero when the device reboots. Not directly comparable with another device's or the consumer's clock | Stays `null` right after boot on devices without an RTC and on sites without NTP. May jump when synchronization happens |

Consumers use them as follows.

- Ordering and de-duplication use `sequence`, never a time.
- The interval between two Observations (cycle time, length of a gap) is the difference of `uptime_ms` within the same boot. A decrease relative to the previous Observation means the device rebooted; do not compute an interval across that boundary. A reboot does not change the series.
- Calendar placement uses `unix_epoch_ms`. When it is `null`, the consumer can build one anchor per boot as "own receipt time of a live message minus its `uptime_ms`" and place every Observation of that boot, including those buffered in the outbox during a Broker outage and delivered after reconnection. Observations buffered before a reboot and delivered without wall-clock time keep their intervals but cannot be placed on the calendar.
- `null` is not an error. A change of `unix_epoch_ms` from `null` to an integer tells the consumer that the device clock became trustworthy.

NTP synchronization is recommended and verified at installation where it is available. Interval measurement through `uptime_ms` holds on sites without it.

### Publish rate

- `measurement` is published only when the calibrated value differs from the last value published in the same series (on change). The first input of a series is always published. While the value is unchanged nothing is published; consumers rely on the retained latest value and the heartbeat for the current value and liveness. The worst-case rate equals the input rate.
- `state` is published on the first input of a series and whenever the state settles to a different value after thresholding, hysteresis, and debounce.
- `accumulated-count` is published at series start (`sequence = 1, value = 0`) and whenever the count increases.

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
  "uptime_ms": 8123456,
  "unix_epoch_ms": 1784190000123,
  "value": "online"
}
```

`uptime_ms` and `unix_epoch_ms` mean the same as in an Observation (section 4). The `uptime_ms` of a heartbeat is the device's continuous running time.

`value` has three values.

| value | Meaning | Effect on Observations |
|---|---|---|
| `online` | Connected to MQTT and persisting input | None |
| `degraded` | Connected to MQTT and still sending the persisted outbox, but new input cannot be persisted and is discarded | Increments of `accumulated-count` in this interval are lost and never arrive. Consumers record the interval as a gap |
| `offline` | Not connected to MQTT | Observations in this interval are preserved in the outbox and arrive after recovery |

- A heartbeat is the periodic `online` or `degraded`. IoTKit publishes it immediately after connecting and then every `heartbeat_interval` (default 60 s, 5 s to 1 h) with QoS 1 and retain.
- A change between `online` and `degraded` is published immediately, without waiting for the interval.
- At graceful shutdown IoTKit publishes `offline` with `uptime_ms` and, when trusted, `unix_epoch_ms`, QoS 1 and retain, waits up to 2 s for PUBACK, and disconnects.
- On abnormal disconnect the Broker publishes IoTKit's Will (QoS 1, retain). The Will is registered at connect time, so IoTKit observed neither the time nor the uptime of the disconnect and both are `null`.

```json
{
  "uptime_ms": null,
  "unix_epoch_ms": null,
  "value": "offline"
}
```

## 8. Pipeline deletion

When a pipeline is deleted, IoTKit publishes a zero-length payload with retain to its Observation topic.
This publication goes through the same outbox as every Observation, so a deletion while the Broker is unreachable still arrives after reconnection and no stale retained value is left on the Broker.
The Broker clears the retained value, and consumers that subscribe later receive nothing.
Consumers already subscribed receive the zero-length payload. It is not malformed JSON; it is the settled fact that this input is no longer available.

## 9. Consumer obligations and assumptions

- Keep the highest received `sequence` per `series_id` and discard anything at or below it as a duplicate. When `series_id` changes, drop the stored maximum and accept the new series. A series change is a baseline update, not an anomaly.
- A gap in `sequence` is not an anomaly. Retain keeps only the latest value, so intermediate values published while the consumer was disconnected do not arrive. `accumulated-count` is cumulative, so the latest value is enough to catch up. A consumer that needs every Observation uses a persistent session with the Broker.
- Treat the retained latest value delivered right after subscribing as the initial value. The first `accumulated-count` received is a baseline; do not add its total to business results.
- Treat a decrease of `accumulated-count` within one series as an anomaly; do not silently replace the baseline.
- Treat a zero-length payload as pipeline deletion (section 8).
- Record `degraded` intervals as gaps and distinguish them from `offline` (section 7).
- When a heartbeat's `unix_epoch_ms` is an integer, its difference from the receipt time reveals device clock drift; while it is `null` drift cannot be measured. Do not discard Observations on time difference alone.
- Settle the kind from the trailing topic segment and decode the payload without consulting pipeline definitions (section 3).

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
| Producer check (the topics and bytes IoTKit generates compared with the fixtures) | `edge-node/core/pipeline/tests/unit/wire_tests.rs` (`cargo test -p iotkit-core-pipeline`) |

The fixtures are the reference for both producer and consumer. IoTKit's conformance test compares the topics and bytes it publishes with them; consumer conformance tests and simulators generate payloads from them.
A disagreement between document, schema, fixtures, or checks is a contract defect; none is silently adjusted to another.

## 11. Open items

None at present. The `measurement` publish rate (section 4) and the outbox path of the deletion notice (section 8) were decided in #232 child issue 3.
