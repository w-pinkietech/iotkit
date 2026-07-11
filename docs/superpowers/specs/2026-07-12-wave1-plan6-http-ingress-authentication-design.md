# Wave 1 Plan 6: HTTP ingress and authentication boundary

Status: **SETTLED** (2026-07-12)

## 1. Goal and success

Plan 6 adds the first authenticated network measurement path without weakening the existing
custody contract or creating a remotely claimable gateway. A third-party device builder can
obtain one device credential, copy three documented `curl` commands, submit an envelope, and
interpret every response without reading Rust source.

Success requires all of the following:

- HTTP ingest is a separate, default-off listener and crate, not a control-API route;
- credentials, not `Envelope.source`, define sender identity and authority;
- retryable overload never produces a terminal acknowledgement;
- body, time, queue, rate, freshness, staging, and dedup bounds are explicit;
- an unowned gateway exposes no network control API or UI;
- local recovery, restore, and reset cannot silently restore administrative authority;
- the wire contract, operations, audit, health, documentation, and tests agree.

Risk is **Large / Red**. Design, implementation, review, and settlement use
`gpt-5.6-sol/high`. Product code is implemented by a product-code worker after this spec and
its implementation plan settle.

## 2. Scope

### 2.1 In scope

- `iotkit-ingest-http`, an HTTP binding over `iotkit-ingest-contract`;
- `IngestPrincipal`, per-device bearer tokens, subject scopes, token lifecycle, and audit;
- per-principal and global token buckets, bounded admission, overload hysteresis, and health;
- freshness enforcement and bounded unknown-subject staging/dedup state;
- acknowledgement error detail and wire compatibility tests;
- an authenticated, side-effect-free envelope validation endpoint;
- `docs/ingest-contract.md`, including a three-command `curl` journey;
- construction-tier listener configuration and safe bind restrictions;
- removal of network setup ownership and hardening of local admin recovery;
- the approved restore/reset authority contract needed to prevent future regression.

### 2.2 Out of scope

- MQTT ingest, pairing-window registration, `signed_seq`, and `provisioned_key`;
- measured production capacity values (Plan 6 ships configurable conservative values);
- R22 encrypted snapshot containers and filesystem restore staging/fence implementation
  (Plan 6.5); Plan 6 fixes their normative contract and hardens the currently executable
  DB-level restore authorization boundary;
- a destructive OS/image factory-reset implementation; this specification fixes its authority
  and crash contract for the distribution deliverable;
- image writers, batch labels, enrollment automation, and fleet provisioning;
- rich R23 UI. Health and audit hooks ship, presentation does not.

## 3. Architecture and dependency boundary

Add workspace crate `iotkit-ingest-http`. It owns HTTP parsing, bearer authentication,
admission limits, response mapping, and listener health. It may depend on the ingest contract,
collector-facing boundary, storage/auth services, and narrow shared types; it must not depend
on the gateway control-API module.

`scripts/check-layers` gains an `INGRESS` classification and explicit allowlist. The
architecture crate map and placement rules name it. `iotkit-gateway` constructs and supervises
the listener, but contains no HTTP-ingest domain logic. The existing control API remains a
different listener because its actor, token class, rate profile, and failure semantics differ.

The collector boundary becomes conceptually:

```text
Authenticated binding -> IngestRequest { principal, envelope } -> collector
In-process adapter     -> trusted local principal               -> collector
```

`IngestPrincipal` is internal, not sender-serialized. It contains a stable principal ID,
credential ID, allowed subject set (or one resolved subject), flow class/profile, auth epoch,
and actor kind. It is used for:

- subject resolution and authorization;
- dedup namespace;
- per-sender flow accounting;
- audit attribution and stale-credential health;
- intrusion signals.

`Envelope.source` remains required wire metadata for diagnostics and compatibility. It never
selects authority, dedup ownership, or rate buckets. A configured source identity mismatch is
a terminal envelope rejection and an intrusion audit event; logs contain principal/credential
IDs, never bearer plaintext.

## 4. Device credential model

### 4.1 Storage and bearer format

A migration adds device-ingest credential records with:

