# Exit contract (R10)

Status: **MQTT contract design candidate; final review pending** (D7/D9; 2026-07-13). The checked-in
Gateway publisher is still the transitional HTTPS MVE until the planned MQTT migration lands; HTTPS
behavior is not the production contract promised by this document.

This document tells an exit consumer how canonical records leave a Gateway and what an Archival
Store must prove before the Gateway may transfer custody.

## Roles

- **Gateway publisher** — reads the durable outbox, sends bounded batches, retries unchanged batches,
  and maintains target delivery state.
- **Egress binding** — maps the transport-independent contract onto MQTT QoS 1. It does not define
  application meaning or custody policy.
- **Archival Store** — a first-class target that durably stores canonical records and the contiguous
  accepted-through cursor. Its formal ack is the only ack that may authorize Gateway purge.
- **Application projection adapter** — an application-specific, rebuildable consumer such as
  YokaKit. It is non-custodial unless separately implemented and designated as the Archival Store.

The first implementation supports one archive target. The identities and cursor rules remain scoped
per target so later fan-out cannot let one consumer advance another consumer's state.

## MQTT binding

The Gateway connects outward to the registered target using MQTT over TLS, QoS 1, a pinned Site
certificate, and a per-Gateway/per-target bound credential. A general broker PUBACK is delivery
confirmation only; an archival target must be the custody-aware listener described below.

Version-1 namespace:

```text
iotkit/v1/gateways/{gateway_identity}/records
iotkit/v1/gateways/{gateway_identity}/ack-detail
iotkit/v1/gateways/{gateway_identity}/terminal-notice
iotkit/v1/gateways/{gateway_identity}/resync-request
iotkit/v1/gateways/{gateway_identity}/series-snapshot
iotkit/v1/gateways/{gateway_identity}/series-snapshot-ack
```

- Gateway publishes only to its `records`, `resync-request`, and bootstrap `series-snapshot` topics and subscribes only to its
  `ack-detail`, `terminal-notice`, and `series-snapshot-ack` topics.
- Topics and ack details are never retained.
- Wildcard application publish, broker bridging, device-to-device routing, and application-specific
  topics such as `production` are outside the canonical binding.
- For production records, a pending enrollment attempt may publish only the exact contiguous range
  `(accepted_through + 1)..=smoke_pub_seq` on its bound target/endpoint/epoch, under bounded
  byte/count/inflight/time/retry limits. It cannot publish later records or another namespace. It
  may additionally use only its bounded enrollment status, resync, and required legacy snapshot
  control topics, none of which carries purge authority.

## Batch

A publish payload is a bounded canonical batch:

```json
{
  "schema_version": 1,
  "gateway_identity": "<stable gateway id>",
  "ledger_epoch": "<restore generation>",
  "target_id": "<logical target id>",
  "target_endpoint_id": "<registered endpoint identity>",
  "publication_id": "<deterministic target+epoch+range id>",
  "cursor_start": 123,
  "cursor_end": 130,
  "records": ["<canonical measurement/series_definition/annotation records>"]
}
```

- Delivery is **at least once**. Retry preserves `publication_id`, range, and record content.
- Before first publish, Gateway durably records the exact canonical encoded payload plus binding
  generation, endpoint, epoch, range, publication ID, payload fingerprint, contract version, and
  versioned batch policy. Restart or
  configuration change replays that exact attempt and payload until formal advancement or terminal
  quarantine.
- The range is contiguous in publication order. Event time is not a cursor and may be late or
  non-monotonic.
- Batches have configured record/byte/inflight limits. The listener keeps MQTT network/keepalive
  processing alive while its bounded commit queue is backpressured.
- Global record identity is `(gateway_identity, ledger_epoch, pub_seq)`. The Site Server also stores
  a payload fingerprint. The same identity with different content is an integrity/custody conflict,
  never last-write-wins.

## Record families

### `measurement`

One point in one stable series:

