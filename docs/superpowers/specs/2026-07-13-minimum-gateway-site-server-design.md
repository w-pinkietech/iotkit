# Minimum Gateway + Site Server vertical slice design

Status: **User-approved written design** (2026-07-13)

This slice introduces a tier-[3] Archival Store, an MQTT custody endpoint, Gateway-to-Site
authentication bootstrap, a managed-overlay trust profile, and custody transfer.

Authorities: `docs/redesign/` (especially D2, D6–D10) and `docs/architecture.md`.

## 1. Mission brief

### Objective

Deliver the smallest system that can be installed on real hardware and prove the complete path:

```text
OPT3001 on Gateway Pi
  -> canonical illuminance observation
  -> Gateway durable acceptance
  -> MQTT QoS 1 exit delivery over the user's tailnet
  -> Site Server durable archival custody
  -> Site Server API/CLI query
```

The real deployment has one Gateway Pi and one Site Server on the user's home Linux machine.
Multi-Gateway behavior is tested with simulated Gateway identities but is not a real-hardware
acceptance requirement for this slice.

### Acceptance outcome

An installer can:

1. start the Site Server on the home Linux machine;
2. enroll one Gateway through local CLIs without exposing an operational MQTT credential;
3. connect an OPT3001 directly to the Pi's I2C bus;
4. observe canonical `illuminance_lux` records on the Gateway;
5. disconnect and restore the tailnet path without losing accepted records;
6. see the Site Server's accepted-through watermark catch up after reconnection;
7. query the archived values and their Gateway/series/time provenance through API and CLI;
8. resolve `illuminance_lux`, unit, value type, channel/variant, and definition revision without a
   live Gateway metadata query; and
9. demonstrate that a Site projection failure does not cause a false custody acknowledgement.

### Inviolable constraints

- No secret appears in logs, Debug output, audit details, process arguments, or query output.
- Only a durable archival commit may authorize Gateway purge.
- Storage failure, corruption, ENOSPC, partial commit, or an invalid batch produces no custody ack.
- The Site Server never replaces the Gateway collector and never reads a sensor bus directly.
- Gateway/YokaKit/application business vocabulary does not enter canonical measurement records.
- All mutations use typed, audited operation dispatch; no new direct-SQL mutation path.
- Gateway adapters do not depend on `core/engine` or Site Server crates.
- The selected path is MQTT QoS 1. The MVP does not maintain both MQTT and HTTPS publishers.

### Non-goals

- Web UI, Site Console UI, charts, dashboards, notifications, or remote write administration.
- BravePI/contact-count hardware acceptance, `production`, OEE, YokaKit compatibility, barcode,
  alarm, on/off, or gantt-chart application records.
- Derived-series processor or local-rule evaluator implementation.
- General plugin/DSL/rule-engine infrastructure.
- Multi-site tenancy, public Internet listeners, HA, fleet enrollment, batch provisioning, cloud
  aggregation, external CA, TPM, or secure-element support.
- Sophisticated fleet rotation scheduling. A minimum per-Gateway pre-expiry renewal loop is required
  in the MVP; campaign/canary scheduling is deferred.
- Preservation of the transitional HTTPS publisher as a second production binding.

## 2. Constraint ledger

### Settled facts

| Fact | Authority/consequence |
|---|---|
| Sensor normalization belongs to the connecting side | D1; OPT3001 decode and measurement mapping stay in the adapter path |
| Gateway owns device/series registry, calibration, and local custody | D2; Site Server cannot reinterpret raw device bytes |
| YokaKit owns part/process/production-result business data | D2; `production` is outside this slice |
| Gateway R9 includes typed derived measurements and bounded local rules | Responsibility ledger; names are split below, implementation deferred |
| R10 is a canonical record stream with target cursor and at-least-once delivery | D7; Site storage is idempotent by global record identity |
| First exit binding is MQTT QoS 1 | D9 and explicit 2026-07-13 user decision |
| Site Aggregator/projections are non-custodial | D8; projection progress cannot authorize purge |
| Archival Store commit and accepted-through cursor are atomic | D8/D9; PUBACK follows this transaction |
| Per-Gateway, per-target credential and TLS pinning are required | D10 |
| User chooses managed overlay VPN for the home deployment | 2026-07-13 Red decision; D10 is amended with a profile rather than a platform ban |

### User-value judgments

- A real end-to-end IoT path is more valuable now than Plan 6.5 snapshot hardening.
- MQTT is the correct first IoT exit protocol; HTTPS is not the selected production exit binding.
- The home environment uses the user's tailnet. The platform describes and validates the chosen
  trust profile but does not ban externally managed VPNs.
- CLI and API are sufficient; no UI is needed for first use.
- One real Gateway is enough initially; multi-Gateway correctness is simulated.
- OPT3001 comes before BravePI and production-count compatibility.

### Implementation choices fixed by this design

- Same repository, independently classified tier-[3] crates; Site crates do not depend on Gateway
  internals.
- A small transport-neutral egress-contract crate owns public record/batch/ack types.
- The Site Server uses SQLite with `WAL` and `synchronous=FULL` for custody-critical transactions.
- The query projection is rebuildable and advances separately from the archival cursor.
- Versioned `series_definition` records replicate the minimal R11 series semantics needed for a
  self-contained Site query; subject/business labels and application masters remain excluded.
- The local Site CLI is the construction/admin surface. The read API uses an OS-authorized local
  Unix socket; anonymous loopback TCP is not a measurement-read surface.
- Enrollment uses a one-use ticket and pinned box-key mTLS. Site stores credential verifiers only.

### Unverified platform facts

- The exact Tailscale CLI/status fields available on both machines must be reality-checked during
  planning. The product contract cannot depend on an unstable text rendering.
- The selected Rust MQTT implementation must prove manual PUBACK timing, bounded inflight windows,
  session control, TLS client-certificate authentication, and continued keepalive processing while
  the commit pipeline is backpressured. Library selection is a plan task, not assumed here.
- Raspberry Pi `synchronous=FULL` and Site Linux sustained commit rates require measured acceptance;
  no throughput number is asserted in this design.

## 3. Component vocabulary and ownership