- opaque token/credential ID and SHA-256 token hash;
- owning device/principal and auth epoch;
- explicit subject scopes;
- flow class/profile;
- lifecycle state: `current`, `pending`, `revoked`;
- issued, last-used (write-throttled), confirmed, and revoked metadata;
- human reason for issue/reissue/revoke without secret text.

The first implementation supports `simple_bearer`: a gateway-CSPRNG long-lived token shown
exactly once. Plaintext never enters the DB, Debug output, errors, tracing fields, audit, or
health. Constant-time hash comparison is required. Hardware replacement and disposal revoke
credentials attached to the retired hardware authority.

Device add integrates initial token issue as a human-approved action. Revoke and reissue are
human-only because they can silence a sensor. AI credentials cannot invoke them. A configurable
last-used threshold exposes stale/unused credentials in R12 health without auto-revocation.

### 4.2 Make-before-break reissue

Each principal has at most one `current` and one `pending` token. Reissue creates the pending
token while current remains valid; a second reissue while pending exists returns a conflict.
Successful authenticated use marks pending as proven but does not revoke current. A subsequent
human confirmation atomically promotes pending and revokes current. Abandonment revokes only
pending.

If the one-time response containing a new token is lost, the operator identifies and abandons
that pending credential, then issues another; plaintext is never redisplayed. If initial issue
response is lost, the unusable credential is revoked and reissued. Transaction and concurrency
tests cover duplicate requests, response loss, confirmation, abandonment, and DB failure.

### 4.3 Subject resolution

- A one-subject principal may omit `subject_hint`; the collector resolves the sole authorized
  subject from the principal.
- A multi-subject principal must supply `subject_hint` per item. Omission is terminal
  item-level `UnknownSubject`.
- A supplied subject outside the principal scope is terminal `SubjectScopeViolation` and
  emits an intrusion audit signal at item level.
- A supplied subject that is not registered is terminal `UnknownSubject` for an HTTP
  device-token principal; token scopes may contain only registered `system_id` values. Only a
  trusted in-process/official-adapter principal may create an unknown-subject sighting and use
  the existing bounded staging policy.

No sender-controlled field can widen scope.

Subject failures are positionally item-scoped: missing, out-of-scope, or unknown subjects
produce `ItemRejected`, while valid siblings commit and appear in the same
`AckStatus::Accepted { items }`. Only an envelope `source`/principal mismatch (or another
envelope-wide deterministic violation) produces envelope-level `AckStatus::Rejected`.

## 5. HTTP wire and acknowledgement contract

The initial endpoint is versioned (`POST /api/v1/ingest`) and accepts JSON only. Authentication
and admission occur before body consumption wherever the transport permits. Limits cover
header bytes/count, content length, decoded body bytes, item count, connect/read/whole-request
time, and concurrent requests. Chunked or missing-length bodies are read only through the same
bounded reader. Unsupported content encoding is rejected.

Response mapping:

| Condition | HTTP / body | Sender action |
|---|---|---|
| committed accepted/duplicate/terminal contract result | 200 + `EnvelopeAck` | obey ack status |
| invalid/missing credential | 401, no ingest ack | repair credential; do not assume custody |
| authenticated but source/principal mismatch | 200 envelope-level terminal `Rejected` | remove/fix envelope |
| item subject missing/out-of-scope/unknown | 200 `Accepted` with positional `ItemRejected`; valid siblings commit | fix future item input |
| per-principal/global throttle before custody | 429 + bounded `Retry-After`, no terminal ack | retry identical envelope with jitter |
| bounded queue unavailable / service draining | 503, no terminal ack | retry identical envelope |
| storage/commit/internal failure | 503 or connection failure, no terminal ack | retry identical envelope |
| deterministic malformed/oversize envelope | 200 terminal rejected ack when safely parsed; otherwise 4xx without custody | fix input |

`AckStatus::Deferred` remains valid for an already parsed internal admission point, but network
throttle normally uses 429/503. It is never converted to `Rejected`.

