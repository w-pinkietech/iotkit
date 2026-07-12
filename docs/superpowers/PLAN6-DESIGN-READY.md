# Plan 6 Design Ready Pack and Red Decision Packet

Status: **APPROVED / SETTLED** (2026-07-12)

This is workflow evidence, not design authority. After the Red packet is decided and the
independent review is settled, the accepted result is folded into the Plan 6 spec and the
design corpus where required.

## 1. Mission brief

Plan 6 implements the first network ingest path: R2 HTTP ingress plus the R19 ingress-side
security boundary. Success means that a third-party sender can follow the contract document,
authenticate with a per-device credential, send measurements with a three-command `curl`
journey, receive a durability-correct acknowledgement, and be throttled without accidental
data-loss semantics.

The same plan closes the currently exposed initial-admin ownership race before adding another
listener. It does not implement MQTT ingest, the pairing-window path, or encrypted snapshot
containers.

Risk classification: **Large / Red**. The work changes authentication, secrets, public wire
contracts, restore behavior, and a trust boundary. Plan 6 stays on `gpt-5.6-sol/high` through
final settlement.

Acceptance outcomes:

- no unauthenticated network ingest surface;
- no network endpoint can establish initial admin ownership;
- an ingress principal, not sender-controlled envelope metadata, determines identity,
  authorization, rate accounting, deduplication ownership, and audit attribution;
- throttling is retryable and never becomes a terminal `rejected` acknowledgement;
- reset, recovery, and restore cannot silently resurrect old administrative authority;
- replacement explicitly decides continuity for logical gateway identity, TLS pins, and every
  credential class;
- the normal one-box journey remains short, while fleet repetition and recovery costs are
  explicit rather than hidden.

## 2. Constraint ledger

### 2.1 Facts and settled authority

| Constraint | Authority / observed implementation | Consequence |
|---|---|---|
| R2 is a site-LAN network ingress with authentication, rate limiting, and hostile-input resistance. | [`../redesign/responsibility-ledger.md`](../redesign/responsibility-ledger.md) R2/R19; D11 | A listener without the complete security boundary cannot ship. |
| Network ingress is off by default and internet exposure is forbidden. | D11 decisions 2 and 7 | Listener enablement and bind changes are construction-tier R14 operations. |
| HTTP precedes MQTT; pairing-window ingress is a later UI-coupled plan. | D1; Plan-5 deferred ledger item 6 | Plan 6 has no unauthenticated ingress exception. |
| Initial admin ownership is local/per-card only; network box claim is rejected. | User decision recorded in `active-ledger.md` | Setup mode must not grant an unauthenticated network actor ownership. |
| Existing control API defaults to enabled on `0.0.0.0:8443`; setup mode permits unauthenticated passphrase creation and selected operations. | `iotkit-gateway/src/config.rs`; `api/routes.rs`; `api/auth_layer.rs` | This race must close before distribution or additional ingress exposure. |
| Operator sessions are rows in `operator_tokens`; passphrase reset currently does not revoke them. | `core/ops/src/auth.rs` | Local recovery does not currently remove an attacker's already-issued session. |
| Wave-0 snapshot restores only registry/device state, renews the ledger epoch, and bumps the collector generation. Auth tables are not restored. | `iotkit-gatewayctl/src/cmd/snapshot.rs` | Future inclusion of secrets must not weaken the current fail-closed auth result. |
| A backup cannot contain revocations that happened after it was created. | Information constraint | Cross-box restore cannot safely preserve credential validity without a newer external authority. |
| Gateway identity is generated per box and must not be baked into a shared image. D8 scopes external records by `(gateway_identity, epoch, seq)` and describes restore as a new epoch. | R22 / D8 decision 5 | Whether a replacement continues the logical gateway identity or enrolls as a new source must be explicit. |
| The current D2 restore contract requires encrypted snapshots to carry TLS private material, device-token hashes, and operator-token hashes so replacement can reconnect and an operator can enter immediately. | D2 §3.5 | The recommendation preserves TLS/device continuity but deliberately supersedes active operator-token restoration: local admin recovery and operator-token reissue are required. |
| Ingress device tokens are long-lived; revocation is human-only because it silences a sensor. | D11 decision 3 | Restore-time invalidation is an exceptional recovery consequence and must be explicit. |

