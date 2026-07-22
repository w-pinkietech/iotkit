---
type: Contract
title: "IoTKit authenticated ingest contract v1"
description: "Defines the complete authenticated HTTP ingest wire schema, authority, retry, validation, limits, and recovery semantics."
language: en
translation_key: contracts.ingest-v1
status: stable
revision: 2
---

# IoTKit authenticated ingest contract v1

Status: **normative for the authenticated HTTP device-ingest binding**.

This document describes the wire contract implemented by `iotkit-ingest-http`.
The JSON types in `iotkit-ingest-contract` are the shipped reference
representation; this document is the device-builder-facing contract. The
endpoint is versioned at `/api/v1` and accepts JSON over the authenticated
local-network listener.

## The first three commands

An operator must complete the handoff before the device builder runs these
commands. The handoff contains:

- the enabled Edge Node URL, including the `https://` scheme and port;
- one current device bearer token, shown once by the operator's credential
  operation;
- the selected ingress Edge Node public certificate saved as a PEM CA/trust-anchor file; and
- the configured source identifier for that token (normally the stable
  `principal_id`).

The operator may use `iotkit-edge-nodectl device-credential issue` for the one-time
token. The operator must use the fingerprint returned by the construction-tier
ingress TLS operation and hand the matching certificate from the selected
`ingress-tls/generation-N` to the builder. `iotkit-edge-nodectl fingerprint` reports the
control-plane certificate; it is not an ingress trust anchor unless the
operator has deliberately configured the exact same certificate for both
listeners. The token is not written into a configuration file by IoTKit. The
operator supplies the four `IOTKIT_OPERATOR_*` values to the device-builder
shell, for example through the handoff terminal or an ephemeral environment.
Do not put the bearer in source control or a shared image.

Run these commands verbatim in one shell, with the handoff values present:

```sh
export IOTKIT_URL="${IOTKIT_OPERATOR_URL:?set by operator handoff}" IOTKIT_TOKEN="${IOTKIT_OPERATOR_TOKEN:?set by operator handoff}" IOTKIT_CA="${IOTKIT_OPERATOR_CA:?set by operator handoff}" IOTKIT_SOURCE="${IOTKIT_OPERATOR_SOURCE:?set by operator handoff}" IOTKIT_ENVELOPE="${PWD}/one-envelope.json"
printf '%s\n' '{"envelope_id":"builder-example-0001","source":"'"$IOTKIT_SOURCE"'","items":[{"measurement_key":"temperature_c","values":[21.5],"time_source":"edge_node"}]}' > "$IOTKIT_ENVELOPE"
curl --fail-with-body --silent --show-error --cacert "$IOTKIT_CA" --header "Authorization: Bearer $IOTKIT_TOKEN" --header "Content-Type: application/json" --data-binary "@$IOTKIT_ENVELOPE" "$IOTKIT_URL/api/v1/ingest"
```

The first command only exports handoff values into the current shell. The
second writes one complete envelope. The third pins the Edge Node certificate
through `--cacert`; it must never be replaced with `--insecure`. An identical
`envelope_id` and identical payload must be retained for every retry. A `200`
acknowledgement is the only response in this journey that authorizes the sender
to discard its local copy, and only the acknowledgement status determines
whether that copy is complete.

### ESP32 equivalent

An ESP32 client uses the same HTTPS endpoint, bearer header, JSON envelope, and
retry rules. Provision the Edge Node public certificate (or the approved public
SPKI trust anchor) in the device's read-only trust store. Configure the TLS
client to verify the Edge Node certificate and hostname on every connection, then
send `Authorization: Bearer <token>`. A bearer token without server
authentication is not a supported setup. Do not disable certificate validation
to make a first connection work.

## Wire schema

### Request envelope

`POST /api/v1/ingest` has `Content-Type: application/json`. The body is one
`Envelope`:

```json
{
  "envelope_id": "builder-example-0001",
  "source": "<operator-provided principal_id>",
  "items": [
    {
      "measurement_key": "temperature_c",
      "values": [21.5],
      "time_source": "edge_node"
    }
  ]
}
```

### Envelope field table

This table is normative. “No independent field limit” means the shipped Rust
type performs no additional length or non-empty check; the HTTP decoder still
enforces the finite decoded-body limit below. JSON integer ranges are the exact
Rust representation ranges, not a looser JavaScript-number promise.

