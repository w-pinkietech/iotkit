# Task 5 report — backup orchestration, reconciliation, status, and retention

Issue: #113
Base: `dd3b6a7`

## Outcome

Implemented library-only Edge Node encrypted backup orchestration in
`iotkit-core-recovery`:

- `create_backup`, `inspect_backup`, and read-only `backup_status`;
- typed `recovery.backup.begin`, `recovery.backup.complete`, and
  `recovery.backup.record_preflight_failure` dispatcher operations;
- one stable config-adjacent nonblocking recovery-operation guard acquired
  before owner-config/source/artifact access and held through snapshot,
  receipt, encryption, publication, readback, completion, retention, plaintext
  cleanup, and return;
- durable `absent -> started -> success|failed` and `absent -> failed`
  transitions with identical replay idempotence and terminal conflict refusal;
- exact recorded-basename restart reconciliation, with missing, invalid, or
  mismatched artifacts closed as `interrupted` and no adoption of unrelated
  artifacts; reconciliation runs before fresh-backup capacity/probe checks and
  directly fsyncs the held destination before recording success;
- read-only `not_configured`, `operation_busy`, `healthy`, `stale`, and
  `failed` status, including latest-failure precedence and a last verified
  success;
- owner-only, regular, single-link, exact-product-name plaintext cleanup;
  unsafe and near-match entries are preserved and reported;
- receipt-name-bound retention after success only.

The online snapshot is created before `begin`, so it never contains its own
in-progress receipt. Encryption uses the Task 3 held destination capability
and anonymous ciphertext inode; post-link uncertainty preserves the exact
ciphertext and leaves the attempt `started`. Every post-publication return
still attempts exact owner-verified plaintext cleanup.

The source database device/inode and snapshot manifest node/epoch must continue
to match the source opened before snapshot work. Durable state accepts only the
closed Task 5 reason vocabulary. Fault injection is implemented through a
production-neutral internal hook; every test implementation remains outside
product `src/`.

No CLI, restore installation, runtime startup integration, release change, or
IoTKit Edge server backup behavior was added.

## TDD evidence

### Public API RED

Before production implementation:

```text
cargo test -p iotkit-core-recovery --test backup_contract
error[E0432]: unresolved imports
  BEGIN_BACKUP_ATTEMPT_OP
  COMPLETE_BACKUP_ATTEMPT_OP
  RECORD_BACKUP_PREFLIGHT_FAILURE_OP
  backup_status
  inspect_backup
```

### Linux create RED

After the public shape existed but before orchestration:

```text
encrypted_backup_round_trips_custody_state_and_redacts_receipt_audit
called Result::unwrap() on Err value: PlatformUnsupported
```

The GREEN test creates a live custody database, makes an online sanitized
snapshot on tmpfs, publishes and reauthenticates the encrypted artifact, and
checks `C=1`, `B=3`, quarantine counts, a durable success receipt, and audit
redaction.

### State replay RED

```text
exact_terminal_completion_replay_is_idempotent_and_audit_stays_redacted
Err value: PreconditionFailed("backup_attempt_terminal")

exact_preflight_failure_replay_is_idempotent_and_cannot_become_success
Err value: PreconditionFailed("backup_attempt_conflict")
```

Both identical replays are now idempotent; conflicting terminal content and a
preflight-failed attempt becoming success remain forbidden.

### Exact-name retention RED

The reconciliation/retention regression initially left the old exact receipt
artifact because Task 4 directory scanning used `dup()`, whose shared directory
offset had already been exhausted by the cleanup-marker scan:

```text
assertion failed: !exact_path.exists()
remaining artifacts:
  new-recorded.iotkit-node-backup
  unreferenced.iotkit-node-backup
  backup-reconcile.iotkit-node-backup
```

Cleanup-marker and retention enumeration now open `"."` relative to the held
dirfd, producing an independent stream without reopening configured paths.
Task 5 retention additionally matches both authenticated backup ID and the
exact successful receipt basename. The GREEN regression removes the older
recorded artifact while preserving the authenticated but unreferenced copy.

### Post-review regressions

Independent review initially found four Important and two Moderate issues:
source-path replacement, reconciliation ordering, open-ended reason codes,
missing crash-boundary coverage, a first-use status-lock race, and silent
near-match cleanup. Regression coverage now exercises all six.

A follow-up review found that reconciliation still required a fresh create
probe under storage pressure. The fix separates fresh-write verification from
exact-artifact reconciliation. A second follow-up caught that removing the
probe also removed its directory sync. Reconciliation now directly fsyncs the
held verified destination descriptor before committing success. An injected
sync failure leaves the attempt `started`; a later retry proves durability and
then records success.

Final independent review source: `/root/task5_backup/task5_review`. Result:
no release-blocking correctness or security finding.

