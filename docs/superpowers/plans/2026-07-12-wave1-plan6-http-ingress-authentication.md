# Wave 1 Plan 6: HTTP ingress and authentication implementation plan

Status: **SETTLED** (2026-07-12)

## Mission and authority

Implement the settled [Plan 6 specification](../specs/2026-07-12-wave1-plan6-http-ingress-authentication-design.md):
a default-off authenticated site-LAN HTTP measurement path, per-device credentials, bounded
admission, complete ack/validation documentation, and closure of the remotely claimable
control-plane setup state.

This is **Large / Red**. Every product-code task is dispatched through:

```bash
CODEX_MODEL=gpt-5.6-sol CODEX_EFFORT=high scripts/codex.sh impl <prompt> <label>
```

Workers do not commit. Main reads the diff, runs task checks, obtains the required independent
review, and makes each intentional commit. No push/PR/release is authorized.

Design authority: D1, D2, D3, D5, D11, D13, R2/R19/R22, the settled spec, and
`docs/architecture.md`. If implementation exposes a new Red choice, stop; do not improvise.

## Global contracts

Every task preserves these invariants:

- `EnvelopeAck` is returned only at the existing durability point. Storage/internal failure,
  429, and 503 never become terminal `Rejected`.
- Authenticated principal identity, never `Envelope.source`, owns scope, dedup, flow, and audit.
- Missing/out-of-scope/unknown HTTP subjects and trusted-clock freshness failures are positional
  `ItemRejected`; valid siblings commit. Source mismatch is envelope-level `Rejected`.
- Every hostile-input collection, queue, cache, bucket, staging set, and dedup set has a finite
  configured ceiling plus tested overflow behavior.
- Credential/passphrase/key plaintext never enters Debug, logs, errors, audit, fixtures, or DB.
- Unowned/recovering/corrupt/partial-TLS/reset/restore-fenced state never binds control API/UI.
- New state changes use R14 typed operations; local root recovery/reset remain explicitly
  outside network R14 as settled.
- The listener is default-off, site-LAN-only, never Internet-exposed. TLS is normal; explicit
  private-LAN plaintext is degraded health.
- Legacy plaintext snapshot export must fail once device-token secrets exist until Plan 6.5
  (implemented with the credential schema in Task 3).
- Each task begins with a failing test or negative probe and ends with focused tests,
  `git diff --check`, and a Main-owned commit boundary.

Forbidden throughout: MQTT, pairing windows, `signed_seq`, `provisioned_key`, rich R23 UI,
encrypted snapshot container/fence mechanics, destructive factory-reset implementation, batch
provisioner, public bind, and unrelated cleanup.

## Delivery graph

```text
T1 ownership/restore closure ───────────────┐
T2 contract + principal collector seam ────┼─> T4 listener skeleton ─> T5 HTTP path
T3 device credentials + lifecycle ─────────┘                          ├─> T6 bounds/health
                                                                     └─> T7 docs/integration
```

T1–T3 may be implemented as separate reviewed commits. T3 depends on T1's single auth-epoch
owner and extends its restore inventory. T4 depends on their stable interfaces. T4 and T5 leave
a non-configurable internal `ingress_ready = false` gate: no operation can bind the listener.
Only T6 removes that gate after all network-reachable bounds and episode state pass; T4–T6 are
therefore one exposure unit even though Main may retain reviewed intermediate commits.

## Task 1: Close network ownership, recovery, clock, and current restore authority

**Contracts**

- Delete `/api/v1/setup/passphrase` and `SETUP_ALLOWED_OPS`; setup-mode network requests have no
  route because the server is not bound.
- Startup classifies `unowned`, `local_recovery_required`, `owned`, and fail-closed TLS/fence
  states before spawning API/UI.
- `gatewayctl passphrase reset` uses non-echoed confirmation, hashes outside the write lock, and
  atomically updates admin credential, revokes every operator/session token, advances required
  auth generation, and audits.
- Implement shared `ClockTrust`: kernel-synchronized or explicit local-root time confirmation;
  startup untrusted; persisted nondecreasing floor; finite sessions fail closed.
- Harden current restore to exclusively-created/exhaustively-pristine target, clear all
  admin/operator/session authority, mint auth epoch, and force local recovery.
- `gatewayctl time confirm` is a local-root recovery-class action, not a network catalog
  operation; its evidence write is transactional and audited as `local_cli`.

**Likely files**