| Field | JSON type | Requiredness | Constraint and receiver meaning |
| --- | --- | --- | --- |
| `envelope_id` | JSON string | required | No independent field limit; preserve byte-for-byte across retries and deduplication is scoped by authenticated principal. |
| `source` | JSON string | required | No independent field limit; diagnostic/configured-source value and never an authority selector. |
| `declaration_version` | JSON unsigned integer (`u32`) or `null` | optional | `0..=4,294,967,295`; the current collector carries it without declaration-dependent behavior. |
| `items` | JSON array of `ReadingItem` | required | 0..=256 items in the shipped collector; an HTTP deployment may set a smaller positive limit. |

An empty `items` array is representable by the shipped type. `envelope_id` and
`source` are not silently defaulted or inferred.

### ReadingItem field table

| Field | JSON type | Requiredness | Constraint and receiver meaning |
| --- | --- | --- | --- |
| `subject_hint` | JSON string or `null` | optional | No independent field limit; omitted only resolves from a one-subject authenticated scope. |
| `measurement_key` | JSON string | required | One or more dot-separated segments; every segment is `[a-z][a-z0-9_]*`, UTF-8 length is at most 64 bytes, and empty/uppercase/colon segments are invalid. |
| `channel_index` | JSON unsigned integer (`u16`) | optional | 0..=65535; omitted means the declaration's no-channel sentinel. |
| `series_variant` | JSON string or `null` | optional | No independent field limit; omitted uses the shipped receiver default `primary`. |
| `values` | JSON array of finite numbers (`f64`) | required | Every number must be finite; registry declarations may impose the applicable value count/type. |
| `device_time_ms` | JSON signed integer (`i64`) | optional | −9,223,372,036,854,775,808..=9,223,372,036,854,775,807 Unix milliseconds; absolute freshness applies when device time is used. |
| `time_source` | JSON string enum | required | Exactly `device_ntp`, `device_rtc`, `edge_node`, or `edge_node_adjusted`; senders normally use `edge_node_adjusted` only as receiver-produced provenance. |
| `age_ms` | JSON unsigned integer (`u64`) | optional | 0..=18,446,744,073,709,551,615; accepted relative ages must also be within the configured freshness window and fit receiver subtraction. |
| `rssi` | JSON signed integer (`i16`) | optional | −32,768..=32,767; stored as optional radio metadata. |
| `battery_pct` | JSON unsigned integer (`u8`) | optional | 0..=255; the shipped type does not add a separate 0..=100 validation. |

`time_source: edge_node` without an absolute device timestamp uses Edge Node
receive time. A valid `age_ms` is reconstructed as Edge Node receive time minus
the age and is recorded as `edge_node_adjusted`; `device_time_ms` takes
precedence when both are supplied.

### Acknowledgement

The response body for a committed request is an `EnvelopeAck`:

```json
{
  "envelope_id": "builder-example-0001",
  "status": {
    "kind": "accepted",
    "items": [
      { "kind": "stored", "disposition": "durable" }
    ]
  }
}
```

Accepted item statuses are positional and have the same length and order as
the request items. A stored item has `disposition` `durable`, `staged`, or
`quarantined`; a quarantined item may include `quarantine_reason`. An item can
instead be terminally `item_rejected` with a `reason_code`, `message`, and
optional `field_path` JSON Pointer and `schema_hint`.

### EnvelopeAck and status field table

| Field/variant | JSON type | Requiredness | Constraint |
| --- | --- | --- | --- |
| `EnvelopeAck.envelope_id` | JSON string | required | Echoes the submitted identifier; no independent field limit beyond the HTTP body bound. |
| `EnvelopeAck.status` | tagged JSON object | required | Tag field is `kind`; exactly one of the envelope statuses below. |
| `accepted.items` | JSON array of `ItemStatus` | required for `accepted` | Exactly the request item count and order (0..=256). |
| `duplicate` | tagged JSON object | no extra fields | `{"kind":"duplicate"}`; sender may discard its identical spool copy. |
| `rejected.reason_code` | JSON `ReasonCode` string | required for `rejected` | Deterministic terminal envelope reason. |
| `rejected.message` | JSON string | required for `rejected` | Diagnostic text; not a machine-parsed retry instruction. |
| `rejected.field_path` | JSON string or `null` | optional | JSON Pointer when a field is identifiable. |
| `rejected.schema_hint` | JSON string or `null` | optional | Stable diagnostic schema/value hint. |
| `deferred` | tagged JSON object | no extra fields | Stable contract vocabulary for temporary deferral; normal HTTP admission uses `429`/`503` without an ack. |
| `stored.disposition` | JSON `Disposition` string | required for `stored` | Exactly one of `durable`, `staged`, `quarantined`. |
| `stored.quarantine_reason` | JSON `QuarantineReason` string or `null` | optional | Current producers provide it for a quarantined row when a concrete reason exists. |
| `item_rejected.reason_code` | JSON `ReasonCode` string | required for `item_rejected` | Deterministic terminal item reason. |
| `item_rejected.message` | JSON string | required for `item_rejected` | Diagnostic text; not a retry instruction. |
| `item_rejected.field_path` / `schema_hint` | JSON string or `null` | optional | JSON Pointer and stable schema/value hint. |