### 2.2 Fixed Plan 6 scope

In scope: separate `iotkit-ingest-http` crate, `IngestPrincipal`, device-token lifecycle,
subject-scope enforcement, HTTP admission limits, token-bucket throttling with hysteresis,
freshness rejection, bounded staging, acknowledgement contract completion,
`docs/ingest-contract.md`, construction-tier listener operations, and control-plane ownership
closure.

Out of scope: MQTT wire throttling, pairing-window registration, `signed_seq` and
`provisioned_key`, measured production tuning, rich R23 UI, R22 encrypted snapshot containers,
and OS-image/image-writer/batch-label tooling. Encrypted containers remain Plan 6.5; distribution
tooling remains a separately approved distribution deliverable.

### 2.3 Assumptions and facts still requiring implementation probes

- Plan 6 can use the already-shipped local `gatewayctl passphrase reset`; it does not assume an
  image provisioner exists.
- A future provisioner can be designed only after its per-card binding, secret custody, batch
  duplicate detection, and operator handoff are proven. No runtime preseed importer is assumed
  in Plan 6.
- No TPM, cloud revocation authority, or always-available site server is assumed.

### 2.4 Fact, value judgment, and implementation choice

- **Fact:** an old backup cannot prove that a credential was not revoked later.
- **Approved user-value judgment:** replacement usability outweighs automatic invalidation of
  restored device credentials. The accepted cost is that a post-backup device-token revocation
  can roll back when no newer external authority exists.
- **Approved user-value judgment:** preserve D2 continuity for logical identity, TLS pins, and
  device credentials; do not claim rollback protection that the available authority cannot
  provide.
- **Fact:** accepting a bootstrap secret over the LAN is still a network claim, even if the
  secret originated locally.
- **Already approved value judgment:** initial ownership is established locally/per-card.
- **Recommended implementation choice:** Plan 6 uses the local CLI as the sole ownership
  producer and has no setup-token or preseed-import network/runtime path. Easier per-card batch
  provisioning is a named distribution deliverable, not an assumed capability.

## 3. Proposed state machine

### 3.1 Ownership and startup

| State | Event | Guard / atomic action | Next state and exposure |
|---|---|---|---|
| `unowned` | boot | no implicit credential generation; presence of migrated DB alone is not ownership | `local_recovery_required`; network API does not bind |
| `local_recovery_required` | local `gatewayctl passphrase reset` | local OS authority; set passphrase and revoke every operator/session token atomically | `owned`; next start may bind |
| `owned` | concurrent local resets | SQLite serialization chooses an order; each reset atomically changes the hash and revokes all extant operator/session tokens | remains `owned`; only the final passphrase is usable |
| `owned` | passphrase reset | local authority; update hash, revoke all operator/session tokens, audit in one transaction | remains `owned`; all clients reauthenticate |
| `owned` | finite session authentication while wall time is unset or below the persisted auth-time floor | reject session; never lower the floor | remains `owned`; local recovery remains available |
| `owned` | TLS pair partial, mismatched, or corrupt | never regenerate silently; emit actionable local error | remains unbound until explicit rotation/recovery |

The gateway runtime never writes or accepts a setup secret and never exposes a network setup
route. “Fresh/virgin DB” is therefore not an authentication condition. Missing or corrupt DB
enters local recovery; it does not recreate a remotely claimable setup mode.

Finite sessions use a persisted nondecreasing auth-time floor. Session issue/authentication is
refused while wall time is untrusted or behind that floor; a backward jump never extends a
session. First trustworthy synchronization may advance the floor. Restart reloads it. Long-lived
non-expiring service tokens remain governed by revocation/auth epoch, not wall-clock expiry.