```json
{
  "family": "measurement",
  "schema_version": 1,
  "pub_seq": 123,
  "series_key": "<opaque stable series identity>",
  "values": [21.5],
  "event_time": 1720000000000,
  "event_time_source": "device | gateway_adjusted | received_at",
  "time_source": "device_ntp | device_rtc | gateway | gateway_adjusted",
  "time_quality": "<contract value>",
  "received_at": 1720000000123,
  "device_time": 1720000000000
}
```

`device_time` may be null. Values must obey the registered type/unit/finite-value contract. A future
derived series is a distinct series with immutable derivation provenance; it never overwrites its
source observation.

### `series_definition`

Mandatory versioned metadata copied from the Gateway's authoritative R11 registry so the Site can
interpret measurements without a live reverse query:

```json
{
  "family": "series_definition",
  "schema_version": 1,
  "pub_seq": 122,
  "series_key": "<opaque stable series identity>",
  "definition_revision": 3,
  "effective_from_pub_seq": 123,
  "measurement_key": "illuminance_lux",
  "unit": "lx",
  "value_type": "float",
  "channel": "na",
  "series_variant": "raw",
  "value_semantics": "calibrated",
  "registry_revision": 7
}
```

The applicable definition precedes every affected measurement in publication order. History is
immutable. Subject/user labels, hardware identity, process, part, order, and application mapping are
not part of this family.

Absent, late, conflicting, or invalid required definitions are deterministic
`record_schema_invalid` failures: no custody ack, correlated terminal notice, and Gateway outbox
quarantine. Site validates measurement type/unit/value against the applicable definition before raw
custody commit.

For retained publication rows created before this family existed, MQTT cutover first sends one
immutable, hashed `series_definition_snapshot` containing all required definitions, registry revision,
and `snapshot_through_pub_seq`. Site must durably acknowledge this snapshot before accepting production
batches. It is bootstrap metadata and cannot advance the production/purge cursor. Later definition
changes use normal earlier-pub-seq records. If historical definitions cannot be reconstructed, cutover
stops rather than applying current metadata retroactively.

Site acknowledges this bootstrap only on the non-retained `series-snapshot-ack` topic. The tagged
ack contains `schema_version`, `snapshot_id`, snapshot hash, Gateway/target/endpoint/epoch,
`snapshot_through_pub_seq`, `registry_revision`, and `purge_authority=false`. Resync returns this
stored state after response loss. Transient failure produces no ack and a reconnect/retry;
deterministic invalid/conflicting snapshot state produces a tagged terminal notice with
`object_kind=series_snapshot`, snapshot ID/hash, and Gateway/target/endpoint/epoch correlations;
production-only publication/range fields are absent. Neither case advances the production cursor.

### `annotation`

Stream/custody metadata shares the publication sequence. Version 1 includes `epoch_start` and the
structured gap/custody annotations defined by D7. Commissioning smoke is a synthetic record in the
normal production stream with a real pub_seq; activation requires the contiguous formal watermark to
reach it, so it cannot skip a backlog. The custody transaction that first crosses the expected smoke
sequence also activates the pending enrollment/credential slot. It is excluded from ordinary
measurement queries.

## Store, then acknowledge

For a valid gap-free batch, the Archival Store:

1. authenticates the Gateway/target/namespace and validates schema, epoch, range, size, and hashes;
2. begins one custody-critical transaction;
3. inserts or idempotently verifies all canonical records;
4. advances that Gateway's contiguous `accepted_through` cursor in the same transaction;
5. commits with durability equivalent to SQLite `WAL + synchronous=FULL`;
6. only then emits MQTT PUBACK and publishes the formal ack detail.

Example ack detail:

```json
{
  "schema_version": 1,
  "gateway_identity": "<stable gateway id>",
  "target_id": "<logical target id>",
  "target_endpoint_id": "<registered endpoint identity>",
  "ledger_epoch": "<restore generation>",
  "publication_id": "<echoed deterministic id>",
  "accepted_through": 130
}
```

The Gateway advances its target cursor only after validating every correlation field and a
non-regressing accepted-through value for the current epoch. Under D9's gap-free cumulative-equivalent
rule, a success PUBACK may confirm the just-committed batch; the explicit ack detail remains the
inspectable/formal watermark and is resynchronized after reconnect.

