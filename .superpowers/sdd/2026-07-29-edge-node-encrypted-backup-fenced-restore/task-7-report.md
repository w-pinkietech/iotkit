# Task 7 report — local-root Node backup CLI

Issue: #113

## Outcome

Implemented the local-root `iotkit-edge-nodectl backup` surface:

- `backup configure` writes owner-only schema-1 configuration and an escaped
  `RequiresMountsFor=` systemd drop-in as one guarded pair transaction, with
  explicit replacement required for existing configuration/drop-in files;
- `backup create`, `backup inspect`, and `backup status` expose stable JSON
  summaries without database paths, destination/mount identity, passphrase
  paths, database/artifact digests, or serialized private manifests;
- `backup restore` requires an owner-only schema-valid recovery handoff,
  carries the configured `--live-db` path into `RestoreRequest`, stages through
  a private `0700` directory, removes that staging directory on every return
  path, and returns the public fenced-candidate receipt;
- backup commands route before generic database selection/migration, so status,
  create, inspect, and restore do not create or migrate a missing live database;
- owner-only passphrase parsing accepts one terminal LF or CRLF, rejects any
  remaining CR/LF/NUL, invalid UTF-8, and scalar lengths outside 12–1024;
- the recovery inspector authenticates one held no-follow descriptor without a
  config lock or live-database open;
- every command, including the legacy JSON `snapshot` command, opens the
  current schema through `all_edge_node_migrations()`; only the snapshot JSON
  sections and command shape remain the compatibility surface.

The CLI error boundary emits only closed recovery reason codes and nonsecret
actions. No clap argument/config/passphrase debug representation is printed.
The drop-in target is opened nonblocking before type/owner/mode checks so a
FIFO cannot make configuration hang.

## TDD evidence

### CLI RED

Before adding the command parser and implementation:

```text
cargo test -p iotkit-edge-nodectl --test backup_cli
error: unrecognized subcommand 'backup'
```

The subprocess contract then passed after the early route and public recovery
API integration were implemented.

### Migration compatibility regression

The first complete-migration implementation left the snapshot
canonical-schema helper on the frozen pre-recovery migration list and failed
with `restore target schema or migration set is not canonical`. The CLI route
and snapshot canonical-schema check now both use
`all_edge_node_migrations()`, while the snapshot JSON sections and `R22 Wave 0`
manifest validation remain unchanged.

## Coverage

- command exposure and help without a raw `--passphrase` flag;
- early routing with absent config/live database and no side effect;
- configuration and drop-in creation, explicit replacement refusal, and
  nonsecret output;
- encrypted create/inspect/status summaries with C/B and artifact identity;
- conflicting candidate refusal, exact fenced-candidate replay, parent-sync /
  read-back receipt replay, and byte-identical receipt output;
- passphrase LF/CRLF, embedded control, invalid UTF-8, Unicode scalar bounds,
  owner-only mode, regular single-link descriptor validation;
- owner-only config and handoff loaders, bounded closed JSON, and recovery
  contract redaction;
- all existing `nodectl` unit and CLI tests, including snapshot compatibility
  tests and a subprocess init(v23) → export → restore → restore-status journey.

The battle-tested selector routed BT-001/002/003/004. Review covered custody
convergence, abrupt-loss boundaries, storage-pressure acknowledgement, and
computer replacement identity/loss boundaries. No physical power-cut, SD-card,
filesystem-controller, or deployment release evidence is claimed here.

## Verification

WSL Ubuntu-26.04:

```text
cargo test -p iotkit-edge-nodectl                         # 15 + 10 + 69 passed
cargo test -p iotkit-edge-nodectl --test backup_cli      # 10 passed
cargo test -p iotkit-edge-nodectl --test cli legacy_snapshot  # 1 passed
cargo test -p iotkit-core-recovery                         # 119 passed, 1 ignored; 4 contract passed
cargo test -p iotkit-core-recovery owner_passphrase_parser_accepts_one_terminal_line_ending_only # 1 passed
cargo clippy -p iotkit-edge-nodectl --all-targets -- -D warnings # exit 0
cargo clippy -p iotkit-core-recovery --all-targets --no-deps -- -D warnings # exit 0
cargo fmt --all -- --check                                  # exit 0
```

Repository gates:

```text
node scripts/check-okf-docs.mjs                         # passed
scripts/check-layers                                      # passed
python scripts/check-source-layout                          # passed
node scripts/battle-tested-review.mjs check               # passed
node --test scripts/tests/battle-tested-review.test.mjs   # 12 passed
git diff --check                                          # exit 0
```