### ValidationReport field table

`POST /api/v1/ingest/validate` returns this type and never an `EnvelopeAck`.

| Field | JSON type | Requiredness | Constraint |
| --- | --- | --- | --- |
| `envelope_id` | JSON string | required | Echoes the submitted identifier. |
| `valid` | JSON boolean | required | `true` iff `issues` is empty. |
| `issues` | JSON array of `ValidationIssue` | required | Empty for a valid report; each issue is deterministic and side-effect-free. |
| `item_index` | JSON non-negative integer (`usize`) or `null` | optional | Omitted for envelope-wide issues; item positions are zero-based and within the submitted item array. |
| `reason_code` | JSON `ReasonCode` string | required | Same stable vocabulary as acknowledgement diagnostics. |
| `message` | JSON string | required | Human-readable diagnostic only. |
| `field_path` / `schema_hint` | JSON string or `null` | optional | JSON Pointer and stable schema/value hint. |

### Stable enum vocabulary

The shipped v1 wire strings are fixed as follows:

| Enum | Values |
| --- | --- |
| `AckStatus.kind` | `accepted`, `duplicate`, `rejected`, `deferred` |
| `ItemStatus.kind` | `stored`, `item_rejected` |
| `Disposition` | `durable`, `staged`, `quarantined` |
| `QuarantineReason` | `out_of_range`, `unknown_key`, `undeclared_channel`, `device_quarantined` |
| `TimeSource` | `device_ntp`, `device_rtc`, `edge_node`, `edge_node_adjusted` |
| `ReasonCode` | `malformed_measurement_key`, `value_type_mismatch`, `unknown_subject`, `subject_scope_violation`, `batch_too_large`, `stale_timestamp`, `internal` |

`internal` is a read-compatible legacy `ReasonCode` value and is not
serialized by the current producer. Storage/commit failure is never represented
by it: the response is no ack (`503` or connection failure).

The envelope-level statuses are:

```json
{"envelope_id":"builder-example-0001","status":{"kind":"duplicate"}}
```

```json
{"envelope_id":"builder-example-0001","status":{"kind":"rejected","reason_code":"subject_scope_violation","message":"subject is outside the token scope","field_path":"/items/0/subject_hint","schema_hint":"registered subject identifier"}}
```

`rejected` is terminal and is reserved for deterministic envelope violations.
The legacy `reason_code: "internal"` remains readable for old v1 data but is
not emitted by this implementation. Storage and internal failures do not become
`rejected`.

An accepted acknowledgement means that the Edge Node reached its documented
durability point: the reading and its same-transaction downstream publication
record, or the bounded staging/dedup state represented by the disposition, are
durable. A duplicate means the original envelope claim is still within the
bounded deduplication window and the sender may discard its copy. Neither
status promises that an archive consumer has already taken custody.

## Authentication, subjects, and authority

Send `Authorization: Bearer <device-token>`. Tokens are opaque, hashed at
rest, compared without exposing plaintext, and are never included in logs,
errors, audit details, health output, fixtures, or `Debug` output. A missing,
invalid, revoked, or stale token returns `401` with no ingest acknowledgement.

The authenticated principal owns the subject scope, deduplication namespace,
flow accounting, and audit attribution. `Envelope.source` cannot select any of
those.

- A one-subject token may omit `subject_hint`; the receiver resolves its one
  authorized subject.
- A multi-subject token must provide `subject_hint` on every item.
- A supplied subject outside the token scope is an item-level
  `subject_scope_violation`.
- An unknown subject supplied by an HTTP device token is an item-level
  `unknown_subject`; it is not staged by the network path.
- Only a trusted official in-process adapter principal may create a bounded
  unknown-subject sighting.