### 3.2 Snapshot restore

1. Stop the gateway and require an explicit local restore command.
2. Validate the snapshot. Current code checks only five section tables and can preserve an
   auth-only target; that is not pristine. The hardened path requires an absent target created
   exclusively by restore, or an exhaustive all-owned-state emptiness predicate.
3. Restore non-auth state in one transaction.
4. Mint a new ledger epoch and unpredictable **auth epoch**. Apply the user-selected material
   matrix below for logical gateway identity, TLS, and device credentials.
5. Never restore an active admin credential, operator token, or session. Clear any target-side
   auth state and establish the new auth epoch in the same transaction as restored DB state.
6. Commit, then enter `local_recovery_required`. Establish the new admin passphrase locally.

The full restore also writes and fsyncs a durable `restore-in-progress` fence outside both the
DB and the replaceable TLS paths. The fence binds the intended logical gateway/auth/ledger
generations to the selected TLS generation/fingerprint. Startup remains unbound until DB state
and a complete TLS pair match that record; interruption resumes/repairs the restore or remains
unbound. Plan 6.5 may choose staging, fsync, and rename mechanics, but may not weaken this crash
contract.

Response loss is handled by inspecting the committed epoch/state. Repeating restore against a
non-pristine target is refused. A power loss before commit leaves no accepted target; after
commit it leaves the new epochs and selected credential state together.

Restore material decision matrix:

| Material | Continuity option (current D2 intent) | Rollback-safe option | Recommendation |
|---|---|---|---|
| Ledger epoch | always new | always new | always new |
| Logical `gateway_identity` | preserve; Site-managed source continuity | new identity + reenrollment | preserve, because D8 uses epoch as the generation fence |
| TLS key/certificate | restore encrypted; existing pins continue | generate new; every client/device re-pins | preserve, but require explicit rotation if compromise is suspected |
| Admin hash | do not restore active | do not restore active | local reset required |
| Operator tokens/sessions | D2 currently restores hashes active for immediate operator entry | do not restore active | **supersede D2 §3.5**; require local admin recovery and operator-token reissue |
| Device-token hashes | restore active; later revocation can roll back | restore disabled/reissue every device | **approved: continuity**; loudly report the accepted rollback risk |
| Auth epoch | always new; restored device rows may be rebound only by the restore transaction | always new | always new |

Plan 6.5 owns encrypted carriage, restore staging/fencing mechanics, and recovery-passphrase
mechanics; this packet owns their crash contract and which materials may become active after
carriage is available.

### 3.3 Factory reset

Factory reset is distinct from admin recovery and snapshot restore.

- **Admin recovery:** preserve gateway identity, device/config/data state, and audit history;
  replace the admin passphrase and revoke all operator/session tokens.
- **Factory reset:** destructive installer/OS-layer reinitialization; erase the application DB,
  gateway identity, TLS private key, credentials, configuration secrets, and local data; create
  nothing that implies ownership. The next boot is `unowned`.

Factory reset is not an IoTKit network API or AI operation and is not smuggled through a general
R14 operation. Under D13, SSH root and physical/local root are equivalent; the recommended
contract allows either to invoke it. It requires the gateway service stopped, explicit
display of the gateway identity and custody consequences, and typed confirmation. Any upstream
enrollment for the old identity must be retired separately; the erased box cannot prove that
external cleanup occurred.

Because DB, identity, TLS, and config live across SQLite and the filesystem, reset first writes
and fsyncs a durable `reset-in-progress` marker outside the deletion set. Every restart that
sees it completes deletion or remains unbound; only a fully initialized `unowned` state removes
the marker. Boundary fault injection covers every deletion step.

## 4. Trust and secret provenance