On the Windows host, invoking the executable `scripts/check-source-layout`
wrapper directly exceeded the shell's 120-second dispatch timeout; running its
Python body explicitly produced the same `OK` result above. No repository
change is associated with that host-shell quirk.

The Windows host cannot compile this package because the pre-existing
`edge-node/apps/nodectl/src/cmd/passphrase.rs` unconditionally imports Unix
`std::os::fd` and libc termios/signalfd/poll symbols (40 baseline errors before
the new tests run). This task did not alter that unrelated portability path.

## Residual concerns

- WSL verifies Linux filesystem behavior only; tmpfs/O_TMPFILE/link/rename
  deployment capability and physical power-loss durability remain release or
  field gates.
- Slice 1 intentionally has no production recovery-handoff producer; the
  handoff is emitted by the later IoTKit Edge recovery case.
- The snapshot command intentionally keeps its frozen JSON sections and
  manifest validation, but its backing database is always opened and verified
  with the current Edge Node migration set.

## Parent review remediation — round 1

The following review paths were reproduced with focused tests before/after the
changes. The injected pause/crash seams exist only to make process and
filesystem boundaries deterministic in tests.

### Owner-only bounded readers

RED reproduced a status command hanging on an owner-only FIFO instead of
returning a closed error:

```text
cargo test -p iotkit-edge-nodectl --test backup_cli owner_only_readers_reject_fifo_symlink_and_hardlink_without_hanging
status FIFO reader did not fail closed promptly
```

GREEN uses one `O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC` descriptor, validates regular
single-link owner-only metadata, clears nonblocking only after validation, and
reads with `take(limit + 1)`. FIFO, symlink, and hardlink subprocess cases now
pass in 1 test. The growth-after-fstat regression also passes:

```text
cargo test -p iotkit-core-recovery bounded_owner_reader_rejects_growth_after_metadata_observation -- --nocapture
test config_tests::bounded_owner_reader_rejects_growth_after_metadata_observation ... ok
```

### Paired configure transaction

The sequential implementation had a mixed-pair failure window: configuration
could be published before the drop-in write failed. The paired failure matrix
and crash retry are now green:

```text
cargo test -p iotkit-edge-nodectl --test backup_cli configure_pair
2 passed
cargo test -p iotkit-edge-nodectl --test backup_cli concurrent_configure
1 passed
```

`configure_backup_pair` holds the config-adjacent operation guard over
preflight, marker, both publications, parent sync, and cleanup. The durable
marker contains only path hashes and phase state. Normal failure rolls back
both targets; a crash leaves the marker for deterministic retry recovery; a
second concurrent configure returns `operation_busy` without a mixed pair.

### Current migration set for snapshot commands

RED was the canonical-schema mismatch after the CLI opened the current schema
but the snapshot verifier still built an R22 migration set:

```text
cargo test -p iotkit-edge-nodectl --test cli snapshot_export_restore_round_trips_full_columns_and_renews_epoch
restore target schema or migration set is not canonical
```

GREEN removes the frozen DB-open path and uses `all_edge_node_migrations()` in
both the CLI route and snapshot canonical-schema verifier. The real subprocess
journey is covered by:

```text
cargo test -p iotkit-edge-nodectl --test cli subprocess_init_v23_snapshot_export_restore_and_status_regression
1 passed
```

### systemd path encoding

The prior drop-in wrote mount paths verbatim, so whitespace/backslash/quote/%
could be parsed as separators, escapes, or specifiers. The encoder now keeps
absolute path syntax and emits `\\xNN` for all non-safe bytes; non-UTF-8 paths
are rejected. Unit tests cover space, backslash, quote, percent, controls, and
UTF-8 bytes. The generated drop-in is embedded in a temporary service and
verified in WSL with `systemd-analyze verify` as part of
`create_inspect_and_status_emit_only_nonsecret_summaries` (1 passed).

### Restore staging and cleanup

Staging directories are created with `DirBuilderExt::mode(0o700)` before the
inode exists, then owner/mode verified. A post-create injected failure removes
the directory; umask-zero and cleanup tests pass:

```text
cargo test -p iotkit-edge-nodectl --bin iotkit-edge-nodectl backup
4 passed
```

## Parent review remediation — round 2

Round 2 closes the remaining paired-configuration and create-selection
failure paths. The marker is now a closed schema-2 state machine with
owner-only, no-follow bounded reads. It binds the request's canonical config
and drop-in request hashes plus both target path identities, records exact old
and published hashes, and rejects unknown fields, phases, txids, hashes, or
unexpected target/backup combinations without mutation.