`Rejected` and `ItemRejected` add optional `field_path` (JSON Pointer) and optional stable
`schema_hint`, both with explicit deserialization defaults. `ReasonCode::Internal` remains as
deprecated, read-only v1 vocabulary so new readers can consume old data, but no Plan 6 producer
may emit it: internal/storage failure cannot authorize spool deletion. Wire snapshots and
old/new reader fixtures prove that new readers accept every old v1 value and old readers accept
additive optional fields; other breaking changes require a new endpoint version.

`POST /api/v1/ingest/validate` is authenticated and passes through the same exposure, pre-auth,
body, time, and principal flow limits. It returns a distinct `ValidationReport`, never an
`EnvelopeAck`. It performs parsing, source/scope, schema, subject, and freshness checks without
writing dedup, readings, staging, audit success/custody state, or any other product state. It
never claims custody or authorizes spool deletion. Security violations may still emit the same
bounded intrusion audit episode as real ingest. The rich live inspector remains deferred.

`docs/ingest-contract.md` is normative for device builders and contains exactly this minimal
journey before advanced explanation:

1. obtain a device token plus the gateway public certificate/fingerprint from the operator;
2. export gateway URL/token/CA path and save one JSON envelope;
3. use `curl --cacert "$IOTKIT_CA"` (never `--insecure`) and interpret accepted, duplicate,
   rejected, 429, and 503.

Examples redact real secrets and state that an identical `envelope_id` and payload are retained
across retry. The ESP32 path provisions the same certificate/SPKI trust anchor and verifies it
on every connection; a bearer token without server authentication is not a supported setup.

## 6. Admission, throttling, and bounded state

### 6.1 Token buckets

Admission uses a token bucket per authenticated principal plus a global bucket. Both must admit
the request. Before credential lookup, a separate global authentication-work bucket, bounded
authentication-worker semaphore, and bounded TTL-evicted per-source failure limiter constrain
random-token work. The source key is the observed peer address after trusted-proxy handling
(Plan 6 supports no proxy-derived client address). Exceeding either pre-auth limit returns 429
with bounded `Retry-After`; invalid credentials otherwise return 401. The source map has an
explicit cardinality cap and TTL; overflow receives only the stricter global limit, never an
unbounded new entry. Restart initializes these buckets conservatively without a full burst.
Failure episodes are aggregated in audit/health. A fair queue plus a reserved bounded slice for
recently validated credentials prevents a churn of invalid sources from consuming every
authentication worker; cache entries are bounded and invalidated by auth epoch/revocation
generation.

Authenticated cost accounts for request and decoded bytes/items so many large batches cannot
bypass a request-only limit. Admission is two-stage: before body consumption it reserves a
conservative cost derived from authenticated principal, declared length, and fixed overhead;
unknown/chunked length reserves the configured maximum. After bounded decoding, it reconciles
against actual bytes/items before queue handoff. If the additional cost is unavailable, it
returns 429 without custody; unused reservation is refunded. Reservations are released/refunded
on every error, timeout, disconnect, cancellation, and task teardown. Authentication and
bytes/work actually consumed are charged irreversibly on every outcome; parse failure or
disconnect refunds only the unused portion of the conservative reservation. Releasing a queue,
semaphore, or memory reservation never restores already-consumed token budget. Buckets use monotonic time.
Runtime configuration supplies
conservative provisional rates, bursts, class membership, and cooldown values; zero/unbounded
values are rejected unless a separately named safe semantic exists.

Changing a device's flow class or adding a device checks configured aggregate steady-state and
burst capacity against the listener/global budget. A change that exceeds capacity is rejected
unless a human explicitly accepts `capacity_debt` through a construction-tier dry-run/execute
operation. Debt is loud in health and audit until capacity or assignments recover.

Throttle episodes use enter/exit hysteresis and cooldown. R12 exposes current state, debt,
queue occupancy/high-water, per-class pressure, and cumulative `throttled_drop_count` without
unbounded per-token cardinality. Audit emits one start and one recovery/summary record per
episode, not one event per rejected request. R23 receives a future hook.

### 6.2 Queue, body, and time bounds

