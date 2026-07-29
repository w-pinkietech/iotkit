# Task 7 report — local-root Node backup CLI

Issue: #113

## Outcome

Implemented the local-root `iotkit-edge-nodectl backup` surface:

- `backup configure` writes owner-only schema-1 configuration and an exact
  `RequiresMountsFor=` systemd drop-in, with explicit replacement required for
  existing configuration/drop-in files;
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
- non-backup commands use `all_edge_node_migrations()`, while the frozen legacy
  JSON `snapshot` command retains its R22 migration set and behavior.

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

The first complete-migration implementation opened legacy snapshot fixtures
with migration 23 and failed closed with `schema version 23 is ahead of latest
known 22`. Snapshot fixtures were moved to the explicit legacy migration helper
and the non-snapshot fixture expected versions were extended through 23. The
legacy JSON snapshot behavior now remains green.

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
- all existing `nodectl` unit and CLI tests, including legacy snapshot tests.

The battle-tested selector routed BT-001/002/003/004. Review covered custody
convergence, abrupt-loss boundaries, storage-pressure acknowledgement, and
computer replacement identity/loss boundaries. No physical power-cut, SD-card,
filesystem-controller, or deployment release evidence is claimed here.

## Verification

WSL Ubuntu-26.04:

```text
cargo test -p iotkit-edge-nodectl                         # 11 + 6 + 68 passed
cargo test -p iotkit-edge-nodectl --test backup_cli      # 6 passed
cargo test -p iotkit-edge-nodectl --test cli legacy_snapshot  # 1 passed
cargo test -p iotkit-core-recovery --test backup_contract    # 4 passed
cargo test -p iotkit-core-recovery backup                 # 23 + 2 passed
cargo test -p iotkit-core-recovery owner_passphrase_parser_accepts_one_terminal_line_ending_only # 1 passed
cargo clippy -p iotkit-edge-nodectl --all-targets -- -D warnings # exit 0
cargo fmt --all -- --check                                  # exit 0
```

Repository gates:

```text
node scripts/check-okf-docs.mjs                         # passed
scripts/check-layers                                      # passed
scripts/check-source-layout                               # passed
node scripts/battle-tested-review.mjs check               # passed
node --test scripts/tests/battle-tested-review.test.mjs   # 12 passed
git diff --check                                          # exit 0
```

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
- The legacy JSON snapshot command intentionally remains on its frozen R22
  migration surface; the complete migration set is used by all other commands.