Status checks the pending marker while holding the same observation lease as
the config existence check, so a crash after the old pair was moved cannot be
reported as `not_configured`. Published is the commit boundary: failures while
removing old backups or syncing cleanup retain the new pair and marker as
`cleanup_required`; an identical retry finalizes idempotently. A retry with
different arguments first finalizes the prior request and then refuses or
replaces explicitly instead of silently accepting stale arguments.

Create now selects the owner-only config and its configured passphrase under a
single recovery operation lease before running the backup. The deterministic
pause seam proves a concurrent configure receives `operation_busy`, while the
resulting artifact remains decryptable with the selected configuration and not
with the replacement passphrase.

Focused GREEN evidence (WSL Ubuntu-26.04):

```text
cargo test -p iotkit-edge-nodectl --test backup_cli -- --nocapture
16 passed, 0 failed

# Includes:
# - pending marker with missing config => cleanup_required
# - configured retry identity and explicit replacement
# - published cleanup failure / idempotent finalize
# - forged marker and symlink, hardlink, FIFO backup states
# - create selection vs configure race
# - all prior paired rollback and reader safety journeys
```

## Parent review remediation — round 3

Round 3 makes paired backup configuration recovery convergent across the
remaining publication and cleanup seams:

- durable `config_publishing` and `drop_in_publishing` phases retain exact
  transaction-bound temporary names and hashes;
- the exact systemd drop-in is prepared and parent-synced before the config
  rename, so a crash after either target rename can retry without synthesizing
  an unproven artifact;
- recovery permits only an exact retained temp or an already-exact target, and
  fails closed with `cleanup_required` for unexpected target/backup/temp
  combinations;
- finalization atomically renames the marker to an owner-only completion
  receipt and syncs it. Status ignores a valid receipt, while configure
  consumes it idempotently for the same request and rejects corrupt,
  mismatched, or marker-plus-receipt states;
- cleanup and rollback remain bound to exact old-backup provenance and the
  persisted transaction identity. No debug or error output exposes paths,
  credentials, or hashes.

### TDD evidence

The new RED cases covered crash after config rename, crash after drop-in rename,
completion-receipt final-sync uncertainty, and forged/stale phase states. The
focused GREEN command was:

```text
cargo test -p iotkit-edge-nodectl --test backup_cli -- --nocapture
20 passed, 0 failed
```

This includes a table-driven schema-3 matrix for `Prepared`,
`ConfigPublishing`, `ConfigPublished`, `DropInPublishing`, `DropInPublished`,
and `Published`, each with originally absent and present targets. Every
malformed row asserted `cleanup_required` and byte-for-byte preservation of all
tracked artifacts. Rename-crash retries and the completion receipt retry also
passed.

### Verification

Focused and broad Linux/WSL checks passed:

```text
cargo check -p iotkit-edge-nodectl -p iotkit-core-recovery
cargo clippy -p iotkit-edge-nodectl -p iotkit-core-recovery --all-targets -- -D warnings
cargo test -p iotkit-edge-nodectl -p iotkit-core-recovery
scripts/check-layers
scripts/check-source-layout
scripts/verify.sh
```

Package coverage was recovery 119 unit tests plus 4 contract tests and nodectl
15 unit, 20 backup CLI, and 69 CLI tests. The workspace gate also passed
`cargo test --workspace` and workspace Clippy with `-D warnings`. The WSL
worktree has a Windows absolute `.git` path, so `scripts/verify.sh` was run
with explicit `GIT_DIR`/`GIT_WORK_TREE`; the unqualified WSL invocation fails
before running checks because Git cannot resolve that host path.

### Battle-tested review and self-review

The selector chose BT-004 (Edge Node computer replacement loss boundary). This
round only hardens the config/drop-in publication transaction; encrypted
computer replacement backup/restore remains the catalog's explicit coverage
gap and is not claimed as solved here. Every recovery phase validates its
target parent before any rename or cleanup. `ConfigPublishing` validates both
exact temp transitions before it can rename the config, and
`DropInPublishing` additionally requires exact config old-backup provenance so
a forged marker cannot publish a half-pair.

### Integration

Round 3 was committed as:

```text
e8df756 fix(recovery): make backup configuration recovery convergent
```

No push or merge was performed.

## Parent review remediation — round 4

