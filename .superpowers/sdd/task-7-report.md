# Task 7 report: Convergent paired backup configuration recovery

## Outcome

Completed the recovery hardening for the Edge Node backup configuration pair.

- Added durable `config_publishing` and `drop_in_publishing` phases with exact
  transaction-bound temporary names and hashes.
- Prepared and parent-synced the exact systemd drop-in before the config rename,
  so a crash after either target rename can retry without synthesizing an
  unproven artifact.
- Recovered only exact retained temporary files or already-exact targets;
  impossible target/backup/temp combinations fail closed as
  `cleanup_required` without mutation.
- Added an owner-only completion receipt committed by atomic marker rename and
  directory sync. Status ignores a valid receipt, while configure consumes it
  idempotently for the same request and rejects corrupt, mismatched, or
  marker-plus-receipt states.
- Bound recovery cleanup to exact old-backup provenance and the persisted
  transaction identity. No debug or error output exposes paths, credentials, or
  hashes.

## TDD evidence

RED cases were added before the corresponding implementation:

- crash after config rename and crash after drop-in rename both initially
  returned success instead of leaving a recoverable marker;
- completion receipt final-sync uncertainty initially did not leave a receipt;
- forged `ConfigPublished` and phase-matrix states initially lacked the
  fail-closed recovery coverage.

GREEN focused command:

```bash
cargo test -p iotkit-edge-nodectl --test backup_cli -- --nocapture
```

Result: 20 passed, 0 failed. This includes the two rename-crash retry seams,
completion receipt retry, forged `ConfigPublished`, and a table-driven matrix
covering `Prepared`, `ConfigPublishing`, `ConfigPublished`,
`DropInPublishing`, `DropInPublished`, and `Published` for originally absent
and present targets. Every malformed row asserted `cleanup_required` and
byte-for-byte artifact preservation.

## Verification

Focused and broad commands (Linux toolchain in WSL) passed:

```bash
cargo check -p iotkit-edge-nodectl -p iotkit-core-recovery
cargo clippy -p iotkit-edge-nodectl -p iotkit-core-recovery --all-targets -- -D warnings
cargo test -p iotkit-edge-nodectl -p iotkit-core-recovery
scripts/check-layers
scripts/check-source-layout
scripts/verify.sh
```

Package tests passed with recovery 119 unit tests plus 4 contract tests and
nodectl 15 unit, 20 backup CLI, and 69 CLI tests. The workspace gate also
passed `cargo test --workspace` and workspace Clippy with `-D warnings`.

The worktree's `.git` file contains a Windows absolute gitdir, so WSL scripts
were run with explicit `GIT_DIR` and `GIT_WORK_TREE`; the direct unqualified
WSL invocation fails before running a check because Git cannot resolve that
Windows path. This is an environment limitation, not a product-test failure.

## Self-review

- Every recovery phase validates both target parents before any rename or
  cleanup; `ConfigPublishing` validates both exact temp transitions before it
  can rename the config.
- `DropInPublishing` additionally requires exact config old-backup provenance,
  preventing a forged marker from publishing a half-pair.
- Completion receipt validation is owner-only, bounded, schema/phase checked,
  and cannot coexist with a pending marker.
- Existing pair, rollback, symlink/hardlink/FIFO, operation-lock, and retry
  regressions remain green.

The battle-tested selector chose BT-004 (Edge Node computer replacement loss
boundary). This round only hardens the config/drop-in publication transaction;
encrypted computer replacement backup/restore remains the catalog's explicit
coverage gap and is not claimed as solved here.

## Integration

No push or merge was performed. The implementation is ready for the parent
agent's requested commit and review handoff.