These failures are positional. Valid siblings still commit in the same
accepted envelope. A source/principal mismatch is an envelope-level terminal
`rejected` result and emits a bounded intrusion signal; it does not widen
authority.

## Time and freshness

The receiver owns receive time and clock provenance. The default absolute
freshness window is 24 hours and the default future-skew allowance is 5
minutes; deployments may choose smaller finite values within the implementation
limits.

| Input and Edge Node clock state | Result |
| --- | --- |
| No `device_time_ms`; `time_source: edge_node` | Accept using Edge Node receive time. |
| No absolute device time; finite `age_ms` within the window | Accept using receive time minus age and record `edge_node_adjusted`. |
| `age_ms` outside the window or not representable | Terminal item rejection with a freshness reason; do not blind-retry unchanged input. |
| `device_time_ms` while trusted wall time is available and timestamp is fresh | Compare against the trusted wall clock and accept or produce positional `stale_timestamp`. |
| `device_time_ms` while trusted wall time is untrusted | `503` with no acknowledgement for the whole envelope; retain and retry unchanged after clock recovery. |
| Trusted absolute timestamp older than the window | Positional `stale_timestamp`; valid siblings may commit. |
| Trusted absolute timestamp beyond the future-skew allowance | Positional terminal freshness rejection; correct the device clock. |

Startup is clock-untrusted. Sync evidence or explicit local-root time
confirmation is required before absolute device timestamps can claim trusted
wall-time comparison. A backward step or failed auth-time-floor write fails
closed. A restart reloads the persisted nondecreasing floor but starts clock
trust as untrusted again.

## Retry and HTTP response contract

The sender keeps the exact envelope, including `envelope_id`, across every
retry. Add bounded exponential backoff and jitter; honor `Retry-After` when it
is present. Never delete the sender copy because of a response that does not
claim custody.

| Condition | HTTP response | Sender action |
| --- | --- | --- |
| Committed accepted, duplicate, or terminal contract result | `200` + `EnvelopeAck` | Obey the status; discard only after accepted/duplicate, and fix/remove terminal input. |
| Missing or invalid credential | `401`, no ack body | Repair credentials; do not assume custody or blind-retry forever. |
| Authenticated source/principal mismatch | `200` + envelope-level `rejected` ack | Fix the source field and treat the unchanged envelope as terminal. |
| Authenticated item subject failure | `200` + accepted ack with positional `item_rejected` | Keep valid sibling results; fix future item input. |
| Pre-auth or authenticated throttle | `429` + bounded `Retry-After`, no ack | Retry the identical envelope with backoff and jitter. |
| Bounded queue unavailable or listener draining | `503`, no ack | Retry the identical envelope with backoff and jitter. |
| Storage, commit, clock-provenance, or internal failure | `503` or connection failure, no ack | Retain and retry the identical envelope. |
| Safely parsed deterministic malformed/oversize envelope | `200` terminal `rejected` ack; otherwise a bounded `4xx` without custody | Fix the input; do not blind-retry unchanged input. |

`Retry-After` is a bounded number of seconds. A `429`, `503`, timeout,
connection close, or empty body is never an implicit `rejected` and never
authorizes spool deletion. `AckStatus::deferred` remains a valid internal
contract value, but normal network admission uses `429` or `503`.

## Side-effect-free validation

`POST /api/v1/ingest/validate` uses the same bearer authentication, exposure,
header/body/time limits, and principal scope checks as ingest. It returns a
`ValidationReport`, never an `EnvelopeAck`:

```json
{
  "envelope_id": "builder-example-0001",
  "valid": false,
  "issues": [
    {
      "item_index": 0,
      "reason_code": "unknown_subject",
      "message": "subject is not registered",
      "field_path": "/items/0/subject_hint",
      "schema_hint": "registered subject identifier"
    }
  ]
}
```

Validation performs parsing, source/scope, schema, subject, and freshness
checks. It does not write readings, dedup claims, staging rows, successful
custody state, or an ingest acknowledgement. A security violation may still
create the same bounded intrusion episode as an actual ingest request. A
successful validation result never authorizes spool deletion; the sender must
still submit the unchanged envelope to `/api/v1/ingest`.

## Finite limits and listener exposure

The listener is disabled by default and is a separate construction-tier
local-network listener, not a control-API route. TLS is the normal mode. Private-LAN
plaintext is an explicitly degraded mode and is not suitable for the journey
above. Wildcard, public, Internet-capable, and proxy-derived exposures are
rejected. Accepted peers must be inside the configured private local ingress CIDR.

