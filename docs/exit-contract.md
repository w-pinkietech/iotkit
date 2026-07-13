# Exit contract (R10)

Status: Approved MQTT v1 target contract. The checked-in Gateway publisher remains transitional HTTPS
until the MQTT slice is implemented.

This contract defines how canonical records leave one Gateway and when that Gateway may transfer
custody. Application meaning such as production, OEE, process, alarm text, or YokaKit state is not
part of this contract.

## Roles

- **Gateway publisher** reads the durable outbox, publishes bounded batches, retries, and owns the
  local delivery cursor.
- **Standard MQTT broker** transports QoS 1 messages. Its PUBACK confirms broker receipt only.
- **Site Archival Store** durably stores canonical records and the contiguous accepted-through
  cursor, then publishes the application custody acknowledgement.
- **Application consumer** such as YokaKit reads canonical records and maps them into its own domain.
  Its business result does not authorize Gateway purge.

## Topics

```text
iotkit/v1/gateways/{gateway_identity}/records
iotkit/v1/gateways/{gateway_identity}/accepted-through
```

Both topics use QoS 1 and MUST NOT be retained. ACLs restrict each Gateway to publishing its own
records and subscribing to its own acknowledgement. Application-specific topics are outside R10.

## Record batch

```json
{
  "schema_version": 1,
  "gateway_identity": "gateway-01",
  "ledger_epoch": "01J...",
  "publication_id": "gateway-01:01J...:123:130",
  "cursor_start": 123,
  "cursor_end": 130,
  "records": []
}
```

Requirements:

- `cursor_start..cursor_end` is a non-empty contiguous publication range.
- The first and last record `pub_seq` match the range, with no gaps or duplicates inside the batch.
- Retry preserves the same publication ID, range, and record content.
- Global record identity is `(gateway_identity, ledger_epoch, pub_seq)`.
- Event time may be late or non-monotonic and is never a delivery cursor.
- Version 1 limits a batch to 256 records and 1 MiB encoded size.
- The initial publisher permits one application-unacknowledged batch at a time.

`publication_id` is a deterministic correlation and replay identity. Site stores a fingerprint of
the received record content. Receiving different content for an existing global record identity is
a custody conflict; it is never last-write-wins.

## Record families

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
  "time_quality": "synchronized",
  "received_at": 1720000000123,
  "device_time": 1720000000000
}
```

`device_time` may be null. All numeric values must be finite. `series_key` is opaque to consumers;
YokaKit or another application maps it to equipment and business meaning outside IoTKit.

### Annotation

Version 1 retains the existing `epoch_start` annotation. Annotation records share the same
publication sequence as measurements and therefore participate in the same contiguous cursor.

Additional record families require a versioned contract change. They are not added merely to mirror
an application table or MQTT topic.

## Application custody acknowledgement

After storing a batch, Site publishes:

```json
{
  "schema_version": 1,
  "gateway_identity": "gateway-01",
  "ledger_epoch": "01J...",
  "publication_id": "gateway-01:01J...:123:130",
  "accepted_through": 130
}
```

For a valid batch, Site performs one custody transaction:

1. Authenticate the Gateway topic and validate version, identity, epoch, bounds, and contiguous range.
2. Insert or idempotently verify every raw canonical record.
3. Advance that Gateway and epoch's contiguous accepted-through cursor.
4. Commit the raw records and cursor atomically with durable SQLite settings.
5. Only after commit, publish the correlated accepted-through message.

SQL failure, ENOSPC, corruption, cancellation before commit, a gap, or a content conflict MUST NOT
produce an accepted-through acknowledgement. A lost acknowledgement causes a harmless exact replay.

Gateway validates schema version, topic/body Gateway identity, epoch, publication ID, monotonicity,
and that `accepted_through` does not exceed the published batch. Only then may it advance its target
cursor. MQTT PUBACK never advances this cursor and never authorizes retention purge.

## Retry and outage behavior

- Gateway outbox is the retry authority and remains durable until application acknowledgement.
- Broker receipt may release MQTT protocol inflight state but not the application sending window.
- If Site or the network is down, Gateway continues local collection and retains unacknowledged rows.
- On reconnect, Gateway republishes the same batch until Site confirms the contiguous cursor.
- Site exact replay verifies existing rows and republishes the already committed watermark.

## Authentication

The first implementation uses MQTT over TLS inside the selected tailnet, anonymous access disabled,
and one static credential plus topic ACL per Gateway. Secrets are stored outside Git and never appear
in argv, logs, Debug output, audit detail, or query output. D10 owns later authentication hardening.

## YokaKit boundary

IoTKit publishes canonical observations. YokaKit owns equipment/process mapping, production state,
OEE, alarms, UI, and notifications. A YokaKit adapter may consume this stream and produce internal
events, but YokaKit vocabulary does not enter R10 and YokaKit business success is not a custody ack.

## Deferred

The following are outside the initial one-Gateway vertical slice: series-definition replication,
terminal/gap repair protocol, multi-Gateway fleet operations, Site query projection, generic broker
fan-out, legacy HTTPS migration, and alternative egress bindings.