| Material | Generator / first knower | Transfer and trust anchor | At-rest / logs / audit | Revocation, rotation, restore |
|---|---|---|---|---|
| Initial admin passphrase | local operator | local TTY or SSH-root invocation of `gatewayctl`; OS root is the anchor | Argon2 PHC only in DB; terminal input is a known current limitation; audit records actor/method only | local reset revokes all operator/session tokens; never restored active |
| Operator/session token | gateway CSPRNG; shown once to authenticated user | authenticated TLS session after ownership exists | SHA-256 hash; token ID only in audit; secret redacted | explicit revoke/expiry; all revoked on passphrase reset and restore |
| Device ingest token | gateway CSPRNG; shown once after human-approved device add/reissue | operator copies to that one device | hash plus device/scope/profile/flow class; no plaintext logs | human revoke/reissue; invalidated on hardware replacement/disposal; restore follows selected continuity or rollback-safe mode |
| TLS private key | gateway first initialization | fingerprint is verified out of band; encrypted snapshot transfer is Plan 6.5 | filesystem permission boundary; fingerprint is public | continuity choice in restore matrix; partial/corrupt pair fails closed; explicit rotation is audited |
| Gateway identity | gateway first initialization | enrollment presents public identity | local durable state; public ID in audit | preserve on recommended replacement restore with new ledger epoch; new on factory reset; never shared-image material |
| Auth epoch | gateway CSPRNG in the restore transaction | internal only | unique generation identifier; value is not secret | replace on restore; credentials from another epoch cannot authenticate |

## 5. User journey and cost

### 5.1 One box

Actually available Plan 6 path:

1. Flash/install the current gateway software and arrange local TTY or SSH access.
2. Boot; the network control API remains unbound because ownership is absent.
3. On the box, run `gatewayctl passphrase reset` and enter/confirm the passphrase through a
   non-echoed terminal prompt (Plan 6 must replace the current echoed `read_line` path).
4. Restart/start the gateway if it is not supervised to retry automatically.
5. Obtain and compare the TLS fingerprint locally, then open the UI/API and sign in.

There is no LAN-based rescue claim. SD remount alone is not claimed to run `gatewayctl`; it must
provide a bootable/local execution path.

### 5.2 Twenty to one hundred boxes

Plan 6 by itself repeats the five local steps per gateway. A same-site fleet additionally needs
the D8 Site-managed server and enrollment per gateway; many independent Standalone boxes do not.
There is no existing provisioner, install one-pager, systemd unit, label/export tool, or OS-image
pipeline in this repository. Before external distribution, a separately approved deliverable
must add per-card secret generation/custody, encrypted operator export, duplicate detection,
labels, public-ID success manifest, certificate check, restart handling, and Site-managed
enrollment assistance. A shared passphrase or identity in a common image remains prohibited.

### 5.3 Restore and loss

- A replacement box requires local admin ownership establishment again.
- Device-token work after restore depends on R-A: continuity avoids touching every device but
  can resurrect a post-backup revocation; rollback-safe restore requires reissue on every device.
- Preserving logical identity/TLS avoids Site-managed reenrollment and device re-pinning; choosing
  new material adds both steps.
- Losing the passphrase does not require deleting measurements/configuration: local admin
  recovery preserves them and invalidates sessions.
- Losing both local access/media authority and the passphrase is intentionally unrecoverable
  over the LAN.

Target-client impact: the three-command ESP32/`curl` ingest path is unchanged after a device
token has been provisioned. The extra friction belongs to installer ownership and disaster
recovery, not every measurement send.

## 6. Adversarial checks

1. **Two identical requests:** local passphrase resets serialize. Initial device issue permits
   one active token row. Reissue uses two slots: one current and one pending replacement; a
   concurrent second reissue is rejected while pending exists.
2. **Power immediately before/after commit:** credential, auth epoch, revocations, and audit
   event share the DB transaction. Factory reset uses the durable filesystem marker described
   above. Startup binds only after observing complete owned/TLS state.
3. **Success response lost:** token plaintext is never redisplayed. An initial issue whose
   response is lost is revoked by device/token metadata and reissued. Reissue is
   make-before-break: the old token remains current, the new token is pending, successful use of
   the new token plus human confirmation promotes it and revokes the old one. Abandonment revokes
   only the pending token. This bounded overlap is the recommendation; immediate break-before-
   make is an R-A alternative because it trades less overlap for sensor downtime.