| Component | Ownership |
|---|---|
| Driver | Physical I/O/protocol and datasheet-defined conversion to physical quantities |
| Adapter runtime | Scheduling, lifecycle, and `measurement_key + channel` mapping |
| Ingest client | Envelope identity, delivery, retry, and ingest ack handling |
| Adapter | Composed device-facing accountability unit; “device adapter” is explanatory wording only |
| Collector | Identity/series resolution, validation, dedup, quarantine, durable acceptance, and outbox enqueue |
| Derived-series processor | Deterministic, revision-bound generation of new measurement series; conceptual only in this MVP |
| Local rule evaluator | Bounded evaluation producing typed action intents, never direct I/O; conceptual only in this MVP |
| Publisher | Reads the R10 outbox and owns target batching, retry, cursor/delivery state |
| Egress binding | MQTT connection and wire mapping; owns neither application meaning nor custody authority |
| Archival Store | Durably stores canonical records and accepted-through cursor; sole custody-ack authority |
| Query projector | Rebuildable Site-side index for API/CLI searches; non-authoritative |
| Application projection adapter | Consumer-side application transform such as YokaKit; deferred and non-custodial by default |

These names are responsibility boundaries, not a requirement to create symmetrical crates. In
particular, no R9 crate is created for the OPT3001 slice.

## 4. Repository and dependency structure

The repository expands from a tier-[2]-only product to contain separately deployable tier-[3]
software. Layer checks distinguish deployment tiers.

Minimum new public units:

| Unit | Purpose | Allowed dependencies |
|---|---|---|
| `iotkit-egress-contract` | Serde-only canonical R10 record, batch, topic/ack detail, versioning | Serde/value types only; no Gateway/Site storage |
| `iotkit-site-store` | Enrollment registry, raw archive, accepted-through cursor, query projection/checkpoint | Egress contract + storage primitives; no Gateway core |
| `iotkit-site-server` | MQTT listener, local Unix-socket read API, composition root | Egress contract + site store |
| `iotkit-site-serverctl` | Local construction/enrollment/admin CLI and read CLI | Site API/admin interface; no direct mutation SQL |

Existing Gateway `core/publish` remains the outbox/target state owner but is migrated from
HTTPS-specific endpoint/token fields to transport-neutral MQTT target state. The Gateway publisher
uses `iotkit-egress-contract`; the Site Server never depends on `iotkit-gateway`, `core/timeseries`,
`core/publish`, or adapter crates.

`scripts/check-layers`, the crate map, and the placement table change in the same implementation
unit as new crates. A new SITE classification may depend on CONTRACT/TYPE/STORAGE primitives but not
on Gateway composition or adapter layers.

## 5. Data flow

```text
OPT3001
  -> driver: register bytes to lux
  -> adapter runtime: illuminance_lux + channel
  -> ingest client
  -> collector durable transaction
       - reading
       - publication_log row
  -> branches
       - Gateway R11 local query
       - publisher -> MQTT egress binding -> Site custody listener
            -> one Site durable transaction
                 - raw canonical record(s)
                 - payload fingerprint / dedup identity
                 - per-Gateway accepted-through cursor
            -> PUBACK + accepted-through detail
            -> asynchronous query projector
            -> Site read API / CLI
```

The Gateway's source observation is not overwritten. The Site raw archive preserves the exact
canonical R10 record. The projection is a read optimization, not a second authority.

## 6. Canonical R10 MQTT contract

### Namespace and ACL intent

The exact canonical spelling is versioned in `iotkit-egress-contract`; the design-level shape is:

```text
iotkit/v1/gateways/{gateway_identity}/records
iotkit/v1/gateways/{gateway_identity}/ack-detail
iotkit/v1/gateways/{gateway_identity}/terminal-notice
iotkit/v1/gateways/{gateway_identity}/resync-request
iotkit/v1/gateways/{gateway_identity}/series-snapshot
iotkit/v1/gateways/{gateway_identity}/series-snapshot-ack
iotkit/v1/enrollment/{enrollment_id}/request
iotkit/v1/enrollment/{enrollment_id}/response
```

- Gateway may publish only its `records`, `resync-request`, and bootstrap `series-snapshot` topics and subscribe only to its
  `ack-detail`, `terminal-notice`, and `series-snapshot-ack` topics.
- For production records, a pending enrollment attempt may publish only the exact contiguous range
  `(accepted_through + 1)..=smoke_pub_seq`, bound to that attempt's target,
  endpoint, and epoch. Count, byte, inflight, time, and retry-rate limits apply. It may not publish a
  later sequence or another namespace. It may also use only the same attempt's bounded enrollment
  status, resync, and required legacy snapshot control topics; none carries purge authority.
- Active operational credentials cannot use another Gateway namespace.
- No wildcard application publish/subscribe, broker bridging, retained application message, or
  device-to-device routing is provided.
- Enrollment request/response topics are non-retained, bounded, and available only on a mutually
  pinned TLS connection whose client fingerprint matches the ticket. HTTPS is not introduced as a
  second data or enrollment protocol.

### Batch identity

Each bounded batch contains:

- contract/schema version;
- `gateway_identity`, `ledger_epoch`, logical `target_id`, and target endpoint identity;
- deterministic `publication_id`;
- contiguous `cursor_start..cursor_end` in publication order;
- canonical records with original sequence/event-time/source metadata;
- payload hash material required for same-key/different-content detection.

At-least-once retry reuses the exact publication identity/content. Site idempotency key is the
Gateway-scoped record identity; the same identity with different content is a fence-worthy integrity
incident, never last-write-wins.

Before first publish, Gateway durably stores a non-secret delivery-attempt descriptor and the exact
canonical encoded payload blob. The descriptor contains the
binding generation, target endpoint identity, epoch, cursor range, publication ID, payload
fingerprint, contract version, and versioned batch-policy identity. Restart or configuration change
must replay that descriptor and its exact payload unchanged until formal cursor advancement or
terminal quarantine. A new descriptor may be formed only after the prior attempt reaches one of
those states.

Before the first measurement of a series, and before any changed definition becomes effective, the
Gateway emits a mandatory `series_definition` record at an earlier publication sequence. It carries
only `series_key`, definition revision/effective sequence, measurement key, unit, value type, channel,
series variant, value semantics, and registry revision. Definition history is immutable. Site code
does not parse meaning from the opaque `series_key` spelling.