- `core/ops/migrations/*`, `core/ops/src/auth.rs`, `core/ops/src/catalog.rs`
- `iotkit-gateway/src/api/{mod.rs,routes.rs,auth_layer.rs,tls.rs}`
- `iotkit-gateway/src/main.rs`, `iotkit-gateway/src/health.rs`
- `iotkit-gatewayctl/src/cmd/{passphrase.rs,snapshot.rs,time.rs}` and CLI registration
- API/CLI/restore integration tests

**First failing tests / probes**

1. Fresh DB and missing admin credential never open a TCP control listener; route inventory has
   no setup path or setup actor/allowlist.
2. Reset invalidates an already-issued human token and session in the same transaction; injected
   audit/DB failure preserves the previous credential/authority state.
3. Terminal capture contains no entered passphrase and requires confirmation.
4. Auth-only restore target is rejected; successful restore rejects all old admin/operator
   authority and remains unbound.
5. Untrusted/backward clock rejects finite sessions; kernel sync/manual confirmation recovers;
   restart reloads the floor but not trusted status.
6. Every finite-session issue/auth transaction advances the floor atomically; injected floor
   write failure rejects auth. Concurrent auth, periodic long-session checkpoint, backward step,
   and restart share one injected `ClockTrust` owner also consumable by ingress freshness.

**Completion checks**

```bash
cargo test -p iotkit-core-ops
cargo test -p iotkit-gateway --test api_basic --test api_e2e
cargo test -p iotkit-gatewayctl
rg -n 'setup/passphrase|SETUP_ALLOWED_OPS|setup_mode' iotkit-gateway core/ops
```

The final search may find historical test names only if they assert absence/supersession. Main
reviews transaction boundaries and listener spawn order. Commit boundary:
`fix(auth): close network ownership and restore authority`.

## Task 2: Evolve the wire contract and introduce `IngestPrincipal`

**Contracts**

- Add optional defaulted `field_path` and `schema_hint` to both rejection shapes.
- Keep `ReasonCode::Internal` deserializable and deprecated; production code cannot construct it.
- Add a distinct `ValidationReport` wire type that is never an ack/custody signal.
- Introduce internal `IngestRequest { principal, envelope }`; trusted in-process adapters receive
  explicit local principals. Remove collector authority use of `envelope.source`.
- Stable dedup key is `(principal_id, envelope_id)`. Restore resets the dedup window because
  readings/outbox are not restored; do not add auth epoch to the key.
- One-subject omission resolves from principal; multi-subject omission, HTTP unknown subject,
  scope violation, and trusted-clock stale/future are positional item failures. Only trusted
  official/in-process principals can stage unknown sightings.

**Likely files**

- `iotkit-ingest-contract/src/{ack.rs,envelope.rs,lib.rs}` and wire fixtures
- `core/collector/src/{actor.rs,registry_policy.rs}` and tests
- `iotkit-ingest-client`, polling runtime/adapters where the in-process principal is constructed
- `core/timeseries` dedup API/migration only if the stable principal representation requires it

**First failing tests**

1. Old v1 JSON including `internal` deserializes; new rejection fields are absent-compatible.
   A frozen test-only copy of the actual old-v1 reader types also consumes newly serialized
   `Rejected` and `ItemRejected` values containing both additive fields. This is a real old
   reader harness, not only a new-reader fixture.
2. No production path produces `Internal` (constructor/search guard plus behavior tests).
3. Forged `Envelope.source` cannot change dedup or subject scope and produces envelope reject +
   bounded intrusion signal hook.
4. Mixed batch commits valid siblings and returns positional subject/freshness rejections.
5. One-subject omission succeeds; multi-subject omission and HTTP unknown subject reject;
   official-adapter unknown subject stages.
6. Same principal/envelope is duplicate across credential reissue/auth epoch; documented restore
   reset accepts it under the new ledger epoch.

**Completion checks**

```bash
cargo test -p iotkit-ingest-contract
cargo test -p iotkit-core-collector
cargo test -p iotkit-ingest-client
rg -n 'ReasonCode::Internal' --glob '*.rs'
```

Main checks every `process_envelope` caller supplies a principal and no authority decision still
reads `Envelope.source`. Commit boundary:
`feat(ingest): add principal-aware collector contract`.

## Task 3: Device credential schema, lifecycle, capacity operations

**Contracts**

- Add one migration slice owned by `core/ops` (or a narrowly justified auth-owning crate) for
  device credentials, scopes, auth epoch, flow class/profile, and current/pending/revoked state.
- Store SHA-256 token hashes; opaque token plaintext is CSPRNG-generated, shown once, and wrapped
  in a redacted type. Comparison is constant-time.
- Device add + first issue is one human-approved transaction. Reissue permits at most current +
  pending; successful pending use marks proven; human confirm promotes/revokes old atomically;
  abandon revokes pending only. Concurrent second reissue conflicts.