### Parent review gap closure

A later independent parent review found three Important and one Moderate gap.
Each received a focused regression:

- A manually constructed crash state containing both the exact published
  ciphertext and an exact named plaintext stage initially returned
  reconciliation success while leaving plaintext. Reconciliation now opens
  the configured staging directory once, cleans through the held descriptor
  before committing success, and returns `cleanup_required` while preserving
  unsafe near-matches and leaving the attempt `started`.
- A directly inserted failed receipt containing
  `customer-secret-free-text-must-not-leave-storage` initially surfaced as a
  `BackupReadiness::Failed` reason. Status now validates the complete recovery
  schema/state read-only before projection and returns only the closed
  `InvalidStartupState` error; the stored text is absent from Debug output.
- The old create/inspect API shape could not name the configuration path and
  therefore locked beside staging. The compile RED required config-path APIs.
  Create and inspect now acquire the same Task 4 config-adjacent exclusive lock
  before owner-config loading; status observes that lock before config
  existence/loading. Configure-held create/inspect and create-held configure
  both return `operation_busy` before effects. Restoring the old staging lock
  made the reverse integration test observe `Ok(())`; restoring the fix made
  it green.
- Same-millisecond rows initially selected a random-ID lexical winner. Status
  now orders both the latest attempt and last success by durable SQLite
  insertion order (`rowid`) after their timestamp. Reverse-lex fixtures prove
  the later failure wins and the later inserted success is `last_verified`.

## Coverage

- Full encrypted custody snapshot and inspect round trip.
- C/B, activation, publication, quarantine, sanitized deployment credential,
  and audit behavior.
- Private operation parameter redaction and empty audit targets; node, Edge,
  backup, attempt, epoch, path, passphrase, token, and digest values are absent.
- Begin/complete/preflight state validation, identical replay, and terminal
  immutability.
- Lock loser before source open and active-lock `operation_busy` status.
- Cross-operation config-adjacent exclusion for configure/create/inspect and
  first-time configure/status.
- Status not configured, healthy, stale, failed with prior success, and
  non-Linux fail-closed behavior; invalid stored receipts never project free
  text.
- Exact-name successful reconciliation, missing-name interruption, and refusal
  to adopt or retain by unreferenced name; reconciliation remains possible
  without fresh capacity or create permission but requires parent durability.
- Retention only after a newer durable success receipt.
- Exact private plaintext cleanup; symlink, hardlink, broad-mode, and near-name
  preservation.
- Source path replacement, manifest identity mismatch, closed reason codes,
  first-use lock races, after-begin crashes, publication/readback/receipt
  uncertainty, reconciliation parent-sync failure, crash-plaintext cleanup,
  and same-timestamp insertion ordering.
- Existing Task 3 write/link/file-sync/parent-sync uncertainty tests and Task 4
  readback/substitution/retention tests remain part of the full recovery gate.

## Verification

WSL Ubuntu-26.04:

```text
cargo test -p iotkit-core-recovery --test backup_contract
4 passed, 0 failed

cargo test -p iotkit-core-recovery backup
22 passed, 0 failed (plus 2 filtered contract tests)

cargo test -p iotkit-core-recovery
101 passed, 0 failed, 1 ignored; backup contract 4 passed

cargo clippy -p iotkit-core-recovery --all-targets --no-deps -- -D warnings
exit 0
```

Windows:

```text
cargo test -p iotkit-core-recovery
54 passed, 0 failed, 1 ignored; backup contract 3 passed

cargo clippy -p iotkit-core-recovery --all-targets --no-deps -- -D warnings
exit 0
```

The Windows build continues to print the pre-existing dependency warning for
the unused `mode` variable in `iotkit-core-ops`; recovery's strict no-deps
Clippy gate is clean.

Repository gates:

```text
cargo fmt --all -- --check                       # exit 0
node scripts/check-okf-docs.mjs                  # passed
python scripts/check-layers                      # passed
python scripts/check-source-layout               # passed
node scripts/battle-tested-review.mjs check      # passed
git diff --check                                 # exit 0
```

The selector routed BT-001 through BT-003 from the complete issue branch.
Task 5 received independent semantic review for BT-002/BT-003 plus
recovery-specific publication, source identity, plaintext, reconciliation,
status, and retention concerns. This host verification does not claim physical
power-cut, SD-card, or storage-controller evidence.

## Residual concerns

- Linux O_TMPFILE/linkat/tmpfs capability remains a deployment/release gate;
  WSL proves this host path only.
- Physical power-loss durability remains BT-002 evidence, not a per-PR host
  claim.
- Scheduling, CLI owner-file loading, restore, runtime fencing, and operator
  runbooks belong to later tasks and are intentionally absent here.