Existing retained HTTPS-era outbox rows predate `series_definition` and cannot be renumbered or
silently reordered. Contract-v1 bootstrap therefore has one explicit metadata-snapshot transition:

- In `CUTOVER_FROZEN`, Gateway creates a canonical, hashed `series_definition_snapshot` containing
  every definition needed by retained outbox rows plus `snapshot_through_pub_seq` and registry
  revision. It is immutable for that cutover attempt.
- Site durably stores and validates the snapshot before accepting the first MQTT production batch and
  returns a correlated snapshot acknowledgement. The snapshot is target bootstrap metadata, not a
  production cursor and cannot authorize purge.
- Measurements at or below `snapshot_through_pub_seq` resolve against this baseline. New/changed
  definitions after the snapshot use ordinary earlier-pub-seq `series_definition` records.
- If historical definition changes for retained rows cannot be reconstructed from Gateway registry/
  audit state, cutover stops for operator resolution; it does not apply the current definition
  retroactively. Already-purged, unavailable history remains an explicit `archive_lost` baseline.

This is the only legacy ordering exception. New publication streams must satisfy definition-before-
measurement ordering from their first record.

Snapshot acknowledgement uses the non-retained `series-snapshot-ack` topic and is an idempotent,
tagged contract distinct from production `ack-detail`. It carries `schema_version`, `snapshot_id`,
snapshot hash, `gateway_identity`, `target_id`, `target_endpoint_id`, `ledger_epoch`,
`snapshot_through_pub_seq`, `registry_revision`, and the literal `purge_authority=false`. After
SUBACK, the ordinary resync request includes the local snapshot ID/hash; Site returns the stored
snapshot state as well as the production watermark so response loss is harmless. Transient storage,
timeout, or overload failure emits no acknowledgement and closes/retries under the same reversible
failure rule as production. A deterministic invalid/conflicting snapshot emits a tagged terminal
notice with `object_kind=series_snapshot`, snapshot ID/hash, and Gateway/target/endpoint/epoch
correlations; production-only publication/range fields are absent in that variant. It leaves the
production cursor unchanged and keeps cutover purge-held.

### Ack and custody

For a gap-free valid batch, the Site Server:

1. authenticates the Gateway and namespace before accepting payload bytes beyond configured bounds;
2. validates contract version, epoch, contiguous range, record identity, and finite/value rules;
3. begins one `IMMEDIATE` SQLite transaction;
4. inserts/idempotently verifies every canonical record;
5. advances that Gateway's accepted-through cursor to the contiguous committed prefix;
6. commits under `WAL + synchronous=FULL`;
7. only then emits PUBACK and the formal `accepted_through` detail.

The ack detail correlates `gateway_identity + target_id + target_endpoint_id + ledger_epoch +
publication_id`; omission or mismatch prevents Gateway cursor advance.

ENOSPC, corruption, SQL error, cancellation before commit, fingerprint collision, invalid gap, or
partial durable storage returns no success PUBACK. Network loss after commit causes harmless replay.
The query projection is never part of the custody transaction.

The listener continues MQTT network/keepalive processing while the bounded commit queue is full.
It applies bounded inflight windows and backpressure without converting a reversible overload into a
terminal rejection or dropping a committed Gateway record.

### Terminal, gap, resynchronization, and smoke

- Queue saturation before a commit attempt: keep MQTT network/keepalive processing alive, stop
  admitting new work, and delay PUBACK until bounded capacity returns.
- Once an actual transient commit attempt fails: no PUBACK and no terminal notice; after bounded
  cleanup Site explicitly closes the connection. Gateway reconnects with a clean session and
  republishes the exact batch from the outbox. Persistent ENOSPC repeats this visible disconnect/
  retry state without deleting or terminally rejecting the record.
- Deterministic contract violation or payload fingerprint conflict: no PUBACK; Site publishes a QoS1
  non-retained terminal notice containing all target/publication correlations and a stable reason
  code. Gateway quarantines (does not delete) that outbox batch and reconnects.
- After a terminal gap, later PUBACKs release only the MQTT inflight window. They are not cumulative
  purge evidence. Formal `accepted_through` remains stopped before the gap until the quarantined
  batch is corrected/replayed and the contiguous prefix catches up.
- Connections use clean sessions; the outbox is the retry authority. Gateway subscribes and waits for
  SUBACK on ack/terminal/snapshot-ack topics, then sends a correlated resync request. Site responds
  with the current formal watermark/terminal/snapshot state only after subscription is ready.
  Retained messages are forbidden.

The version-1 production-record terminal variant fields are `schema_version, gateway_identity, target_id,
target_endpoint_id, ledger_epoch, publication_id, cursor_start, cursor_end, reason_code,
diagnostic_id`; the snapshot variant is defined above. Neither contains a secret or raw payload.
Stable terminal reasons are
`contract_major_unsupported`, `target_binding_mismatch`, `epoch_conflict`,
`publication_identity_mismatch`, `cursor_range_invalid`, `record_schema_invalid`, and
`payload_conflict`. Storage/timeout/overload errors are forbidden from this list. Resync request/
response carries a fresh `request_id` plus all target/epoch correlations and the Gateway's local
watermark plus local snapshot ID/hash; Site returns formal `accepted_through`, stored snapshot state,
and any active terminal gap for that target.

An absent, late, conflicting, or invalid required series definition is
`record_schema_invalid`: deterministic terminal notice, no custody ack, and quarantine of the
Gateway outbox batch. Site validates type/unit/value against the applicable immutable definition
before raw custody commit. “Projection incomplete” is reserved for projector failure/corruption after
a previously valid custody commit; it is not an escape hatch for accepting uninterpretable data.

Commissioning smoke is a synthetic record family in the normal `records` stream with a real
production publication sequence. Enrollment records its expected smoke sequence. A pending target may
deliver the contiguous backlog through that sequence using the real custody pipeline, but Gateway
purge remains disabled. Activation requires formal `accepted_through >= smoke_pub_seq`; smoke cannot
skip an older backlog or use a separate cursor. The synthetic row remains in raw archive and is
excluded from ordinary measurement queries.