- Hardware replacement/disposal revokes old hardware authority.
- Subject scopes contain registered `system_id` values only.
- Device add/class change performs aggregate capacity validation. Exceeding it requires explicit
  human construction-tier `capacity_debt` approval; debt is health/audit state.
- Issue/reissue/confirm/abandon/revoke/list are typed R14 operations with dry-run and redaction.
- In the same task, gate legacy plaintext replacement export whenever any device credential
  exists; it fails with an actionable Plan 6.5 message and never emits hashes or a false complete
  backup success.
- Reuse T1's single auth-epoch owner; do not add a second epoch source. Extend T1's exhaustive
  restore/pristine inventory for every credential/scope/flow table and keep target authority
  clearing atomic with restored DB state.
- Expose persistent bounded health `replacement_backup_unavailable` plus the exact Plan 6.5
  recovery action while credentials exist and encrypted export is absent.

**Likely files**

- `core/ops/migrations/*`, `core/ops/src/auth.rs`, `core/ops/src/ops/*`, catalog tests
- `core/ledger` device add/replace integration seam
- `iotkit-gatewayctl` command presentation (plaintext once)
- `iotkit-gatewayctl/src/cmd/snapshot.rs` replacement-export gate
- health model for stale credentials/capacity debt

**First failing tests**

1. Migration constraints reject two current/two pending credentials and unregistered scopes.
2. Issue/auth/revoke and redacted Debug/log/error/audit snapshots.
3. Lost initial/reissue response, second concurrent reissue, pending proof, confirmation,
   abandonment, and transaction-failure state-machine tests.
4. Retire/replace revokes old credential authority.
5. Device add/class change capacity matrix: under budget, refused excess, explicit debt, and debt
   recovery; AI cannot approve debt or credential lifecycle operations.
6. `last_used_at` writes are throttled and stale health has bounded counts, no secret/cardinality
   leak.
7. Legacy export refuses with a synthetic credential present and capture proves no hash/plaintext
   reaches stdout/stderr/artifact.
8. An otherwise pristine restore target containing only device credential/scope rows is rejected
   without changing authority or auth epoch; successful restore clears all such target authority.

**Completion checks**

```bash
cargo test -p iotkit-core-ops
cargo test -p iotkit-core-ledger
cargo test -p iotkit-gatewayctl
```

Main inspects unique indexes/checks and transaction concurrency, then commits:
`feat(auth): add device credential lifecycle and capacity guard`.

## Task 4: Add `iotkit-ingest-http` and the safe listener construction boundary

**Contracts**

- New workspace crate `iotkit-ingest-http`; no route/domain logic in gateway control API.
- Add `INGRESS` to `scripts/check-layers` and the architecture crate map/allowlist.
- Gateway composition supervises the listener independently and reports task exit accurately.
- The common T1 ownership/recovery/reset/restore/TLS-generation gate is a prerequisite for every
  network listener, control and ingest; no restored enable flag can bypass it.
- Default disabled. Construction-tier operations own enable/bind/interface/site CIDR/TLS or
  explicit private plaintext. Config/runtime guard reject Internet-capable exposure.
- Complete matching TLS pair required; partial/corrupt/mismatch fails closed; explicit audited
  rotation only. Plaintext private mode is persistent degraded health.
- Listener configuration has one durable DB owner and is changed atomically by typed R14
  construction operations. Gateway generation observation applies/restarts the listener without
  an unaudited config-file mutation path; failure retains the last safe state or stays unbound.
- Desired and applied listener/TLS generations are explicit. Applying a committed desired
  generation stages/validates TLS and bind before switchover; any failure reports desired versus
  applied state and retains the last safe applied listener or stays unbound, with no transient
  unsafe bind.
- Until T6, a compiled internal readiness gate makes every enable operation fail
  `ingress_not_ready`; it is not a user setting and cannot be bypassed.

**Likely files**

- `iotkit-ingest-http/{Cargo.toml,src/lib.rs,src/config.rs,src/tls.rs}`
- root `Cargo.toml`, `Cargo.lock`, `scripts/check-layers`
- `iotkit-gateway/src/main.rs`, config/health/composition tests
- `core/ops` listener descriptors plus the owning migration/store
- `docs/architecture.md` crate map (placement row already reserves the target)

**First failing tests / probes**

1. Layer checker rejects a deliberate ingress-to-control-API dependency fixture.
2. Default config opens no ingest socket; enabled safe loopback/site-LAN test config does.
3. Public/wildcard-with-public-route and unapproved interface/CIDR fail validation and runtime
   peer checks; IPv4-mapped/IPv6 cases cannot bypass classification.