4. **Clock unset/jumps/backwards:** ownership and auth epochs do not depend on wall-clock ordering.
   Finite session issue/auth fails closed until wall time is trustworthy and at least the
   persisted maximum-seen floor; rollback never lowers the floor. In-boot limiting/cooldown uses
   monotonic time.
5. **Old backup restored elsewhere:** new ledger/auth epochs are minted. Admin/operator/session
   authority remains inactive. Logical identity/TLS and device-token behavior follows the
   explicit R-A choice and emits the accepted rollback/reprovisioning consequence.
6. **Same-LAN attacker acts first:** no network claim route exists and the API does not bind in
   `unowned`/`local_recovery_required`, so timing gives no ownership path.
7. **ENOSPC/corrupt DB:** local reset cannot partially establish ownership; failure prevents
   bind. Missing/corrupt DB enters local recovery rather than network setup mode.
8. **Bootstrap replay:** Plan 6 has no preseed artifact or importer. Re-running local reset is an
   explicit root action that revokes sessions and is audited.
9. **Secret disclosure:** passphrase reset revokes all operator sessions; device-token disclosure
   requires human revoke/reissue and records a reason without recording the secret.

## 7. Traceability

| Invariant | Mechanism | Failure behavior | Verification |
|---|---|---|---|
| LAN actor cannot claim a box | no setup route; pre-bind owned-state gate | API remains unbound | startup integration test and network negative probe |
| Local recovery ejects existing sessions | passphrase update + revoke-all in one transaction | rollback leaves old passphrase/state intact | transaction fault-injection and old-token rejection test |
| Restore follows the approved authority contract | new auth epoch; admin/operator/session inactive; device tokens follow explicit continuity/rollback-safe mode | local recovery required; continuity mode emits rollback-risk status | old admin/operator rejection plus mode-specific device-token tests |
| Restore cannot mix DB and TLS generations | durable restore fence binds DB generations to TLS fingerprint/generation | startup remains unbound and resumes/repairs | fault injection before/after DB commit and TLS rename/fsync |
| Sender metadata cannot choose identity | `IngestPrincipal` constructed by auth layer | mismatch is terminal reject + intrusion audit | forged `envelope.source` and cross-scope tests |
| Throttling cannot delete sender spool | pre-body 429/Retry-After or deferred, never rejected | sender retries; no durability ack | wire conformance and spool-sender test |
| Listener enablement is deliberate | construction-tier typed R14 op; default off | no listener | config/operation authorization tests |
| Secrets do not leak | admin input is non-echoed; generated tokens display once; redacted types and hashes at rest | fail without echoing entered secrets | terminal-capture plus Debug/error/log snapshot tests |
| Factory reset is absent from IoTKit API/AI | local-root operation; SSH root explicitly counts | API/AI request has no route | API/catalog inventory plus SSH/local command authority tests |
| Clock rollback cannot extend a session | persisted auth-time floor + fail-closed trust gate | finite sessions rejected until time recovers | unset/sync/backward/restart negative tests |
| TLS loss cannot silently change pins | owned state requires a complete matching pair | gateway remains unbound | missing-key/cert, mismatch, corruption, rotation and restore tests |
| Device reissue cannot strand or multiply authority silently | current+pending two-slot state and explicit promotion/abandon | second concurrent reissue rejected; old remains until promotion | concurrency, lost-response, promotion and abandonment tests |

Required canon/document propagation after approval: amend D2 Phase 2–3 and its UI-only criterion,
D13 premise/decision 2 setup window and screen inventory, responsibility R19/R22 propagation,
the Plan 5 setup route/restore sections, `docs/architecture.md`, `README.md`, and the future
install one-pager. Browser commissioning remains available **after** local ownership; only
network establishment of ownership is removed.

## 8. Bundled Red decision record

The three decisions are bundled because choosing convenience in one place can reintroduce
ownership through another.