Library selection begins with a go/no-go spike. A candidate must prove application-controlled
post-commit PUBACK, explicit server disconnect after commit failure, bounded inflight/queue behavior,
keepalive independence from the commit worker, TLS client-certificate verification, and clean-session
outbox replay. A normal broker/listener API that acknowledges before the application commit is
ineligible.

## 7. Site archive and query projection

### Raw authority

The raw archive stores immutable canonical records. Raw record identity remains target-independent:
`(gateway_identity, ledger_epoch, publication_seq)` plus a payload fingerprint. Formal delivery state
is stored separately under `(gateway_identity, target_id, target_endpoint_id, ledger_epoch)`. Raw bytes/payload and
the parsed canonical fields needed for validation are retained so contract evolution remains
inspectable.

`target_id` is the immutable logical consumer relationship. `target_endpoint_id` is an immutable
endpoint identity within it; address/pin rotation creates a new binding revision and must prove the
prior contiguous watermark before inheritance. A replacement target gets a new target ID and starts
from a proven import/backfill baseline, never an inherited cursor by name. A unique state constraint
permits exactly one ACTIVE archival binding per Gateway in the MVP.

### Projection

The projector reads each Gateway's committed raw records in its own `(epoch, pub_seq)` order and
maintains query tables for:

- Gateway identity and epoch;
- stable series identity and measurement key;
- event time, received time, time quality;
- typed value/unit and data-quality flags;
- raw-record locator and projector checkpoint.
- immutable `series_definition` history and effective revision.

Projection and each per-Gateway checkpoint commit atomically with each other, but independently of archival ack. A
projection failure leaves raw/custody intact. API responses include the projector watermark and an
`incomplete` indication plus a per-Gateway watermark vector when projection lags archival cursors.
`site-serverctl projection rebuild`
deletes/recreates only derived tables and replays raw records.

No aggregate/OEE/production table or business/subject label is part of this slice.

### Raw retention, capacity, and backup

- Raw archival deletion is unsupported in the MVP. Projection rebuild/cleanup cannot delete raw rows.
- Site health exposes filesystem free space, measured ingest rate, estimated exhaustion horizon,
  last successful backup/verification, raw row/byte growth, and per-Gateway lag.
- Commissioning rejects a declared capacity that cannot hold the selected outage/backup safety
  horizon. Thresholds use conservative configured defaults and measured smoke throughput; this design
  claims no universal production rate.
- `site-serverctl backup create` produces a bounded, internally consistent official backup artifact
  with manifest/hash/schema/site/export marker and Site TLS recovery material. It never creates a
  live-startable DB copy. The fixed allowlist of regular files plus an authenticated inner manifest
  is encrypted as a standard age-v1 passphrase-recipient envelope using the reviewed Rust `age`
  implementation; no custom cipher/KDF is designed here. The envelope/profile version and encoded
  KDF parameters are recorded and resource/total-size limits are checked before extraction.
- Backup/restore recovery secrets enter only through non-echo stdin or an already-open safe owned
  descriptor/mode-0600 regular file; argv, environment, symlink, logs, audit details, and shell history
  are forbidden. Output uses an O_EXCL mode-0600 temp file in the destination directory, file fsync,
  atomic rename, directory fsync, and best-effort temp cleanup on failure. Import extracts only the
  manifest allowlist into a private staging directory and rejects links/path traversal/extra entries.
- Restore verification is part of acceptance: import into a fresh generation, reconcile each
  Gateway, rebuild projection, and record missing acknowledged/purged ranges as `archive_lost`.

### Read API and CLI

The initial measurement-read API uses a root-owned Unix socket, mode 0660, with OS peer credentials
and an explicit `iotkit-site-readers` group. It supports bounded queries by Gateway,
series/measurement, and event-time range, plus archival/projector watermark status. Unbounded
full-history queries are rejected. `iotkit-site-serverctl query` calls the same API and inherits OS
group authorization; it does not mutate the database directly. Anonymous loopback TCP may expose a
non-sensitive health summary only, never measurements.

Construction mutations use a root-owned local Unix socket with OS peer credentials/permissions and
typed audited dispatch. `iotkit-site-serverctl` never opens the Site SQLite file for mutation. The
network-facing listener has no construction/admin route.

Network-exposed measurement-read API and its operator-token policy are a later explicit opt-in. The MQTT listener
alone binds the selected overlay address in this deployment.

## 8. Path profiles: platform choice, site decision

D10 supports two declared path profiles:

### `self_managed_static`

Per-Gateway static WireGuard-equivalent peer, locally generated keys, constrained routes/firewall,
explicit peer rotation/revocation, and no external control-plane authority.

### `managed_overlay`

A site-selected managed overlay VPN. The platform does not prohibit this profile. Configuration
records:

- provider/profile identifier and administrative authority;
- node-admission and account-recovery owner;
- intended Gateway and Site node identities;
- bind interface/address and allowed destination ports;
- ACL owner and last verified/attested revision/time;
- node/key rotation and revocation procedure;
- control-plane outage/compromise consequences;
- restore/re-enrollment procedure.

The first deployment uses `managed_overlay` with the user's tailnet. The Site MQTT listener binds its
tailnet address only; loopback remains available for local health/admin. Host firewall and overlay ACL
restrict Gateway-to-Site reachability to the MQTT/enrollment port. Subnet routing or Site-to-home-LAN
reachability is not required by IoTKit and is not enabled by this profile.

The overlay is defense in depth and reachability. It never replaces MQTT TLS pinning, box-key mTLS,
per-Gateway credentials, namespace ACLs, or application audit. If overlay configuration cannot be
machine-verified, commissioning reports `operator_attested` rather than claiming verified security.

## 9. Enrollment and credential state machine

### Secret provenance