4. Plaintext on private LAN is degraded; plaintext on unsafe bind is impossible.
5. Missing/corrupt/mismatched TLS pair never regenerates and never binds; explicit rotation
   changes fingerprint and audits.
6. Listener crash/drain removes healthy status without stopping in-process collection.
7. Both control and ingest sockets remain closed in unowned, local-recovery, reset/restore-fenced,
   corrupt/partial-TLS, and generation-mismatched states.
8. Fault injection after desired commit, TLS staging/replacement, bind, drain, restart, and
   switchover proves desired/applied health accuracy and no unsafe transient listener.

**Completion checks**

```bash
scripts/check-layers
cargo test -p iotkit-ingest-http
cargo test -p iotkit-gateway
```

Commit boundary: `feat(ingress): add default-off site-LAN HTTP listener`.

## Task 5: Implement authenticated HTTP ingest and validation with bounded admission

**Contracts**

- Endpoints: `POST /api/v1/ingest` and `/api/v1/ingest/validate`, JSON only.
- Enforce header/connect/concurrency/pre-auth limits before credential lookup. Pre-auth state is
  global plus bounded TTL per observed peer, with conservative restart and fair/reserved service
  for recently validated credentials; no proxy-derived source address.
- Two-stage principal charging reserves before body (maximum for chunked/unknown), reconciles
  decoded bytes/items before queue, charges consumed work on every outcome, and refunds only
  unused reservation.
- Bound body/read/whole request/concurrency and queue wait. Every cancellation path releases
  memory/semaphore/queue capacity without refunding consumed work.
- Response mapping follows spec exactly: 401 invalid auth, 429 + bounded Retry-After, 503/no ack
  for queue/internal/untrusted absolute-clock comparison, 200 for committed ack.
- Validation uses the same limits but writes no dedup/readings/staging/custody state and returns
  `ValidationReport`, never `EnvelopeAck`.
- Recently-valid cache entries are bound to auth epoch and revocation generation. Any committed
  revoke, reissue promotion, reset, or restore advance invalidates both authentication and its
  reserved admission lane before another request can pass.
- T5 does not remove the internal readiness gate; its endpoints remain unreachable in production
  until T6 supplies all durable dedup/staging bounds and bounded episode accounting.

**Likely files**

- `iotkit-ingest-http/src/{auth.rs,admission.rs,routes.rs,response.rs,lib.rs}`
- integration fixtures and controllable clock/token-bucket test support
- narrow collector/device-auth APIs from T2/T3

**First failing tests**

1. Random-token source churn/cardinality cap/restart/global starvation and valid-client fairness.
2. Content-length/chunked/missing-length, late malformed JSON, timeout, disconnect, queue-full,
   and cancellation leave every resource bounded and charge consumed work.
3. Concurrent same envelope produces one commit and one duplicate after the winner commits.
4. Exact HTTP/ack matrix, Retry-After bounds, no terminal ack on overload/storage failure.
5. Validation report matches real deterministic validation but leaves DB before/after identical
   (except bounded security episode audit for malicious scope/source input).
6. Clock untrusted: receive-time/age cases follow spec; absolute comparison returns 503/no ack.
7. Cached-auth races: revoke/reissue/reset/restore commit between lookup and admission causes
   rejection; old generation cannot use the reserved lane.
8. Deterministic cancellation matrix before enqueue, after enqueue, before commit, after commit,
   and after response serialization. After handoff the collector completes detached or safely
   rolls back; no ack precedes commit and lost committed success retries as `Duplicate`.

**Completion checks**

```bash
cargo test -p iotkit-ingest-http
cargo test -p iotkit-core-collector
```

Run deterministic tests under a controllable monotonic/wall clock; do not use sleeps. Main
reviews cancellation and token accounting paths. Commit boundary:
`feat(ingress): authenticate and bound HTTP ingest`.

## Task 6: Bound staging/dedup and expose episode health/audit

**Contracts**

- Enforce per-principal/global staging rows+bytes+age and a separate finite pin budget. Only
  trusted official/in-process sightings can enter it.
- Enforce per-principal/global dedup age+count. Purge failure/degraded window is health+episode
  audit, never a changed committed ack.
- Token-bucket throttle uses hysteresis/cooldown. Audit emits start and recovery/summary once per
  episode, not per request. Expose bounded aggregate `throttled_drop_count`, queue high-water,
  capacity debt, staging/dedup state, credential staleness, mode/bind, and recovery action.
- No health/audit output has unbounded token/source cardinality or hostile payload text.
- Staging overflow atomically evicts the oldest eligible unpinned sighting with bounded audit,
  then stores/acks the new item. Global/per-principal pin budgets reserve at least one maximum
  envelope of evictable capacity; an R14 pin operation that would consume it fails precondition.
