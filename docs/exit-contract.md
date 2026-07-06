# Exit contract (R10)

How an **archive consumer** receives measurements from the gateway and what it
must do. This is the contract implemented by the push task
(`iotkit-gateway/src/publish_task.rs`) and the record types
(`iotkit-gateway/src/record.rs`). The MVE supports a single archive target;
multi-consumer, replay, and bounded backfill are future work.

## Roles

- **Gateway** — buffers measurements and pushes them out. Holds custody until the
  consumer acknowledges.
- **Archive consumer** — an HTTPS endpoint that durably stores what it receives
  and acknowledges a cursor. Its ack is what authorizes the gateway to purge.

The consumer has **no special privileges** — it is just one consumer of the
contract (D7). It must be registered with `gatewayctl target add`, which requires
an `https://` endpoint and a connectivity+auth smoke check before delivery is
enabled.

## Delivery

The gateway POSTs batches to the target endpoint:

```
POST <endpoint_url>
Authorization: Bearer <per-target token>
Content-Type: application/json

{
  "publication_id": "<target_id>:<epoch>:<cursor_start>:<cursor_end>",
  "records": [ <record>, ... ]
}
```

- **`publication_id`** is deterministic: `target_id:epoch:cursor_start:cursor_end`.
  The same cursor range always produces the same id, so a crash-and-resend is
  byte-identical and the consumer can dedupe.
- **At-least-once.** After a crash between POST and cursor-advance, the gateway
  re-sends the same range with the same `publication_id`. The consumer must
  tolerate duplicates.
- **Batches are bounded** by record count and a byte cap; a single record larger
  than the cap is still delivered alone (never an empty stall).
- **HTTPS only.** The gateway refuses to POST a bearer token to a non-`https://`
  endpoint (enforced at every use site, not just at `target add`).

## Records

Two families. **Record identity is `(epoch, pub_seq)`** — the consumer should
idempotently upsert on that pair. `readings.seq` (the gateway's internal
sequence) is never sent.

### measurement

```json
{
  "family": "measurement",
  "schema_version": 1,
  "epoch": "<ledger epoch, UUIDv7>",
  "pub_seq": 12345,
  "series_key": "<system_id>:<measurement_key>:<channel|na>:<variant>",
  "values": [ 21.5 ],
  "event_time": 1720000000000,
  "event_time_source": "device | gateway_adjusted | received_at",
  "time_source": "device_ntp | device_rtc | gateway | gateway_adjusted",
  "time_quality": "<D1 time quality>",
  "received_at": 1720000000123,
  "device_time": 1720000000000
}
```

- `series_key` is the stable logical identity of the series (channel `-1` renders
  as `na`). Treat it as an opaque key unless you have the gateway's series table.
- `device_time` may be `null`.

### annotation

Stream metadata sharing the same `pub_seq` sequence. Currently only `epoch_start`:

```json
{
  "family": "annotation",
  "schema_version": 1,
  "epoch": "<new epoch>",
  "pub_seq": 42,
  "subtype": "epoch_start",
  "prior_epoch": "<old epoch>"
}
```

## Acknowledgement

The consumer responds to a batch POST with:

```json
{ "publication_id": "<echo of the request publication_id>", "acked_pub_seq": <cursor_end> }
```

The gateway advances the target's cursor **only if**:

- `publication_id` matches the one it sent (this also confirms the epoch, since
  the epoch is embedded in it), **and**
- `acked_pub_seq == cursor_end` (all-or-nothing for the batch; partial ack is not
  supported in the MVE), **and**
- the HTTP status is 2xx.

Anything else → the cursor does not move and the batch is retried with bounded
backoff. The cursor never moves backward.

## Epochs and restore

`epoch` fences restore generations. A snapshot restore (hardware swap) mints a
**new** epoch and enqueues an `epoch_start` annotation carrying the `prior_epoch`.
The consumer should treat a new epoch as a signal to re-baseline its cursor for
this gateway: records under the new epoch start from the smallest `pub_seq`, and
the consumer's old-epoch cursor no longer applies. The gateway cannot promise to
re-deliver data from before the restore (that box is gone).

## Retention interaction (why the ack matters)

The gateway purges a reading only when it is both **past a retention floor**
(data-age based, default 72h, configurable) **and already acknowledged** for the
registered archive target. Un-acknowledged originals are protected even when old.
So: **if you stop acking, the gateway stops purging** — the backlog grows and,
under sustained pressure, new writes eventually fail with `ENOSPC` rather than
silently dropping stored data. Keeping up with acks is how you keep the buffer
drained.
