# Edge Node Computer Replacement Design

Date: 2026-07-29

Issue: [#92](https://github.com/w-pinkietech/iotkit/issues/92)

Status: approved on 2026-07-29

## 1. Purpose

An IoTKit Edge Node currently has no custody-preserving recovery path when its
computer is lost or replaced. The legacy `nodectl snapshot` is an inventory
transfer artifact: it excludes readings, the publication log, accepted cursor,
and ingest deduplication state. It also cannot be used when device credentials
exist. Treating that snapshot as a machine backup would silently discard the
state needed to explain which observations IoTKit Edge durably accepted.

This change adds a deliberate Edge Node replacement workflow. It keeps the same
logical `edge_node_id`, fences the failed computer and its old MQTT credential,
restores every recoverable local custody row, reconciles the old publication
epoch with IoTKit Edge, records any irrecoverable tail as an explicit potential
loss, and only then starts a new publication epoch on the replacement computer.

The result must remain safe when backup was never configured. Backup improves
the recoverable point but is not a prerequisite for normal data collection or
for an explicitly loss-bearing replacement.

## 2. Scope and non-goals

This design covers:

- optional, default-off encrypted backups of the custody-complete Edge Node
  SQLite database after removing deployment credentials from the offline
  snapshot;
- online consistent snapshot creation without stopping normal collection;
- operator-selected mounted filesystem destinations;
- restore into a new candidate path without overwriting a live database;
- a process-wide recovery fence on the replacement Edge Node;
- broker credential rotation/revocation evidence;
- durable recovery cases and idempotent permits on IoTKit Edge;
- deterministic reconciliation of the restored old publication epoch;
- an explicit potential-tail-loss decision when state cannot be recovered;
- a new publication epoch under the same logical `edge_node_id`;
- SQLite and PostgreSQL parity for IoTKit Edge recovery state;
- operator CLI, Console status/actions, audit, diagnostics, and a bilingual
  runbook;
- power interruption, corruption, clone, wrong-target, and stale-backup tests.

This design does not:

- back up an operating-system image, boot partition, installed packages,
  network configuration, MQTT private material, TLS private keys, or plaintext
  device tokens;
- add object-store protocols to IoTKit. S3, SMB, NFS, removable media, and
  another host are supported by mounting a filesystem outside IoTKit;
- make backup mandatory or silently fall back to local storage when a requested
  mount is missing;
- reuse the legacy plaintext snapshot as a replacement backup;
- let IoTKit provision or implement an MQTT broker;
- transfer an Edge Node between different IoTKit Edge deployments;
- infer exact observations that were created after the last durable evidence;
- treat a replacement as forensic erasure of the failed storage medium;
- automatically merge two concurrently operating clones.

## 3. Delivery decomposition

Issue #92 is the parent outcome. The implementation is too cross-cutting for one
reviewable pull request, so it is delivered through four child issues and draft
pull requests:

1. encrypted Edge Node database backup, fenced restore, optional scheduling, and
   mount validation;
2. IoTKit Edge recovery cases, broker credential fencing evidence, and recovery
   permits;
3. old-epoch reconciliation, new-epoch activation, and loss/gap audit;
4. recovery wire/OpenAPI/Console parity, integrated bilingual operations
   documentation, and the end-to-end fault matrix.

All new wire paths and Console mutations remain default-off behind the durable
recovery capability until all four slices are integrated. An intermediate
release may create and validate backup artifacts, but it must not claim that
computer replacement is complete. Closing #92 requires the combined operator
journey and release-gate evidence.

Slice 1 accepts a versioned recovery handoff for conformance and restore-drill
validation but does not produce a production handoff. Slice 2 owns recovery
case creation and the production handoff producer; operators are never told to
fabricate or bypass one.

## 4. Roles and authority

The workflow separates local secret handling from deployment-wide custody
authority:

| Role or component | Authority |
|---|---|
| Local root on the Edge Node | Configure backup, create/inspect/restore an artifact, supply a passphrase file, install a candidate database, and view local recovery diagnostics |
| IoTKit Edge `system_admin` | Start a replacement case, record broker-fence evidence, issue a recovery permit, accept potential tail loss, activate the new epoch, and cancel only before any fence evidence or permit is committed |
| IoTKit Edge `settings_admin` | Read replacement state and diagnostics, but cannot accept loss or activate a replacement |
| Operator/viewer | Read state, known boundaries, last backup health, and the next required action |
| Edge Node recovery runtime | Apply only a recovery-bound permit, reconcile the old epoch, and durably report results |
| MQTT Broker deployment | Rotate/revoke credentials and enforce exact-topic ACLs; it does not decide IoTKit custody |

Every mutation uses the owning typed dispatcher:

- Edge Node local and recovery mutations go through `edge-node/core/ops`;
- IoTKit Edge recovery mutations go through `edge/src/application/`.

CLI, MQTT, HTTP, and Console handlers are thin adapters and never write recovery
tables directly.

## 5. Backup capability

### 5.1 Optional configuration and health

Backup is unconfigured and disabled by default. An unconfigured Edge Node
continues to collect, retain, and publish normally. Diagnostics and the Console
represent backup readiness with one of four explicit states:

- `not_configured`: no backup destination or schedule is enabled;
- `healthy`: the latest scheduled artifact succeeded and is within the
  configured freshness target;
- `stale`: a valid artifact exists but is older than the freshness target;
- `failed`: the latest attempt failed or the configured destination could not
  be verified.

`not_configured` is not a process health failure. It is an operational recovery
warning with an explicit consequence: loss of the computer cannot
automatically recover the local ledger, unacknowledged publication rows, or
ingest deduplication state.

Backup scheduling is supplied by a reference systemd service and timer. The
CLI also supports manual creation. The schedule, retention count, freshness
target, destination, expected mount identity, and passphrase-file reference
live in an owner-only local configuration. The passphrase itself is read from
an owner-only file or systemd credential, never from argv, environment,
journal, success JSON, audit, or Git. Deployment instructions require separate
passphrase escrow and periodic restore drills.

### 5.2 Destination selection

The output destination is an operator-provided mounted filesystem that passes
the backup capability check. Typical candidates include a removable encrypted
SSD, a separate volume on the IoTKit Edge host, or an NFS/SMB/other remote
mount whose client and server combination supplies the required semantics.
Naming a filesystem type alone does not make it supported.

Configuration records an absolute destination directory and the expected mount
point and identity. `backup configure` captures a Linux mount source/UUID
identity, and every scheduled run revalidates it before snapshot work begins.
The destination must be a distinct current mount, owner-writable without
broader read access, and have sufficient free space. A deliberate storage
replacement requires an explicit reconfiguration so a changed identity cannot
be accepted accidentally.

`backup configure` probes create-new, owner-only mode, file `fsync`,
same-directory no-replace publication, parent-directory durability, and
read-back behavior without touching an existing artifact. A filesystem or
server that cannot provide those operations fails closed. A successful local
probe proves only the mounted interface; remote stable-storage, replication,
and media durability remain deployment properties and require restore drills.

If the mount is absent, replaced, read-only, too full, or permission-insecure,
the attempt fails. The CLI never writes to a same-named local directory and
never falls back to another path. The reference systemd unit also uses
`RequiresMountsFor=` or an equivalent ordering/availability guard.

### 5.3 Artifact creation

The replacement artifact is distinct from both the legacy inventory snapshot
and the IoTKit Edge server backup. It has an Edge Node-specific magic value,
artifact kind, and format version, with a conventional
`.iotkit-node-backup` suffix.

The exact outer header, encrypted manifest, protected handoff, and restore
receipt form one versioned recovery contract: paired English/Japanese current
documentation, machine-readable schemas/exported Rust types, checked-in
secret-free golden fixtures, and conformance tests must agree.

Creation performs these steps:

1. Open the running Edge Node database read-only through the storage layer.
2. Create a consistent SQLite online backup into an owner-only, non-backed-up
   tmpfs staging directory. Copying the database and WAL files is forbidden.
3. On the offline snapshot only, use a typed operation to clear any legacy
   plaintext deployment credential stored in SQLite, including
   `target_registry.credential_token`. The live database is unchanged. The
   artifact is custody-complete, not a credential-transfer mechanism.
4. Run schema/migration validation, require those credential fields to be
   empty, run `PRAGMA quick_check`, and run foreign-key
   validation against the snapshot.
5. Derive the authenticated inner manifest from the sanitized snapshot, never
   from the concurrently changing live database.
6. Encrypt the snapshot and inner manifest with Argon2id-derived key material
   and XChaCha20-Poly1305. The outer header is unencrypted but is authenticated
   as AEAD associated data. It contains the magic, artifact kind, container
   version, bounded KDF salt/parameters, and nonce needed to decrypt. Parameter
   bounds are checked before memory or CPU-intensive allocation.
7. Create a new mode-`0600` ciphertext temporary file in the destination
   directory, write and `fsync` it, atomically publish it without replacing an
   existing name, `fsync` the parent directory, then reopen and authenticate
   the published bytes.
8. Record success only after read-back verification and remove plaintext
   staging. A crash may leave only a non-authoritative
   temporary ciphertext file, never a named successful artifact.

Retention deletes only product-owned artifacts that have been fully
authenticated, are older than a newer verified artifact, and match the
configured node and retention policy. It never follows symlinks, deletes
unknown files, or removes the last verified artifact after a failed run.

The authenticated manifest contains:

- `artifact_kind`, container format, and database schema versions;
- unique `backup_id`;
- `edge_node_id` and the old `ledger_epoch`;
- snapshot creation time;
- local accepted cursor `C`;
- allocated publication high-water `B`;
- snapshot mode and an optional durable replacement-shutdown seal;
- database byte length and SHA-256 digest;
- counts needed to validate ledger, readings, publication, dedup, activation,
  and audit coverage.

No manifest surface exposes credential values or hashes. Normal status output
contains the artifact ID, time, size, node identity, epoch, and health only.

## 6. Restore and the process-wide fence

### 6.1 Candidate restore

Restore always targets an explicitly new path. It refuses a path that exists,
the configured live path, a symlink, an identity mismatch, or an artifact
already bound to a different recovery case.

The CLI:

1. authenticates and decrypts the complete container into owner-only tmpfs;
2. verifies the inner manifest, AEAD tag, database digest, supported schema and
   migrations, `quick_check`, foreign keys, node ID, epoch, and the expected
   recovery ID when one is supplied. For the selected old epoch, it also
   verifies `C <= B`, cursor/activation epoch agreement, contiguous
   publication rows for `C+1..B`, materializable readings, valid
   family/schema/content, allocator agreement with `B`, and absence of a
   conflicting active epoch;
3. creates a mode-`0600` candidate temporary file on the same filesystem as the
   requested final candidate path;
4. copies the verified database into that unpublished temporary path, opens it
   offline, creates a fresh random `candidate_instance_id` that was not present
   in the backup, and uses `core/ops` to transactionally install the recovery
   fence/receipt, including exact Edge ID, backup ID, handoff schema version,
   credential generation, and proposed epoch; rotate local authority; and
   clear stale listener authority;
5. forces the closed candidate into a self-contained durable SQLite file using
   offline journal/checkpoint handling, verifies that startup needs no
   unshipped WAL state, reruns integrity and fence validation, and `fsync`s the
   file;
6. no-replace renames the already fenced candidate to the requested candidate
   name and `fsync`s its parent directory.

Cross-filesystem restore therefore moves ciphertext and verified bytes but
never performs a cross-filesystem rename of the live database. A failed or
interrupted restore leaves the current database untouched. Incomplete candidate
files are not auto-promoted or auto-deleted.

Rename success, parent-directory `fsync`, and published read-back are separate
fault boundaries. After rename, a failure may leave a named candidate, but it
is already fenced and self-contained. An exact replay verifies the stored
receipt against the same artifact/handoff, re-syncs the parent, and completes
read-back; changed content is a conflict and is never overwritten.

Startup requires a valid recovery fence, receipt, and
`candidate_instance_id` before it interprets any restored activation state. A
named database that contains restored active state without that fence fails
closed before any normal task starts.

### 6.2 Startup gate

After minimal config parsing obtains the database path, the Edge Node
composition root probes durable recovery state through a read-only,
no-create/no-migrate connection before effective-config/adapter logging,
migration, identity mutation, listener binding, adapter spawning, or normal
background work. Only an already published `durably_fenced_candidate` may enter
`fenced_waiting_permit`; an unfenced restored database fails startup.

While fenced or reconciling, the following remain disabled:

- Input Adapters and the collector;
- authenticated HTTP ingest;
- local control API and browser sessions;
- new outbox allocation of every record family;
- retention and purge;
- quarantine release;
- commissioning smoke;
- descriptor changes and other normal configuration mutations;
- normal new-epoch publication.

The restricted recovery runtime may connect to the configured Broker with the
fresh replacement credential, receive recovery messages, publish bounded
recovery status/results, and replay specifically permitted old-epoch rows. It
cannot turn a restored active database directly into an active node.

For a no-backup replacement, local root uses a separate
`recovery initialize-empty` operation with the protected replacement handoff.
The handoff contains the nonsecret recovery ID, stable node ID, expected IoTKit
Edge ID, proposed epoch, and credential-generation reference. The operation
creates an empty fenced database with a fresh `candidate_instance_id`; it does
not import IoTKit Edge's descriptor replica or claim any device/series
continuity.

Restore rotates the local administrative authentication epoch, invalidates all
local admin/operator/session authority, and clears applied listener/TLS
authority so network services cannot bind from stale configuration. Stored
device-token hashes remain inside the fenced database because devices may need
to resume after recovery, but they are unusable while ingest is disabled and
must pass the normal generation and scope checks before re-enablement. MQTT
passwords, TLS private keys, and other deployment credentials are never present
in the artifact and must be freshly provisioned.

## 7. Broker credential fence

IoTKit does not implement or remotely administer a Broker. The repository
provides a reference local-root Mosquitto helper and an equivalent documented
procedure for other Brokers.

For a specific recovery case, the helper:

1. verifies the expected old credential generation;
2. revokes or rotates the old computer's credential;
3. provisions a fresh generation with the same node-scoped topic policy and
   recovery topics;
4. reloads Mosquitto and probes rejection of the old generation and successful
   authentication/ACL behavior of the new generation;
5. emits a nonsecret receipt bound to the recovery ID, node ID, old and new
   credential generations, a deliberately scoped nonsecret configuration
   revision, probe outcomes, and time.

The credential itself is transferred by the deployment's protected handoff,
not through IoTKit Edge or the receipt.

A privileged IoTKit Edge CLI/application operation validates and durably
records the receipt. For a non-Mosquitto deployment, the system administrator
records equivalent operator evidence through the same typed operation. IoTKit
Edge does not pretend it can prove external Broker state; it records who
attested the fence, which procedure produced the evidence, and which credential
generation the later permit requires.

No recovery permit is issued until fence evidence is present. If the old
computer returns, its revoked credential must fail Broker authentication. A
subscriber cannot observe which credential authenticated an MQTT publication,
so a payload's claimed generation is never trusted as Broker evidence.
Unexpected MQTT client takeover, candidate identity, or recovery content moves
the case to durable `conflict_hold`; broker authentication probes and the
privileged receipt remain the credential-generation evidence.

## 8. Durable recovery protocol

### 8.1 State machines

IoTKit Edge owns this durable case state:

```text
active(old)
  -> replacement_fencing
  -> fenced(E)
  -> candidate_binding
  -> reconciling_old
  -> old_epoch_converged(W)
     -> [ordinary] loss_decision_required -> loss_accepted
     OR [sealed final snapshot] no_loss_proven
  -> old_epoch_terminal
  -> recovery_activating(new)
  -> active(new)
```

The replacement Edge Node owns this durable state:

```text
unpublished_temp_candidate
  -> durably_fenced_candidate(candidate_instance_id)
  -> fenced_waiting_permit
  -> reconciling_old
  -> old_epoch_converged(W)
  -> waiting_loss_resolution
  -> old_epoch_terminal
  -> new_epoch_activation_pending
  -> active(new)
```

The waiting-loss state resolves only from a matching IoTKit Edge result that
records either `loss_accepted` or `no_loss_proven`. A scheduled live backup
followed by loss of the computer always requires loss acceptance, even after
all backed-up rows converge. `no_loss_proven` is limited to a snapshot carrying
a verified replacement-shutdown seal: local root first stops collection,
adapters, ingest, and allocation through a typed operation, durably seals the
old database against normal restart, and then creates the final offline
artifact. An artifact made from an equally complete recovered original
database may use the same offline seal procedure. The seal, manifest `B`, and
candidate must agree, and `B` must converge with `E`.

Any identity, candidate instance, credential generation, artifact, cursor,
range, content, command, or transition mismatch enters durable
`conflict_hold`. Recovery never repairs a conflict by deleting rows, issuing a
different node identity, or advancing SQL manually.

Every command has a unique ID and is idempotent. Replaying the same ID with the
same content returns the stored result; replaying an ID with different content
is a conflict.

The additive wire surfaces are:

```text
iotkit/v1/edge-nodes/{edge_node_id}/recovery/request
iotkit/v1/edge-nodes/{edge_node_id}/recovery/result
iotkit/v1/edge-nodes/{edge_node_id}/backup-status
```

Recovery request/result messages are QoS 1 and not retained. Backup status is a
non-custody, nonsecret, complete current-state report published QoS 1 retained.
It contains readiness, artifact ID/time, and bounded reason codes, but never a
path, mount source, passphrase reference, or credential field. All three topics
remain under the existing exact node-scoped ACL principle.

Retained backup status is advisory monitoring data. A stale retained report
from the failed Node never selects an artifact, proves its availability, or
authorizes restore/recovery; local artifact verification and the case-bound
manifest are authoritative.

### 8.2 Recovery case and permit

Starting a case freezes IoTKit Edge's old-epoch accepted cursor and records:

- `recovery_id`;
- `edge_id` and `edge_node_id`;
- old ledger epoch;
- expected backup ID or an explicit no-backup mode;
- failure-detection time and operator reason;
- actor and audit identity;
- current credential generation;
- proposed new ledger epoch.

Case preparation then produces the protected replacement handoff without a
candidate ID. Restore or `initialize-empty` consumes that handoff and creates
the candidate's fresh instance ID. Through the fresh fenced Broker credential,
the candidate reports a case-bound hello containing the instance ID and
nonsecret manifest identity. A `system_admin` verifies the local CLI output and
uses a compare-and-set typed operation to bind exactly that candidate ID to the
still-unbound case. A second or changed candidate is a conflict. No permit is
created or published until candidate binding is durable.

Case creation fails while IoTKit Edge has an unresolved archive restore,
`recovery_hold`, or cursor conflict for that node and epoch. `E` is usable as
recovery authority only after those states are resolved through their existing
typed operations.

The recovery permit is an additive versioned custody-contract message carried
on node-scoped recovery request/result topics. It is bound to:

- recovery and command IDs;
- exact authorized `candidate_instance_id`;
- exact node ID and old epoch;
- backup ID, or explicit `no_backup`;
- the Edge accepted cursor `E`;
- the broker-fence receipt and new credential generation;
- proposed new epoch;
- grant revision.

The Edge Node applies it only in `fenced_waiting_permit`, persists the exact
permit and result before acting, and rejects conflicting content. The permit
has no wall-clock expiry because a recovered Node starts clock-untrusted. It is
an immutable, one-time grant bound to one active recovery ID, one authorized
candidate instance, and one exact revision. Once durably issued it cannot be
replaced or revoked; a required correction moves the case to operator hold and
requires a new recovery case/candidate rather than publishing a superseding
permit. An exact QoS replay returns the stored result.

Every replay, cursor transition, loss result, and epoch transition carries the
exact permit ID and revision. IoTKit Edge accepts an effect only when they match
the case's immutable permit and current durable state. A delayed permit remains
safe because it is either still the one current grant or the receiving Node is
no longer in `fenced_waiting_permit`; there is no unseen higher revision that
silently invalidates it. MQTT PUBACK is transport evidence only. IoTKit Edge
advances recovery state only after committing the matching application result.

Two candidates restored from one artifact receive different instance IDs.
IoTKit Edge authorizes exactly one instance; sharing a credential, MQTT client
takeover, or presenting the same recovery ID does not authorize the other
candidate.

### 8.3 Old-epoch reconciliation

For a restored snapshot:

- `C` is the Edge Node accepted-through cursor recorded in the snapshot;
- `B` is the highest publication sequence allocated in that snapshot;
- `E` is IoTKit Edge's accepted-through cursor frozen when the case is fenced;
- `W = max(B, E)` is the highest position supported by either durable side.

The protocol follows a closed decision table:

| Condition | Required behavior |
|---|---|
| `E < C` | `conflict_hold`; IoTKit Edge contradicts a cursor the Node already treated as accepted |
| `C < E <= B` | Verify Edge fingerprints where required and advance/replay exact rows through `E`; no invented row is accepted |
| `E > B` | Only the matching permit may advance the local accepted cursor from `B` through `E`, because IoTKit Edge already has those rows |
| `B > E` | Publish the exact stored rows `E+1..B` through the normal raw validation and custody transaction |
| `B = E` | No old-epoch row transfer is needed |

Concretely, the Node recovery publisher first submits its exact stored rows
`C+1..min(B,E)` as case-bound replays. IoTKit Edge compares each existing raw
fingerprint; a mismatch is a conflict. Each correlated custody acknowledgement
remains exactly the submitted batch's `cursor_end`, as required by the current
contract. Frozen `E` travels in recovery state, not in a custody ack. If
`E > B`, a separate permit-bound operation authorizes the Node to advance its
local accepted cursor from `B` through `E` without inventing local rows. If
`B > E`, the Node submits the exact stored rows `E+1..B` through the normal
custody transaction.

The old epoch becomes converged only when:

- both sides have durably converged through `W`;
- all backup-contained unacknowledged rows have either been exactly accepted or
  matched to Edge's existing fingerprints;
- no old-epoch publication remains claimable by the supported recovery path;
- no identity, sequence, or content ambiguity remains.

Convergence does not make the epoch terminal. The epoch becomes terminal only
after IoTKit Edge also commits either the required loss acceptance or the
limited `no_loss_proven` evidence and the Node commits the matching result.
Later-recovered media may still contain the declared unknown suffix; it cannot
be claimed by the already completed replacement path.

During reconciliation, IoTKit Edge accepts only the case-bound old epoch and
expected ranges. It does not run semantic or output backfill for an exact raw
replay already stored. Newly accepted old rows follow the normal future
semantic/output rules once, using their existing global
`(edge_node_id, ledger_epoch, pub_seq)` identity.

### 8.4 Epoch-scoped publication allocation

The current Edge Node `publication_log` uses a database-global SQLite
`AUTOINCREMENT` primary key. That cannot satisfy the custody contract's
requirement that a new epoch starts at publication sequence 1.

The schema changes to an epoch-scoped identity and allocator:

- publication rows are uniquely keyed by `(ledger_epoch, pub_seq)`;
- a durable per-epoch allocator owns the next sequence;
- migration preserves every existing row and the active epoch's next value;
- all joins, purge, ack, batch, diagnostics, and tests use the composite
  identity;
- allocation and record insertion remain in the same serialized transaction.

The proposed new epoch has no allocation until old-epoch terminal state and the
matching activation permit are durable. Its first allocation is exactly 1.

### 8.5 New epoch activation

After the old epoch is terminal, IoTKit Edge durably authorizes the proposed new
epoch under the same `edge_node_id`. The first record is publication sequence 1
and is a versioned recovery `epoch_start` annotation containing:

- prior epoch;
- recovery ID;
- backup ID when present;
- loss-decision ID when potential loss was accepted.

The custody contract adds this recovery annotation version and shared
accept/reject fixtures. Existing non-recovery annotation decoding remains
supported. No measurement, quarantine release, commissioning record, or
pre-transition reading can precede the recovery epoch-start record or be
backfilled across the boundary.

Only after IoTKit Edge commits sequence 1 and returns the matching
`accepted-through` may the Node move to `active(new)`, enable normal services,
and allow future collection. An interruption at any transition repeats the
same durable command and record rather than creating another epoch.

## 9. No-backup replacement and potential loss

If backup was not configured or no artifact survives, the replacement starts
from a new empty recovery candidate. It does not reconstruct Edge Node ledger,
unacknowledged publications, deduplication claims, device-token hashes, or
local administration from IoTKit Edge's descriptor replica.

IoTKit Edge still retains its accepted raw history through `E`. The operator
must:

1. fence the old broker credential;
2. explicitly choose `no_backup`;
3. accept the potential tail loss;
4. recommission devices and credentials;
5. activate a new epoch under the same logical node ID.

No-backup recovery preserves only the stable collection-node ID and the raw
history already held by IoTKit Edge. Recommissioned devices and series receive
new local identities. The existing device replace/retire operations cannot
reconstruct continuity because their authoritative old ledger is absent.

Loss acceptance is a `system_admin` typed operation requiring the exact node
ID and a nonsecret reason. It records a separate
`potential_edge_node_tail_loss` fact, not the existing IoTKit Edge archive
restore `archive_lost` fact.

For both stale-backup and no-backup recovery, the durable metadata states:

- data is known complete through `W`;
- the potential missing publication range begins at `W + 1`;
- the upper publication bound is unknown;
- exact missing record count is unknown;
- event-time start and end are unknown;
- backup creation time (when present) through failure-detection time is an
  operational evidence window, not an event-time claim.

For `no_backup`, `C` and `B` are unavailable rather than reported as zero, and
the known watermark is defined as `W = E`. The missing lower bound is therefore
`E + 1`; this does not assert that the failed Node allocated no later rows.
There is no old-row replay phase. Wire and Console models represent `C` and `B`
as absent values, not numeric zero.

`W` describes the custody publication stream only. The same recovery fact also
states that post-backup local-only ledger changes, quarantine rows, ingest
deduplication claims, and device retry state are unknown. A restored dedup table
is authoritative only as of the snapshot. The current ingest contract has no
signed spool sequence or generation, so IoTKit cannot mechanically prove a
fresh retry boundary.

Each contract-native HTTP principal therefore remains disabled until the
operator records one explicit, audited choice:

- `pre_failure_spool_disposed`: the device spool was reset/disposed through
  the device's own procedure; possible unsubmitted contents are covered by the
  loss decision; or
- `resume_with_duplicate_risk`: the spool is resumed and duplicate ingestion
  of post-backup Envelopes is explicitly accepted under the new epoch.

Reissuing a token alone is not a fresh dedup namespace because dedup authority
is `(stable_principal_id, envelope_id)`. This change does not introduce signed
sequence or spool-generation fields. Recovery metadata, diagnostics, and the
Console expose potential duplication separately from potential loss, and the
product does not call this exact-once recovery.

The gap annotation, audit entry, Console, diagnostics, history, and CSV expose
that uncertainty without fabricating an end cursor or timestamp. If the
original storage is later recovered, importing it is a separate conflict
resolution operation; the already activated new epoch is never silently
rewound.

## 10. IoTKit Edge data model and user experience

IoTKit Edge currently stores one activation/current cursor shape per
`edge_node_id`. Replacement requires explicit incarnation history:

- a stable collection-node record keyed by `edge_node_id`;
- immutable incarnation rows keyed by `(edge_node_id, ledger_epoch)`;
- one current-incarnation pointer;
- recovery case, broker-fence evidence, permits/results, loss decision, and
  transition audit;
- cursor and fingerprint ownership per incarnation;
- gap annotations linked to recovery and incarnation.

SQLite and PostgreSQL implement the same logical operations and contract tests.
Raw history already uses the global epoch-qualified identity, but activation,
descriptor selection, semantic boundaries, output boundaries, current value,
CSV, diagnostics, and Console queries must deliberately choose current-only or
all-incarnation behavior. No query may accidentally join old and new rows by
node ID and publication sequence alone.

When a restored ledger preserves the same series identities, existing semantic
rules and active output bindings continue future-only in the new epoch after
the sequence-1 recovery annotation. They do not reprocess old raw rows. An
empty/no-backup replacement creates no such continuity: recommissioned series
remain unmapped until the operator confirms or configures their meaning.
Descriptor revision is tracked per incarnation, and the first complete
descriptor for a new epoch starts that epoch's own revision history.

The Console adds a replacement panel under **Equipment / Collection Nodes**:

- current recovery state and old/new epoch;
- backup readiness and latest artifact time/ID, without filesystem paths or
  secrets for non-admin users;
- known cursors `C`, `B`, `E`, and `W` with plain-language explanations;
- broker-fence status;
- next required action;
- potential loss statement;
- conflict-hold reason and safe operator guidance.

Only `system_admin` sees mutation forms. Destructive-looking actions require
CSRF, reauthentication where the account policy requires it, exact node-ID
confirmation, closed reason codes, and a human-readable nonsecret reason.
Browser refresh and double-submit remain idempotent. The local passphrase,
backup path, and MQTT credential are never entered into the Console.

## 11. Error handling and restart behavior

Every recovery transition is transactionally persisted before its external
effect is reported as complete. Crashes are handled as follows:

- before a backup artifact rename: no successful artifact exists;
- after artifact rename but before health update: inspection discovers and
  verifies the immutable artifact, then idempotently records success;
- during restore: the live path remains untouched and the partial candidate is
  not used;
- before broker receipt commit: the helper may be rerun and must return the
  same generation outcome or a conflict;
- after permit commit but before publish: the same permit is retried;
- after raw commit but before `accepted-through`: the exact old row replay
  returns the committed cursor;
- after epoch-start raw commit but before its ack: the exact sequence-1 record
  is replayed;
- after Node activation but before Edge result receipt: the Node reports the
  same durable activation result.

`cancel` is allowed only in `replacement_fencing` before broker fence evidence
or a candidate permit is committed. After old-credential revocation, replay,
loss acceptance, or epoch activation, the system administrator may move the
case to an explicit operator hold but cannot restore `active(old)` or undo
audit/cursor facts automatically.

Storage failure, corruption, ENOSPC, clock uncertainty, timeout, cancellation,
or an unexpected message produces no success acknowledgement. A failed
recovery never automatically creates a new identity, changes an epoch, relaxes
ACLs, overwrites a database, deletes the old artifact, or accepts loss.

## 12. Security and secret handling

The implementation follows these non-negotiable rules:

- no token, token hash, password, passphrase, private key, connection string,
  customer identifier, or sensitive configuration appears in logs, errors,
  audit details, fixtures, issue comments, PRs, diagnostics, or `Debug`;
- artifact names and nonsecret IDs are bounded and generated by the product,
  not interpolated into shell commands;
- passphrase files, temporary plaintext, candidate databases, backup
  configuration, and credential handoffs are owner-only;
- decrypted bytes are minimized, staged on tmpfs, and zeroized where the Rust
  type permits;
- artifact KDF parameters have bounded accepted ranges to prevent malicious
  resource exhaustion;
- restore rejects symlink and no-replace violations;
- recovery topics and credentials retain exact node-scoped ACLs;
- every loss, fence, permit, conflict, and activation decision is audited
  without secret or payload content.

Mount sources, broker configuration details, and their raw digests are
sensitive-derived deployment metadata. Owner-only local diagnostics may use
them for exact verification, but ordinary logs, Console responses, and audit
summaries contain only bounded opaque revision IDs and closed outcome codes.

## 13. Verification

For every product behavior, the closest focused test is written before the
implementation. Required evidence includes:

### Backup and restore

- online snapshot while concurrent writes continue;
- manifest values derived from the snapshot;
- wrong passphrase, truncated/corrupt artifact, altered header/KDF bounds, and
  wrong artifact kind;
- wrong node, wrong epoch, wrong recovery ID, invalid `C/B`, unsupported schema,
  failed quick check, and foreign-key failure;
- cursor/activation epoch mismatch, noncontiguous `C+1..B`, missing reading
  materialization, invalid record content, allocator mismatch, and conflicting
  active epoch;
- missing/replaced/read-only/full mount and insecure permissions;
- filesystem capability-probe refusal, no local fallback, no overwrite, mode
  `0600`, file/parent durability, and post-publication read-back;
- interruption before/after each staging, encryption, rename, and candidate
  step;
- fence/authority rotation and self-contained SQLite validation before
  candidate publication;
- restore from another filesystem into a same-filesystem candidate;
- legacy plaintext snapshot refusal when credentials exist and no fallback in
  every replacement path.

### Fence and reconciliation

- restored active database cannot start collector, adapter, HTTP ingest,
  control API, retention, quarantine release, smoke, or normal publisher;
- old broker credential rejection and new credential exact-topic probes;
- missing, stale, mismatched, replayed, and conflicting fence receipts;
- recovery permit identity, candidate instance, immutable revision,
  delayed-delivery/current-state rejection, and idempotency without trusted
  wall time;
- all `E < C`, `E = C`, `C < E < B`, `E = B`, `E > B`, and `B > E` branches;
- fingerprint match/conflict, batch-end-correlated ack, and lost-ack replay;
- scheduled-backup loss cannot bypass `loss_decision_required`, while a final
  post-shutdown snapshot can produce `no_loss_proven`;
- interruption at every durable state transition;
- old computer returning before and after new-epoch activation;
- publication allocation starts new epoch at 1 and never collides across
  epochs;
- epoch-start is first, exact, idempotent, and not backfilled.

### Loss and product behavior

- backup disabled does not affect normal collection or health readiness;
- `not_configured`, `healthy`, `stale`, and `failed` diagnostics;
- no-backup recovery cannot claim ledger, dedup, credential, or series
  continuity;
- no-backup wire/UI uses absent `C/B`, `W=E`, and no replay;
- per-principal spool disposal and duplicate-risk acceptance;
- loss lower bound and unknown upper/event-time bounds are rendered exactly;
- no reuse of archive-restore loss commands or semantics;
- SQLite/PostgreSQL recovery-operation parity;
- history, current value, semantic, output, CSV, diagnostics, and Console
  queries across two epochs;
- authorization, CSRF, confirmation, audit, double-submit, and browser
  responsive journeys.

### Repository and field evidence

- custody-contract shared fixtures and conformance tests;
- `scripts/check-layers`, `scripts/check-source-layout`, and bilingual OKF
  documentation checks;
- focused crate and contract tests after each slice;
- `scripts/test-edge-console-frontend.sh` and
  `scripts/test-edge-console-e2e.sh` for the operator journey;
- `scripts/verify.sh` before each Rust behavior PR is handed off;
- `node scripts/battle-tested-review.mjs select --base origin/master`, with
  BT-004 reviewed explicitly;
- the host release gate in a new report directory after all slices integrate;
- target Raspberry Pi tests that cut power during backup, restore-candidate
  creation, reconciliation, and epoch activation, then verify restart state.

Injected process failures are required but do not replace physical power-cut
and storage-removal evidence for the release candidate.

## 14. Documentation and compatibility

The implementation updates both language versions of:

- the product model and system architecture where incarnation ownership and
  recovery components change;
- the Edge Node custody contract and shared wire fixtures;
- the installation/recovery runbook with optional backup configuration,
  restore, broker fencing, no-backup replacement, and restore drills;
- diagnostics and Console operator guidance.

The current contract text that defers encrypted Edge Node replacement,
reactivation, node-ID reuse, and clone handling is removed only when its
replacement behavior and conformance evidence land. The legacy inventory
snapshot remains available for its existing narrow purpose and is explicitly
named as non-custody-preserving.

This file records the reviewed design but is not product authority. No
implementation slice may enable behavior that contradicts the current OKF
corpus. The bilingual custody and ingest contracts, exported wire types,
schemas, shared fixtures, and conformance tests must change together in the
contract slice before recovery messages, ID reuse, restored dedup behavior, or
new-epoch activation can be enabled.

This is an additive pre-1.0 product capability and will be called out in the
next appropriate `0.MINOR.0` release after all child PRs integrate. The design
and intermediate implementation PRs do not independently bump the workspace
version, create a tag, or publish a release.

## 15. Completion criteria

Issue #92 is complete only when:

1. a running Edge Node can produce a verified encrypted full-database artifact
   without exposing secrets;
2. restore creates a validated fenced candidate and never overwrites a live
   database;
3. old and new credentials, recovery IDs, epochs, artifacts, and cursors cannot
   be confused or silently merged;
4. every recoverable unacknowledged row converges under the `C/B/E/W`
   algorithm;
5. every irrecoverable tail is accepted explicitly and represented with honest
   unknown bounds;
6. the same node ID starts a new epoch at publication sequence 1 only after the
   old epoch is terminal;
7. backup-disabled deployments have a documented, tested, explicitly
   loss-bearing recovery path;
8. SQLite/PostgreSQL, contract fixtures, Console, CLI, bilingual documentation,
   battle-tested review, host integration, and physical failure evidence agree.