ENOSPC, corruption, SQL failure, cancellation before commit, invalid schema/range, partial storage,
or payload conflict produces no success PUBACK and no accepted-through advancement. A transient
storage failure is not a terminal rejection. Commit followed by response loss causes harmless replay.

## Epoch, restore, and clone behavior

Formal Gateway restore preserves logical identity but mints a new ledger epoch. The Site Server keeps
an active-epoch registry and reconciles the new epoch through the enrollment/recovery flow. Old-epoch
sessions cannot advance the new epoch's cursor. Same identity/key/epoch oscillation or equal record
identity with different content fences delivery for operator recovery.

Site restore starts network-unbound. A root-owned generation anchor outside the ordinary DB backup
must match the live DB and no restore marker may exist before listeners bind. Official backup artifacts
are non-live and import into a fresh generation through a local typed operation that invalidates
tickets/pending/verifiers/sessions and reconciles each Gateway's box key, epoch, cursor, and retained
range. Full-disk/root rollback that also restores the anchor requires an external witness to detect and
is an explicit residual limitation.

## Query projections do not acknowledge custody

The Site query projector runs after raw archival commit. Its tables and checkpoint may be dropped and
rebuilt from canonical raw records. Projection failure or lag cannot delay, synthesize, or replace the
archival ack. Query responses expose the projection watermark/incomplete state.

## Retention interaction

Normal Gateway retention removes archive-acknowledged data only after the minimum retention floor.
Under resource pressure the authoritative D2/R17 order is:

1. archive-acknowledged data beyond the floor;
2. data outside the configured custody policy;
3. unresolved quarantine data; and
4. unacknowledged originals only as the final explicit data-loss class.

The fourth class requires a `custody_lost` audit event and a structured gap annotation. Silent
deletion is forbidden. If the Archival Store stops acknowledging, the backlog and risk horizon must
be visible; no ordinary broker PUBACK may make that backlog purgeable.

Generic non-custodial brokers and Site republishing are deferred from the MVP. Version 1 implements
only the custody-aware Site listener; later D9 broker compatibility cannot inherit archival or purge
semantics from an ordinary PUBACK.

## Terminal gaps and resynchronization

- Queue saturation before a commit attempt delays admission/PUBACK while network keepalive continues.
  Once an actual transient commit attempt fails, Site returns no PUBACK/terminal notice and explicitly
  closes the connection after bounded cleanup; Gateway clean-reconnects and republishes from outbox.
- Deterministic contract violations or payload conflicts return no PUBACK and emit a correlated,
  stable-reason terminal notice. Production-record variant fields are `schema_version, gateway_identity, target_id,
  target_endpoint_id, ledger_epoch, publication_id, cursor_start, cursor_end, reason_code,
  diagnostic_id`; the snapshot variant is defined above. No secret/raw payload is included.
  Version-1 terminal reasons are
  `contract_major_unsupported`, `target_binding_mismatch`, `epoch_conflict`,
  `publication_identity_mismatch`, `cursor_range_invalid`, `record_schema_invalid`, and
  `payload_conflict`. Storage/timeout/overload errors are forbidden from this list. Gateway
  quarantines the batch without deleting it and reconnects.
- While a gap exists, later PUBACKs release only inflight capacity; formal `accepted_through` remains
  before the gap and is the only purge watermark.
- Clean MQTT sessions are used; the outbox is retry authority. Gateway waits for SUBACK on
  ack/terminal/snapshot-ack subscriptions, then publishes a correlated resync request. Site returns
  current watermark/terminal/snapshot state after subscription readiness. Request/response includes
  fresh `request_id`, Gateway/target/endpoint/epoch correlations, the Gateway's local watermark, and
  local snapshot ID/hash. Retained messages are forbidden.

## Application compatibility

Legacy `production`, `alarm`, `onoff`, `barcode`, and `gantt-chart` payloads are generated, if needed,
by an application projection adapter outside Gateway Core. The canonical publisher never reshapes a
record for a particular target or imports YokaKit business masters.