| Material | Generator / first knower | Transfer | Stored form | Revocation/restore |
|---|---|---|---|---|
| Gateway identity | Gateway first boot | Public | Plain identifier | Preserved by R22; epoch changes on formal restore |
| Gateway box/TLS private key | Gateway first boot | Never leaves except encrypted R22 snapshot | Gateway secret | Registry revocation; epoch fence on restore |
| Gateway public fingerprint | Gateway | Human comparison + pinned mTLS | Public registry field | Explicit recovery only |
| Site TLS private key | Site Server | Only inside the encrypted official backup/export | Site secret | Restore requires local import and independent pin confirmation; rotation is a construction operation |
| Site TLS pin | Site Server | Ticket + independent human display | Public pinset | Explicit pinset rotation |
| Enrollment ticket secret | Site Server CSPRNG | Protected stdin or owned mode-0600 file | Salted verifier only | Single use/TTL/tombstone; invalid after Site restore |
| MQTT operational credential | Site Server CSPRNG | Once over mutually pinned TLS | Gateway plaintext; Site verifier only | Two-slot rotate/revoke/session disconnect; excluded from Gateway snapshot |
| Managed-overlay credentials | Overlay provider/endpoints | Product external | Product records metadata, never provider secret | Site's declared overlay procedure |

### CLI construction operation

The installer first runs `gatewayctl identity show`. On the Site machine, one combined local
construction operation displays and audits every effect before confirmation:

- site ID;
- Gateway identity, epoch, and box-key fingerprint;
- target ID and endpoint ID/address;
- Site TLS pin/pinset;
- path profile/provider;
- MQTT namespace/scope;
- archival-responsible designation; and
- ticket TTL.

After affirmative confirmation, `site-serverctl gateway enroll issue` emits a versioned ticket once.
Ticket input to `gatewayctl target enroll` is non-echo stdin or an owned regular mode-0600 file.
Symlinks, unsafe owner/mode, oversized input, command-line secrets, and unknown fields/version are
rejected. Before connection, Gateway displays non-secret site/endpoint/pin/target/archive effects for
confirmation. The installer compares the displayed Site pin with the value obtained independently on
the Site machine in step 3 of the installation journey; trusting only the pin inside the transferred
ticket is insufficient because whole-ticket substitution must remain detectable.

### States

```text
ABSENT
  -> TICKET_ISSUED
  -> PENDING_DELIVERY(attempt_id, verifier, activation_deadline)
  -> PENDING_SMOKE
  -> ACTIVE(slot A)
  -> RENEWAL_DUE -> ROTATING(slot A active, slot B pending) -> ACTIVE(slot B)

ticket/pending -> TERMINAL(expired|revoked|clock|failure)
active -> REVOKED
active -> FENCED(restore|epoch|clone|integrity conflict)
```

Terminal states retain non-secret tombstones for audit/replay defense.

### Redemption and response loss

1. Gateway pins the Site certificate from the independently compared ticket and presents its box
   certificate/key through mTLS.
2. Site requires exact ticket hash, identity, epoch, box fingerprint, endpoint, pinset, scope, and
   path-profile binding. Concurrent redemption uses a unique/CAS transition.
3. Site generates credential slot A, commits only its salted verifier as pending, and returns the
   plaintext once over mTLS.
4. Gateway durably stores it before requesting commissioning smoke.
5. If the response was lost and Gateway lacks the credential, box-key-authenticated status returns
   only pending metadata. Gateway requests CAS replacement; Site invalidates the old verifier, creates
   a new attempt/credential/verifier, and returns the new plaintext once. No credential ciphertext or
   recoverable bearer plaintext is stored at Site.
6. Pending credential ACL permits only the exact contiguous production range
   `(accepted_through + 1)..=smoke_pub_seq` for its bound target, endpoint,
   and epoch, under the limits in section 6. The synthetic record traverses the real custody
   validation and durable transaction but is marked commissioning data and excluded from user
   measurement queries. Later sequences and every other namespace are rejected.
7. The same custody transaction that first advances formal `accepted_through` across the expected
   smoke sequence atomically activates the pending enrollment/slot. PUBACK/status follows that commit.
   A lost response is recovered by authenticated status query; it does not mint another slot.

Ticket expiry uses Site wall time plus an in-process monotonic deadline. Site persists one
nondecreasing authentication-time high-water mark shared by ticket issue/redemption, operational
authentication, renewal, and replacement. A backward jump suspends new issue/rotation/replacement,
terminally expires outstanding tickets, and prevents expired credentials from becoming valid again.
Live sessions use monotonic remaining lifetime and are disconnected at the original expiry; after a
restart with unresolved rollback, bearer authentication fails closed until a local audited clock
recovery. Box-key authentication may identify the Gateway for recovery but cannot bypass the clock
gate or mint a new operational expiry while time remains untrusted. A
forward jump may expire early, never extend. Redemption locks the ticket and starts a distinct
bounded activation deadline.

Operational credentials carry `issued_at`, `expires_at`, slot, credential generation, and a
`renewal_at` safely before expiry. TTL is no shorter than the declared path outage tolerance times the
D10 safety factor. Gateway schedules renewal using monotonic elapsed time from receipt and Site
validates wall-clock expiry. The minimum automatic loop performs box-key-mTLS reissue into slot B,
delivers through the real custody path, activates B only after smoke, then disconnects/revokes A.
Failure leaves A active and raises `credential_health`; renewal begins before expiry. Fleet
campaign/canary orchestration remains deferred, not per-Gateway renewal.

For rotation, the smoke-crossing custody transaction atomically activates B, prevents A from new
authentication, and writes a durable `disconnect_old_slot` effect while preserving B as valid. Live
A-session disconnect is an external effect executed only after commit and retried from that effect
record after restart. Crash before commit leaves A active/B pending; crash after commit leaves B
active and the disconnect effect pending. No crash boundary revokes both slots or activates across an
older terminal gap.

Ordinary revocation uses the same crash boundary: one transaction invalidates the verifier and
credential generation and enqueues an idempotent `disconnect_revoked_slot` effect. Every PUBLISH is
authorized against the current slot generation, not merely the state captured at CONNECT, so a crash
before physical disconnect cannot continue accepting records. The listener retries the disconnect
effect after restart. Managed-overlay node removal may be scheduled as an additional external effect,
but local application revocation commits and remains effective even when that provider is unavailable.

Gateway durably binds every received credential to `enrollment_id/attempt_id`, target, slot, and
credential generation before smoke. Delayed responses from invalidated attempts/generations are
discarded and audited; they cannot replace a newer pending or active slot.