The ingress-to-collector queue is bounded. No request waits forever for a slot. Admission order
is: connection/header/pre-auth limits, credential lookup, authenticated conservative
reservation, bounded body parse, cost reconciliation, freshness/contract validation, bounded
queue handoff. Capacity reservations are released on disconnect, timeout, parse failure, and
panic-safe task teardown.

### 6.3 Staging and dedup bounds

Unknown-subject sighting staging has configurable global and per-principal row/byte ceilings and
age retention. Within the same transaction as the new staging write, overflow evicts the oldest
eligible unpinned entry and emits bounded audit/health; the newly acknowledged staged item is
therefore present at the durability point. Rows under active operator investigation may be
explicitly pinned, but global and per-principal pin budgets are validated to leave at least one
maximum envelope of evictable capacity. A pin operation that would consume that reserve is
rejected as an R14 precondition, so ingest never reaches an all-pinned no-victim state. Concurrent
eviction, staging, normal sibling writes, and dedup claim share the serialized transaction.

Dedup keys are `(stable_principal_id, envelope_id)`. Auth epoch controls credential validity and
cache invalidation but is deliberately absent from the normal dedup identity. R22 restore does
not restore readings/outbox and therefore must not restore their dedup claims: suppressing a
retry could hide a reading absent from the replacement. Restore explicitly resets the dedup
window, reports that unchanged post-restore retries may be accepted again, and relies on the new
ledger epoch plus downstream idempotency/replay negotiation to expose possible duplicates.
Retention is bounded by age and a
configured global/per-principal maximum. If maintenance cannot preserve the configured window,
health becomes degraded and an episode audit states that duplicate suppression guarantees are
reduced. Purge failure never changes a committed ack into failure.

## 7. Freshness

Plan 6 has one `ClockTrust` state machine shared by session and freshness decisions. Startup is
`untrusted`. It becomes `trusted(source, observed_at)` only when either (a) the Linux kernel time
sync signal reports synchronized and wall time is at least the persisted auth-time floor, or
(b) local root runs `gatewayctl time confirm` after the command displays the current time and
persisted floor and requires typed confirmation. Persistence records only the nondecreasing
floor and evidence metadata, never a claim that a later boot is already trusted. Kernel-sync
loss, a wall time below the floor, or a backward step beyond the configured small tolerance
returns the state to `untrusted`; recovery requires fresh sync evidence or another local-root
confirmation. R12 reports state, evidence source, and the exact recovery command.
While trusted, every finite-session issue/authentication transaction advances the persisted
floor to at least its observed wall time; a bounded periodic checkpoint advances it during long
sessions. A failed floor write fails the authentication action rather than extending authority.

After time provenance is resolved, externally authenticated observations older than the
configured freshness window are terminally rejected with `StaleTimestamp` and a field path;
future-skew has an explicit finite limit. These terminal comparisons require trusted wall time.
With trusted time, stale/future failures are positional `ItemRejected` outcomes; valid siblings
commit in the same accepted envelope.
While untrusted, gateway-receive-time observations remain acceptable and `age_ms` can be checked
directly against the window, but an absolute device timestamp that requires wall comparison
returns 503/no ack for the entire envelope so the sender retains it. Gateway receive-time
observations cannot be stale at arrival. Tests cover absent clocks, both trust sources and loss,
device time, reconstructed age, exact boundaries, overflow, forward/backward wall-clock changes,
restart, and mixed-item batches.

## 8. Listener construction and exposure

HTTP ingest defaults to disabled. Enable/disable, bind address/interface, plaintext/TLS mode,
and allowed site-local CIDRs are typed construction-tier R14 operations with dry-run and audit.
Configuration cannot authorize a public/internet bind. Interface/address validation and the
runtime peer guard both enforce site-local reachability.

TLS is the normal mode. Plaintext is allowed only when explicitly selected for a private
site-local interface/CIDR and produces persistent degraded health. It is never accepted on a
wildcard that includes a non-private route. Partial, mismatched, or corrupt TLS material fails
closed; it is never silently regenerated. Rotation is explicit, audited, and publishes the new
fingerprint for out-of-band verification.