### R-A: Restore versus credential rollback

**Why Red:** restore semantics, authentication strength, and fleet recovery cost.

**Recommendation:** preserve the logical `gateway_identity` and encrypted TLS key/certificate,
mint new ledger/auth epochs, and invalidate admin/operator/session authority. For device tokens,
choose between the following two explicit contracts:

- **Continuity (current D2 intent, recommended for usability):** restore token hashes active so
  devices reconnect without touching 20–100 units. Accept and loudly report that a revocation
  made after the backup can roll back in Standalone mode. A newer site authority may overlay a
  revocation delta when available, but is not assumed.
- **Rollback-safe:** restore device-token metadata disabled and reissue every device. No
  post-backup revocation can be undone, but replacement is no longer automatic.

The recommendation also uses make-before-break device-token reissue: one current plus one
pending token, with promotion after successful use and human confirmation. Immediate
break-before-make is available only if avoiding temporary overlap is worth sensor downtime.
Plan 6.5 encrypts carried material; encryption alone does not solve rollback.

Alternatives:

- Change logical gateway identity/TLS on replacement: strongest separation from the old box,
  but requires Site-managed reenrollment plus every pinned client/device to re-pin and
  supersedes D2/D8 continuity.
- Preserve credentials only for same-box rollback using a non-snapshotted local counter: does
  not solve the required replacement-box case and creates two restore contracts.

### R-B: Factory reset meaning and authority

**Why Red:** irreversible data loss, identity lifecycle, and authority boundary.

**Recommendation:** keep admin recovery non-destructive; define factory reset as a separate
local-root full erase that creates a new logical gateway identity. D13 already treats SSH root
as equivalent to physical root, so either may invoke it; “not remote” means not exposed through
IoTKit API/AI/R14. A durable reset marker makes interruption resume-to-erase or remain unbound.
External retirement of the old identity remains an explicit site-side step.

Alternatives:

- Make “factory reset” auth-only: safer for data but dangerously ambiguous; use the explicit
  name “admin recovery” instead.
- Offer a network factory-reset operation with step-up: convenient but expands a destructive
  remote attack surface and conflicts with local ownership recovery.
- Require physical presence and reject SSH root: stronger presence proof, but deliberately
  narrows D13's existing root-equivalence and increases headless/fleet recovery cost.

### R-C: Local-only bootstrap product and UX boundary

**Why Red:** product scope and unavoidable installer/recovery steps.

**Recommendation:** Plan 6 implements only the existing local `gatewayctl` producer over local
TTY or SSH root; there is no setup-token or runtime preseed importer. This is five concrete
steps per box and is intentionally manual. Before external distribution, approve a separate
provisioning deliverable for per-card generation/custody, encrypted export, duplicate
detection, labels/manifests, certificate checking, restart handling, and enrollment assistance.

Alternatives:

- Possession-proof network claim: smoother browser onboarding, but reverses the already approved
  “no network box claim” principle.
- Bring image-writer/batch provisioning into Plan 6: better fleet UX sooner, but expands Plan 6
  into currently nonexistent OS-image/distribution infrastructure and delays HTTP ingress.

### Decision (approved 2026-07-12)

The user approved all three recommendations as one packet:

1. **R-A continuity:** preserve logical gateway identity, TLS material, and active device-token
   continuity; accept and report post-backup device-token revocation rollback; use
   make-before-break reissue. Admin/operator/session authority never returns active.
2. **R-B reset authority:** factory reset is a full erase invokable by SSH root or
   physical/local root, never by IoTKit API, AI, or a general R14 operation.
3. **R-C scope:** initial ownership is local-CLI-only in Plan 6. Provisioning and batch tooling
   are a separate pre-distribution deliverable.

This approval authorizes the direct consequences and required canon amendments above; it does
not authorize push, release, or a future external revocation service.

## 9. Next work after approval

Fold this record into the design canon and formal Plan 6 specification. Product implementation
remains frozen until the specification and contract-centered implementation plan settle.