### Restore and clone fences

Site has a root-owned generation anchor and restore marker outside the DB/ordinary DB backup. Network
listeners bind only when the live DB's `site_id + instance_generation` matches the anchor and no
restore marker exists. Official backup artifacts are non-live and can be imported only by a
local-root typed operation into a newly generated instance. The operation stages and verifies the
artifact, fsyncs the marker, invalidates restored tickets/pending/verifiers/sessions/clock state,
commits the new DB generation, fsyncs the external anchor, then clears the marker. Any interruption or
mismatch remains network-unbound.

After restore, each Gateway authenticates with its box key and reconciles active epoch plus its
effective cursor/outbox retention range against contiguous raw Site data before new acks. Recoverable
ranges enter `archive_repair_hold` and backfill; acknowledged ranges already purged at Gateway and
missing from backup/raw become `archive_lost` with operator confirmation. Site TLS key/pin recovery is
also explicitly confirmed. A root-level raw DB replacement or full-disk rollback that also restores
the external anchor is outside the guarantee; preventing it requires an external witness/nonrollback
hardware and remains deferred.

Formal Gateway restore follows D2: new epoch and old-machine invalidation. Observable concurrent
same-key sessions, repeated lease oscillation, or same record identity with different payload fences
delivery and requires local operator recovery. An exact byte clone used only after the original is
offline is indistinguishable without a non-clonable key/external witness; the MVP does not claim to
prevent it. Ordinary connection loss/retry with one active lease does not create a clone incident.

## 10. State and failure behavior

### Gateway-to-Site delivery

```text
DISCONNECTED -> CONNECTING -> SUBSCRIBED/RESYNCED -> DELIVERING
     ^              |                    |               |
     +---- backoff <-+--------- failure <-+---- retry <---+

DELIVERING -> QUARANTINED_GAP -> RECONNECTING -> DELIVERING(window-only ack)
DELIVERING -> FENCED on identity/epoch/payload conflict
```

Gateway outbox cursor advances only from a valid formal accepted-through detail for the current
target/epoch. PUBACK alone is cumulative-equivalent only under the D9 gap-free rules; the explicit
detail remains the inspectable authority.

### Projection

```text
CAUGHT_UP -> LAGGING -> CAUGHT_UP
     |          |
     +-> FAILED -> REBUILDING -> LAGGING
```

Projection state never changes Gateway custody or Site raw retention.

### Site/Gateway outage

- Site or tailnet down: Gateway continues local collection/query and retains unacknowledged outbox.
- Gateway down: Site serves already archived records and reports stale Gateway watermark.
- Site ENOSPC/corruption: no PUBACK; Gateway retries and local backlog grows visibly.
- Gateway ENOSPC: ingest does not falsely acknowledge; existing data remains visible; degradation
  follows R17 and never silently discards unacknowledged custody without its required incident path.
- Projector failure: archival ingestion/ack continues; query reports lag/incomplete.
- Site capacity warning/critical: raw data is not deleted; operator expands storage or creates and
  verifies backup/migration through the typed operation before ENOSPC.

## 11. User journeys and step counts

### First installation: one Gateway

1. Install/start Site Server locally.
2. Configure/verify the selected managed-overlay node, bind address, firewall, and ACL attestation.
3. Read Site TLS fingerprint locally.
4. On Gateway, show identity/epoch/box fingerprint.
5. On Site, issue the combined enrollment construction operation and compare/confirm fields.
6. Transfer one short-lived ticket through protected stdin/file.
7. On Gateway, review non-secret effects and redeem.
8. Observe custody smoke success/ACTIVE at both CLIs.
9. Enable/configure the direct-I2C OPT3001 adapter if not already enabled.
10. Query one lux record locally on Gateway.
11. Query the archived record with `site-serverctl`.
12. Disconnect/reconnect the overlay and verify catch-up watermarks.
13. Create and verify one encrypted official Site backup.
14. Observe capacity/risk-horizon and credential-health status.

Secrets manually handled: one short-lived ticket and the operator-chosen backup recovery secret.
Operational MQTT credentials are short-lived and never displayed.
Long strings compared: Gateway box fingerprint and Site TLS fingerprint, displayed in grouped form.
No browser, certificate-warning bypass, public port, or repeated per-sensor credential step.

For 20–100 Gateways this manual flow is intentionally not claimed efficient; fleet enrollment is a
named later deliverable. The first slice proves one real Gateway and simulated concurrent identities.

### Routine query

1. Run bounded `site-serverctl query` on the Site machine.
2. Inspect data plus archival/projector watermarks.

No secret entry is required for local read-only CLI use.

### Site recovery from official backup

1. Start local-root recovery; listeners remain network-unbound.
2. Select the owned mode-0600 backup and enter the recovery secret through non-echo stdin.
3. Verify age envelope, authenticated manifest/hash/schema/site/export marker, and resource limits.
4. Stage/import into a fresh Site generation; invalidate restored tickets, pending attempts,
   operational verifiers, sessions, and old clock state.
5. Display and independently confirm the recovered/new Site TLS pin.
6. Reauthenticate each Gateway by box key, reconcile epoch, and issue fresh operational credentials.
7. Compare each Gateway cursor/outbox retained range with contiguous raw Site data; apply
   `archive_repair_hold` and backfill where possible.
8. Require explicit operator confirmation for any unrecoverable `archive_lost` range.
9. Rebuild query projection and verify per-Gateway archival/projector watermarks.
10. Clear recovery mode and bind listeners only after the generation anchor matches.

Manual secret entries: one backup recovery secret. Long strings: Site pin plus affected Gateway
identity/epoch summaries. Expected downtime lasts through steps 1–10; no archival ack is returned
during recovery, while Gateways continue local collection within their retention capacity.

## 12. Adversarial-six and extended failure analysis