## 9. Control-plane ownership closure

Plan 6 removes `/api/v1/setup/passphrase` and every setup-mode unauthenticated operation. Before
binding the control API/UI, startup requires an owned database, a complete matching TLS pair,
and no reset/restore fence. A missing/corrupt DB or absent admin credential enters
`local_recovery_required`; it never creates a network setup window.

`gatewayctl passphrase reset` is the only Plan 6 ownership/recovery producer. It accepts input
through a non-echoed local TTY or SSH-root session, hashes outside the SQLite write lock, and in
one transaction updates the admin credential, advances/preserves the appropriate auth
generation, revokes every operator/session token, and writes audit. Concurrent resets serialize;
only the last committed passphrase remains usable. Failed writes leave prior state intact and
the runtime unbound when ownership was absent.

Finite sessions use a persisted nondecreasing auth-time floor. Issue/authentication is refused
while wall time is untrusted or behind the floor; rollback never lowers the floor or lengthens a
session. Restart reloads the floor but begins clock trust as untrusted. Long-lived service tokens
remain governed by explicit revocation and auth epoch.

## 10. Restore, recovery, and factory reset contract

Plan 6 records the approved R22 contract; Plan 6.5 implements encrypted carriage and the durable
filesystem restore fence.

Plan 6 owns the currently executable DB restore closure. Restore must create the target
exclusively itself or prove exhaustive emptiness across every application-owned table/state;
checking only snapshot section tables is forbidden. In the restore transaction it clears any
target admin credential, operator token, and session authority, mints the new auth epoch, and
commits restored DB state into `local_recovery_required`. Startup therefore cannot treat a
restored target as owned. Plan 6.5 alone owns encrypted secret carriage and cross-filesystem
TLS/fence staging; that split may not defer this DB-level authorization closure.

Once any device-token secret exists, the legacy plaintext snapshot exporter must refuse to
claim or create an R22 replacement snapshot: omitting tokens breaks the approved continuity
contract, while writing their hashes plaintext breaks D2. Until Plan 6.5 lands, health and the
CLI state that replacement backup is unavailable and name the missing encrypted-container
support. Legacy state-only artifacts may be inspected/imported only through an explicitly named
non-replacement mode that still enters local recovery. No successful command may silently label
such an artifact as a complete backup. Plan 6.5 removes this temporary distribution blocker.

Replacement restore preserves logical `gateway_identity`, TLS private key/certificate, and
active device-token hashes, while minting new ledger and unpredictable auth epochs. The user
accepts that a device-token revocation after the backup can roll back in Standalone mode; restore
must report this loudly. Admin credentials, operator tokens, and sessions never return active.
The restored gateway remains unbound until local admin recovery, then operator-token reissue.

A durable `restore-in-progress` record outside both the DB and replaceable TLS paths binds the
intended DB generations to the TLS generation/fingerprint. Any partial, corrupt, or mismatched
state resumes repair or stays unbound. Restore accepts only a target created exclusively by
restore or an exhaustively pristine target; response loss is resolved by inspecting committed
generation state, never by replaying over a non-pristine target.

Admin recovery is non-destructive. Factory reset is different: SSH root or physical/local root
may invoke a full erase of application DB, identity, TLS key, credentials, secrets, config, and
local data. It is not an IoTKit API, AI, or general R14 operation. A durable
`reset-in-progress` marker forces restart to complete erase or remain unbound. Completion creates
an unowned box and a new identity only during later initialization; upstream retirement of the
old identity is a separate operator task.

## 11. Operations, audit, and health

New state changes use typed operations. Token issue/reissue/confirm/abandon/revoke and listener
configuration declare permission tier, dry-run behavior, idempotency/conflict behavior, audit
shape, and secret-redaction tests. Initial plaintext is returned only from the successful
human-authorized execution and never from dry-run.

Audit records stable principal/credential IDs, operation, reason, source address where relevant,
result, and episode summaries. It never records bearer/passphrase plaintext or full hostile
payloads. Intrusion signals cover invalid scope, configured-source mismatch, repeated invalid
credentials (coarsely aggregated), and bound violations without becoming a log-amplification
vector.