Round 4 retains the completion receipt as durable evidence for the current
configuration pair. Ordinary success and exact idempotent retries never unlink
that receipt. A replacement transaction leaves the prior receipt in place while
its pending marker takes precedence and blocks status/create. At commit, the
published marker atomically replaces the receipt; only the resulting
cleanup-only marker is unlinked.

The two possible marker-plus-receipt orientations are closed and recoverable:

- before exchange, the current pair matches the new marker and its old hashes
  bind the prior receipt;
- after exchange, the current pair matches the new receipt and its old hashes
  bind the prior receipt now stored under the marker name.

A parent sync follows receipt replacement before marker cleanup. Failure or
process crash after replacement, after its sync, after marker unlink, or after
the cleanup sync leaves either the marker, the durable receipt, or the strictly
related pair. The same no-replace retry converges without `DestinationExists`.
Different arguments still require explicit replacement.

The recovery core and nodectl now deserialize one closed `BackupPairRecord`
schema. It rejects unknown fields; noncanonical schema, phase, txid, hash, and
temporary-name values; and inconsistent optional fields. The core binds the
receipt to the actual config path and exact current owner-only, private,
regular, single-link config bytes. It opens receipt/marker FIFOs nonblocking and
rejects symlinks and hard links. Nodectl additionally binds the exact configured
drop-in path and bytes. The core intentionally cannot validate that second path:
the durable record stores only its hash, not the raw path. No record implements
`Debug`, and errors/status expose neither paths nor hashes.

### TDD evidence

The first RED cases were:

```text
cargo test -p iotkit-edge-nodectl --test backup_cli durable_completion_receipt -- --nocapture
# failed: ordinary success must retain durable completion evidence

cargo test -p iotkit-core-recovery completion_receipt -- --nocapture
# failed: malformed/stale receipt was accepted

cargo test -p iotkit-core-recovery receipt_and_marker_coexist -- --nocapture
# failed: strict post-commit cleanup state was rejected
```

The pending-state test also exposed that a valid crash in `Prepared` after the
old pair was moved aside did not resume. Recovery now restores the
receipt-proven old pair, removes and syncs the pending marker, and re-enters
preflight under the caller's explicit replacement policy.

Focused GREEN coverage includes:

- durable receipt after ordinary success and exact retry;
- different no-replace refusal and explicit replacement;
- valid old receipt plus pending marker blocking status/create and resuming;
- receipt replacement and marker-cleanup failure/crash boundaries;
- stale other-config path hash, malformed schema/hash/txid, unknown fields,
  forged temp names, and modified config bytes;
- receipt symlink, hard link, and FIFO rejection without mutation or hang;
- strict post-commit marker/receipt coexistence and invalid coexistence;
- exact nodectl drop-in path/content validation;
- all prior 20 backup CLI cases and the 12-state forged phase matrix.

### Verification

WSL Ubuntu-26.04:

```text
cargo test -p iotkit-edge-nodectl -p iotkit-core-recovery
# recovery: 122 passed, 1 ignored; 4 contract passed
# nodectl: 15 unit, 23 backup CLI, 69 CLI passed

cargo clippy -p iotkit-edge-nodectl -p iotkit-core-recovery \
  --all-targets -- -D warnings
cargo fmt --all -- --check
node scripts/check-okf-docs.mjs
scripts/check-layers
scripts/check-source-layout
node scripts/battle-tested-review.mjs check
node --test scripts/tests/battle-tested-review.test.mjs
```

All commands passed. `scripts/verify.sh` also passed with explicit WSL
`GIT_DIR`/`GIT_WORK_TREE`, including workspace tests and workspace Clippy with
`-D warnings`. The unqualified WSL Git path remains unable to resolve this
worktree's Windows absolute `.git` pointer; Windows `git diff --check` passed.

### Battle-tested and self-review

The selector chose BT-002, BT-003, and BT-004 for power-loss,
storage-pressure, and Edge Node computer-replacement concerns. Receipt exchange
precedes its parent sync; marker unlink follows that sync and never removes the
receipt. Injected sync failures return `cleanup_required`, never success. Tests
cover process aborts and injected filesystem failures, not physical power cuts,
SD-card/controller behavior, or every filesystem's deployment guarantees.
Encrypted computer replacement remains BT-004's explicit coverage gap.

The state orientation is determined only from closed records, exact current
pair hashes, and the new record's old-pair hashes. Corrupt, stale, other-path,
or otherwise ambiguous evidence is preserved and returns `cleanup_required`.
No raw path, credential, or digest is added to status, errors, audit, or
`Debug`.

No push or merge was performed.