- After these bounds and bounded episode structures pass, remove the internal readiness gate.
  This is the first commit in which ingress can be enabled.

**Likely files**

- `core/timeseries`, `core/ledger` sighting staging, collector maintenance
- `iotkit-ingest-http` episode state
- `iotkit-gateway/src/health.rs`, ledger audit helpers

**First failing tests**

1. Per-principal/global row+byte limits, pin budget, age expiry, and concurrent admission.
2. Dedup per-principal/global cap and purge failure degrades without invalidating ack.
3. Hysteresis/cooldown produces exactly one start/one summary per episode under thousands of
   drops; counters saturate safely.
4. Health JSON cardinality/size remains bounded with many source/token IDs and names the next
   operator action.
5. Staging overflow and concurrent mixed batches atomically evict only eligible oldest entries,
   store every acknowledged new staged item, and preserve valid siblings/dedup. Pin-limit tests
   prove the reserved evictable capacity cannot be consumed and protected candidates survive.
6. Replacement-backup-unavailable health remains loud while device credentials exist and clears
   only when complete encrypted support is actually available (not merely configured).

**Completion checks**

```bash
cargo test -p iotkit-core-timeseries -p iotkit-core-ledger -p iotkit-core-collector
cargo test -p iotkit-ingest-http -p iotkit-gateway
```

Commit boundary: `feat(ingress): bound staging dedup and overload health`.

## Task 7: Device-builder contract, end-to-end journey, and integration settlement

**Contracts**

- Create normative `docs/ingest-contract.md` with version, schemas, ack semantics, subject rules,
  freshness/clock cases, retry table, validation endpoint, limits, and examples.
- First journey is three shell commands after operator handoff: export URL/token/CA, write one
  envelope, and pinned `curl --cacert`; never use `--insecure`. Include equivalent ESP32 trust
  anchor guidance without requiring Rust knowledge.
- Explicitly distinguish current implementation from Plan 6.5 blocker and future MQTT/pairing.
- Update README/architecture/current-state wording after code actually lands.
- End-to-end test uses a real TLS listener, issued device token, pinned CA, durable DB assertion,
  duplicate retry, mixed item results, 429/503 retry, validation no-write, restart, and recovery.

**Likely files**

- `docs/ingest-contract.md`, `README.md`, `docs/architecture.md`
- `iotkit-ingest-http/tests/e2e.rs`, gateway/CLI integration tests
- `scripts/verify.sh` or docs command checker only if required to exercise the journey in CI

**First failing tests / probes**

1. Execute the documented commands verbatim against an ephemeral gateway; ordinary self-signed
   TLS without the provided CA fails, pinned command succeeds.
2. Accepted/duplicate/rejected/429/503 examples deserialize to the shipped types.
3. Secret scan of captured stdout/stderr/log/audit does not find the test token/passphrase.
4. Owned/unowned/recovery/TLS fault network probes match the final route/listener inventory.

**Completion checks**

```bash
cargo fmt --all --check
scripts/check-layers
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
scripts/verify.sh
git diff --check
```

Main then dispatches a fresh Sol/high final integration review over the exact manifest. Commit
boundary: `docs(ingest): publish authenticated HTTP contract` (or fold small code-test cleanup
into a clearly named final integration commit).

## Review and interruption protocol

After every task:

1. Main reads every diff and verifies only the named scope changed.
2. The worker runs `scripts/verify.sh`; Main reruns it before every task commit, in addition to
   focused tests and the task's completion commands.
3. Build a manifest for changed product/tests/docs plus relevant contract artifacts.
4. Fresh Codex Sol/high reviews security, custody/ack, concurrency, cancellation, canon, and UX.
5. Every Rust/product-code fix is redispatched through `scripts/codex.sh impl`; Main edits only
   docs/workflow/CI within its authority. Fix all C/I, rerun full verification, confirm on the
   final hash, then commit and record hash/receipt in the active ledger.

If interrupted, never infer completion from files alone. The active ledger records last committed
task, current manifest/review status, failing/passing commands, and exact next executable step.
Uncommitted worker output remains unowned until Main verifies it. A task is not complete merely
because its focused test passes.

## Final acceptance

Plan 6 is complete only when all task commits are present, `scripts/verify.sh` passes, the
documented pinned-TLS journey runs verbatim, the final integration manifest has zero unresolved
Critical/Important findings, canon/current-state docs agree, and no push/release has occurred.
Plan 6.5 remains a hard pre-distribution follow-up for encrypted replacement backups.