### Shipped finite receiver defaults

These are derived from the shipped `Default` constructors and hard limits. They
are receiver-side implementation defaults, not sender-configurable JSON fields;
construction-tier deployment may choose a smaller valid bound where stated.
Internal worker and bucket values are listed for finite-capacity evidence and
must not be treated as a new control-plane API.

| Resource | Default |
| --- | ---: |
| measurement key | 64 UTF-8 bytes; dot-separated `[a-z][a-z0-9_]*` segments |
| request headers | 32 headers / 8,192 bytes |
| decoded JSON body | 65,536 bytes |
| items per envelope | 256 hard maximum / 256 HTTP default |
| concurrent requests / connections | 16 / 32 |
| collector handoff queue | 8 |
| authentication cache | 64 entries / 60 seconds |
| header read / whole request / collector wait | 5 s / 10 s / 5 s |
| TLS handshake | 5 seconds per peer |
| Retry-After | 1 second; configured range 1..=3,600 seconds |
| pre-auth source state | 1,024 entries / 60 seconds / 8 failures per window |
| authentication workers / reserved workers | 2 / 1 |
| general authentication rate / burst / initial tokens | 16 / 32 / 1 |
| reserved authentication rate / burst / initial tokens | 8 / 8 / 1 |
| principal-state capacity | 64 principals |
| principal flow classes (low / default / high) | 1,000,000 / 1,000,000 rate and burst units each |
| global flow rate / burst | 4,000,000 / 4,000,000 units |
| throttle cooldown | 5,000 ms |
| relative freshness / future skew | 86,400,000 / 300,000 ms (24 hours / 5 minutes) |
| deduplication rows / principal rows / age | 100,000 / 10,000 / 72 hours |
| unknown-subject staging rows / bytes / age | 10,000 / 64 MiB global; 1,000 / 8 MiB per principal; 30 days |
| unknown-subject staging reserve | 256 rows / 64 KiB |
| staged rows per hardware identifier | 1,000 rows |
| opportunistic dedup purge interval | 3,600,000 ms (1 hour) |
| TCP listen backlog | 128 pending connections |

Staging retains a finite evictable reserve for one maximum envelope and accounts
for both rows and bytes. Deduplication is keyed by
`(stable_principal_id, envelope_id)`, bounded by age and row ceilings. A failed
maintenance purge degrades health and reduces duplicate-suppression guarantees;
it never changes a committed acknowledgement into an unacknowledged result.

Admission reserves conservative request cost before body consumption and
reconciles against actual bytes/items before collector handoff. Reservations
are released or refunded on timeout, disconnect, parse failure, cancellation,
or queue failure; already consumed authentication and work budget is not
refunded. Invalid input cannot grow an unbounded cache, queue, source map, or
audit stream.

## Current implementation and deferred work

The current implementation provides the authenticated HTTP binding, bearer credential
authority, bounded admission, TLS/private-LAN listener construction, freshness
handling, side-effect-free validation, bounded staging/dedup state, health and
episode audit hooks, and local recovery authority closure. It is intentionally a
device-builder HTTP path, not a remotely claimable Edge Node setup path.

Encrypted Edge Node replacement-backup containers and cross-filesystem restore
staging/fence mechanics remain deferred distribution work. Until that work lands,
legacy plaintext replacement snapshot export is unavailable once a
device-token secret exists; the Edge Node must say so without emitting the token or
its hash. State-only inspection is not a complete replacement backup.

MQTT ingest, pairing-window registration, `signed_seq`, `provisioned_key`, batch
provisioning, shared-image credentials, rich R23 UI, and destructive OS/image
factory reset are future or separately approved work. They are not alternate
ways to bypass this HTTP token, TLS, subject, freshness, or custody contract.

## Recovery and operational meaning

An unowned, locally recovering, restore-fenced, reset-fenced, or TLS-invalid
Edge Node does not bind the control API or ingest listener. Local root recovery
must re-establish ownership; restore invalidates prior admin/operator/session
authority and device auth generations are checked again. A device-token retry
after a committed restore may be accepted again on the replacement because
readings and dedup claims are not restored; downstream idempotency and the new
ledger epoch expose that possible duplicate.

A credential response is one-shot. If it is lost, abandon/revoke the affected
credential and issue another one; never ask the Edge Node to redisplay it. Human
approval is required for issue, reissue, promotion, abandonment, and revoke
operations that can silence a device.