Health reports listener enabled/bound/mode, safe exposure classification, queue pressure,
throttle episode, capacity debt, staging/dedup degradation, credential staleness counts, and
actionable next steps. Sensitive identities are bounded/redacted.

## 12. State and failure invariants

| Invariant | Failure behavior | Required proof |
|---|---|---|
| LAN timing cannot claim an unowned box | API/UI never binds | startup/network negative test |
| sender metadata cannot choose authority | terminal reject + intrusion audit | forged source/scope tests |
| ack implies the documented durability point | no ack on commit/storage failure | fault injection around commit |
| throttle cannot delete a sender spool | 429/503/Deferred only | wire + retry-client tests |
| credential plaintext is one-shot and redacted | revoke/reissue after response loss | capture/log/Debug snapshots |
| reissue has bounded overlap | max current+pending; explicit promotion | concurrency/state-machine tests |
| every buffer is finite | refusal/degraded health, never silent growth | property/load tests |
| clock rollback grants no extra session life | finite auth fails closed | unset/sync/backward/restart tests |
| partial TLS/restore/reset never exposes service | stay unbound or resume repair | boundary fault injection |

## 13. Verification and acceptance

Implementation proceeds contract-first with failing tests before production behavior. Required
gates include:

1. wire fixtures for every ack and HTTP response, including old/new compatibility;
2. principal-versus-envelope identity, one/multi-subject, HTTP-unknown-subject rejection,
   trusted-adapter staging, cross-scope, and dedup tests;
3. token lifecycle concurrency, lost response, secret capture, and revocation tests;
4. deterministic pre-auth source churn/cardinality/restart/fairness, two-stage reservation,
   valid-client-under-invalid-load, rate/queue, and hysteresis tests using controllable time;
5. body/header/time/concurrency hostile-input and disconnect tests;
6. freshness boundary, kernel-sync/manual-confirm clock trust transitions, untrusted absolute
   timestamp retry, and wall-clock anomaly/restart tests;
7. staging/dedup ceiling and degraded-health tests;
8. owned/unowned startup, network route inventory, recovery revocation, and TLS fault tests;
9. end-to-end pinned-TLS three-command `curl` journey, ESP32 trust-anchor guidance,
   side-effect-free validation, and identical
   retry/duplicate behavior;
10. `cargo fmt --all --check`, layer checks, targeted tests, workspace tests, clippy, and docs
    link/command checks.

Initial tuning is accepted only as conservative configurable defaults with their rationale.
Capacity benchmarks and power-loss rigs are a named follow-up before production-scale claims.

## 14. User journey and usability gate

One gateway requires one local ownership step before browser/API use. After ownership, a human
adds a device, copies its one-time token, and the device builder follows the three-command ingest
document. Normal measurement submission requires no gateway shell access. Losing an admin
passphrase uses local recovery without deleting measurements; losing both local authority and
the passphrase is intentionally unrecoverable over LAN.

Plan 6 does not pretend that repeating local setup for 20–100 boxes is easy. Before external
distribution, a separately approved deliverable must provide per-card generation/custody,
encrypted export, duplicate detection, labels/manifests, certificate checks, restart handling,
and Site-managed enrollment assistance. Shared image credentials remain forbidden.

## 15. Traceability and supersession

- R2/R19 and D11: authenticated, bounded, default-off site-LAN ingress;
- D1/D5: ack durability, retry semantics, freshness, subject resolution;
- D2/R22/D8: approved replacement continuity with new epochs and explicit rollback risk;
- D13/D3: local-only initial ownership and no setup-mode network exception;
- Plan 5 spec: retained as historical implementation record; its network setup behavior is
  expressly superseded;
- `PLAN6-DESIGN-READY.md`: approved Red decisions and adversarial evidence;
- Plan 6.5: encrypted snapshot carriage and restore-fence mechanics;
- distribution deliverable: factory-reset implementation and batch provisioning UX.

Canon, README, architecture, deferred-hardening ledger, and active workflow ledger must all
carry the same distinction between current code and approved target before implementation starts.