| Scenario | Mechanism and outcome |
|---|---|
| Two identical ticket issues/redemptions | Local issue operation has unique request/audit ID; redemption is DB-unique/CAS. Same exact attempt is idempotent; different identity/key is rejected and audited. |
| Power loss immediately before/after commit | Before commit: no success/PUBACK and retry. After commit before response: exact replay/status observes committed state. Gateway writes credential before smoke. |
| Commit succeeds but response is lost | Delivery replays same publication; Site idempotently verifies. Enrollment uses status/CAS replacement or ACTIVE status, never stored recoverable secret. |
| Clock unset/jumps/rolls back | Gateway event time retains quality. Site's shared auth-time high-water prevents ticket or credential TTL extension, expired-slot revival, and new issue/rotation; live expiry stays monotonic. Restart fails closed until audited time or box-key recovery establishes a new generation. |
| Old backup restored to another box | Gateway formal restore changes epoch; Site restore closes listeners, invalidates tickets/pending, distrusts credentials, and reconciles each box key/epoch before fresh credential. |
| Same-LAN/tailnet third party races owner | Ticket is identity/key-bound, mTLS required, short-lived and rate-limited. Wrong key cannot redeem; attempts audit without secret material. |
| Site ENOSPC/DB corruption | No custody transaction commit, no PUBACK, no cursor movement; health becomes unhealthy and Gateway retains data. |
| Network partition | Gateway retries with jitter and keeps local custody; Site reports stale watermark; reconnection replays. |
| Credential response loss | Pending status then box-authenticated CAS replacement; old verifier invalidated; no plaintext retained server-side. |
| Exact Gateway clone | Concurrent/oscillating lease or payload collision raises clone suspicion and fences delivery. A byte clone used only after the original is offline is indistinguishable without non-clonable hardware/external witness; the MVP states this limitation. |
| Projection corruption | Drop/rebuild projection from raw archive; custody and Gateway cursor unchanged. |
| Old Site backup copied/restored | Official backup is non-live; generation/restore-marker mismatch keeps listeners unbound. Typed import creates a fresh generation, invalidates credentials, and reconciles every Gateway. Full-disk/root rollback including the anchor is an explicit residual risk. |
| Managed-overlay control-plane compromise | Attacker still faces TLS pin, box mTLS/credential, namespace ACL; availability/metadata exposure and node admission remain declared residual risks handled by provider/site revocation. |

## 13. Traceability

| Invariant | Mechanism | Failure behavior | Verification |
|---|---|---|---|
| No silent data loss | Gateway durable ingest + outbox; Site raw+cursor atomic commit | No ack on storage failure; retry/backlog | fault-injected SQL/ENOSPC and response-loss tests |
| Only archive ack permits purge | `archive_responsible` target and formal accepted-through | Projection/PUBACK misuse cannot advance custody cursor | negative cursor/purge integration tests |
| Same identity cannot overwrite different data | record key + payload fingerprint | fence and alarm | collision test with equal key/different payload |
| Secrets stay secret | protected input, redacted types, verifier-only Site storage | operation fails closed before logging unsafe data | Debug/log/audit snapshot tests and repository secret-field scan |
| Enrollment is owner-approved | combined R14 construction display/confirmation + exact binding | no ticket before confirmation | CLI golden/negative tests |
| Lost enrollment response is recoverable | mTLS status + verifier CAS replacement | old pending verifier revoked; new one returned once | crash/response-loss state-machine tests |
| Authentication time cannot extend backward | monotonic deadlines + shared durable auth-time high-water | issue/rotate/replace suspended, tickets terminal, expired slots never revive, bearer restart fails closed | ticket/active/expired/rotating rollback and restore tests |
| Old Site DB cannot bind as live | external generation anchor + non-live backup marker + restore-in-progress protocol | network-unbound recovery mode | old-backup/raw-copy/interrupted-restore tests |
| Short-lived credential renews safely | monotonic renewal_at + box-mTLS two-slot rotation + real custody smoke | old slot stays active; credential_health alarm | expiry/partition/delayed-response tests |
| Revocation survives crash/provider outage | verifier/generation invalidation + durable disconnect effect + per-PUBLISH generation check | local rejection is immediate; physical/provider effects retry | crash-before/after-disconnect and overlay-outage tests |
| Projection cannot claim custody | separate DB transaction/checkpoint | query lags but archive acks continue | projector failure and rebuild tests |
| Site query has stable semantics | mandatory immutable series_definition history | missing/late/conflicting definition gets terminal no-ack; valid raw remains queryable if projector later fails | reorder/replay/revision tests |
| Raw archive is not silently exhausted/deleted | no-delete policy + capacity horizon + encrypted verified backup | alert, then no-PUBACK at ENOSPC | fill/backup/restore/archive_lost tests |
| App vocabulary stays outside core | canonical measurement contract; projection adapter boundary | unknown/app field rejected or ignored by version rules | contract schema and layer tests |
| Overlay is not app authentication | TLS pin + mTLS + MQTT credential and ACL | overlay admission alone cannot publish | unauthorized same-tailnet connection tests |
| OPT3001 remains useful without Site | Gateway local R11 branch | Site outage affects only upstream delivery | real Pi disconnect/reconnect acceptance |

## 14. Verification strategy

### Contract and unit tests

- Golden canonical batch/ack-detail/terminal/resync/series-definition/snapshot-ack encodings and
  version rejection, including `purge_authority=false` and snapshot response-loss resync.
- OPT3001 adapter normalization proves the driver's internal `"lux"` label is emitted canonically as
  measurement key `illuminance_lux` with UCUM unit `lx`; known register vectors cover conversion.
- Topic/namespace ACL and client-ID binding.
- Batch range, finite value, fingerprint, epoch, and size bounds.
- Enrollment parser limits, file ownership/mode/symlink rejection, redaction, verifier comparison,
  ticket CAS, activation deadline, shared auth-time high-water, tombstones, credential replacement,
  attempt/slot/generation correlation, expiry, automatic renewal, and durable revocation effects.
- Raw archive idempotency and same-key/different-payload fence.
- Projection replay/rebuild and watermark/incomplete behavior.
- Site generation-anchor/export-marker/restore transaction and no-delete capacity-health behavior.

### Integration and fault tests

- Real MQTT QoS 1 client/listener with manual post-commit PUBACK.
- Power-loss/connection-loss boundaries around raw+cursor commit and enrollment transitions.
- ENOSPC/SQL failure/corruption injection: no PUBACK/no activation/no cursor advance.
- Keepalive progresses while commit queue is saturated.
- Gateway retry reuses exact batch/publication identity.
- Restart or batch-limit/configuration change between publish and ack replays the persisted delivery
  attempt descriptor and byte-identical payload.
- Terminal gap forces reconnect, stops formal watermark, and restores cumulative equivalence only
  after the quarantined prefix is corrected.
- SUBACK-before-resync prevents clean-session loss of current watermark/terminal state.
- Commissioning smoke is a normal-stream sequence after any backlog and cannot activate or advance
  purge across an older gap.
- HTTPS-to-MQTT cutover pauses old publisher lease and purge, proves baseline, and rejects stale old
  binding responses.
- Site restart and Gateway restart converge without duplicate effects.
- Managed-overlay bind/route profile is inspected using stable structured provider output where
  available; otherwise the result is explicitly operator-attested.
- Simulated multiple Gateway identities prove independent cursors/ACLs and partial connectivity.

### Real-hardware acceptance

- Raspberry Pi direct-I2C OPT3001 reading in canonical lux.
- Read back the real OPT3001 configuration/device registers and compare a controlled-light reading
  with a known/reference range; Site `series_definition` must show `illuminance_lux`/`lx`.
- Home Linux Site Server on configured tailnet address.
- Disconnect/reconnect tailnet; verify zero acknowledged-record loss and eventual cursor catch-up.
- Query the same record locally at Gateway and from Site API/CLI with matching identity/value/time.
- Restart both processes and both machines at least once around a pending backlog.
- Verify one official backup/import into a fresh Site generation and one automatic credential renewal.
- Force Site time backward across expired, active, and rotating credential slots; verify no credential
  revives, live expiry remains monotonic, and restart fails closed until audited recovery.
- Prove the OPT3001 transport's exact configuration-register write bytes before real readback; the
  expected byte order must be fixed by the register contract, not inferred from a successful reading.
- Measure commit latency and bounded-memory behavior; report facts rather than claim a production rate.

Full `scripts/verify.sh` remains required at each implementation milestone, but broad unrelated stress
tests are not repeated when a change cannot affect them; verification follows the repository's
risk-proportionate rule.

## 15. Freeze now versus defer

### Freeze before implementation

- Component names/responsibility boundaries in section 3.
- MQTT contract version, topics, batch identity/range, ack/terminal/resync state, PUBACK timing, and ACL.
- Raw archive identity/fingerprint and raw+cursor transaction.
- Mandatory series-definition synchronization, query projection non-authority, and per-Gateway watermark vector.
- Managed-overlay trust-profile fields and residual-risk disclosure.
- Enrollment ticket schema/bindings/limits, pinned mTLS, hash-only storage, CAS response-loss recovery,
  custody smoke, activation, clock rollback, Site generation-anchor restore, observable-clone fences/
  exact-clone limitation, and tombstones.
- Two-slot credential schema, minimum automatic renewal, expiry, and revoke/session-disconnect semantics.
- No-delete raw retention, capacity horizon, official encrypted backup, import, and archive-loss reconciliation.
- CLI construction effects and secret-safe input/output.

### Defer without hiding the boundary

- Derived-series processor and local-rule evaluator implementation.
- BravePI count/reset/wrap/replacement semantics and pulse-to-product interpretation.
- Application projection adapter/YokaKit compatibility and `production` MQTT payload.
- Query aggregation beyond bounded raw measurement search.
- Network-exposed Site read API and web UI.
- Fleet rotation campaigns/canaries, fleet enrollment, decommission automation, multi-site/HA/cloud.
- Additional egress bindings, including HTTPS.
- Generic non-custodial broker targets and Site republishing; D9 compatibility remains a later
  deliverable and is not part of this custody-aware listener MVP.

## 16. Migration and rollback

The existing HTTPS publisher is transitional implementation, not a second MVP promise. Migration is
non-destructive and gives one binding generation an exclusive publisher lease:

```text
UNBOUND or HTTPS_ACTIVE
  -> CUTOVER_FROZEN
  -> MQTT_PROBING
  -> MQTT_ACTIVE
```

1. Introduce transport-neutral target state with `binding_generation`, logical target cursor, binding
   state, and exclusive lease alongside the old schema.
2. `CUTOVER_FROZEN` stops new HTTPS cycles, waits for the old lease/inflight request to finish, blocks
   purge, and captures the exact logical cursor/outbox horizon. It then audits every publication
   sequence `(captured accepted_through + 1)..=captured_head` for continuity and canonical
   materializability. Any sparse hole or row that cannot be materialized fails closed: MQTT probing
   does not start, no cursor is inherited across the hole, and purge remains held. The minimum MVP
   refuses that legacy cutover; a later reviewed repair may reconstruct a sequence-preserving
   annotation or establish an explicit operator-confirmed loss/baseline. Any later HTTPS response
   carrying the old generation is stale and cannot advance state.
3. Site and Gateway establish a baseline. For a never-bound Gateway it is the start of retained
   outbox. For an existing HTTPS target, MQTT may inherit the logical cursor only after the new Site
   proves it imported the identical contiguous raw prefix. Otherwise it starts from the earliest
   retained outbox; already-purged history is recorded as an explicit operator-confirmed
   `archive_lost` baseline, never silently assumed present.
4. `MQTT_PROBING` enrolls the target and delivers the contiguous production prefix through the
   expected commissioning-smoke pub_seq while purge remains disabled.
5. Formal Site `accepted_through` proving the baseline/smoke atomically changes the lease/generation
   to `MQTT_ACTIVE`; only then may purge resume. Old HTTPS credential material is destroyed/revoked.
6. Remove HTTPS-specific production code/schema only after no target or rollback record references
   it and a final migration verification passes.

Rollback from `CUTOVER_FROZEN` or a probe with no MQTT accepted progress may restore HTTPS with its
captured cursor and a new lease generation. After MQTT accepts any new prefix, rollback to HTTPS is
forbidden unless that consumer independently proves the same contiguous watermark. Otherwise the
system remains purge-held and requires a reviewed construction decision; it never imports an
unproven cursor, accepts a stale response, or abandons custody.

This slice does not modify or resume Plan 6.5.
