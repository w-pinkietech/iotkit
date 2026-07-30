# Task 9 — Optional Edge Node scheduling and recovery authority

Issue: #113

## Scope

This task adds the optional systemd service/timer and their template contract,
a real temporary-drop-in test, the bilingual Edge Node recovery contract, and
the operator/ingest wording for the shipped slice-1 boundary. Round 1 also
enforces the existing-tmpfs-parent/exact-leaf staging contract in recovery and
nodectl tests, and corrects the architecture/generation/reconciliation/drill
wording. Round 2 closes the recovery crate's cross-platform pure-helper compile
gap and removes the real-artifact restore-drill ambiguity. It does not add a
production handoff producer, Broker fence, remote permit, activation, or a
usable replacement journey.

## RED (before unit templates)

The test was added before either unit existed:

```text
node --test scripts/tests/edge-node-backup-systemd.test.mjs
✖ backup service has the owner-only runtime staging contract
  Error: ENOENT ... deploy/systemd/iotkit-edge-node-backup.service
✖ backup timer is opt-in and uses the daily jitter contract
  Error: ENOENT ... deploy/systemd/iotkit-edge-node-backup.timer
﹣ nodectl configure pins the exact captured mount point in a temporary drop-in
  # Linux-only product coverage; Rust backup_cli is not a Windows product substitute
2 failed, 1 skipped
```

The third case is not a self-authored fixture: on a Linux host with a runnable
`iotkit-edge-nodectl`, it invokes `backup configure` against owner-only temporary
directories, reads the persisted captured mount point, and asserts the generated
drop-in is exactly `RequiresMountsFor=<captured mount point>`. The Windows RED
run skipped only that Linux executable path.

## Round 1 RED (before staging implementation)

The focused Linux staging tests were written before the staging implementation:

```text
cargo test -j1 -p iotkit-core-recovery staging_verification -- --nocapture
running 3 tests
2 negative cases ... ok
staging_verification_creates_only_the_exact_absent_leaf_from_a_tmpfs_parent ... FAILED
  panicked ... DestinationInvalid
test result: FAILED. 2 passed; 1 failed
```

The failure was the intended missing behavior: an absent exact leaf under an
existing tmpfs parent was rejected instead of being created descriptor-
relatively. The additional configure and nodectl cases cover non-tmpfs,
symlink, broad-mode, and pair-publication rejection boundaries.

## GREEN

Windows host:

```text
node --test scripts/tests/edge-node-backup-systemd.test.mjs
2 passed, 0 failed, 1 skipped (Linux product invocation)
```

WSL Ubuntu host:

```text
node --test scripts/tests/edge-node-backup-systemd.test.mjs
3 passed, 0 failed, 0 skipped
```

The WSL run exercised the real configure path and exact generated drop-in. The
service template contains `Type=oneshot`, `UMask=0077`,
`RuntimeDirectory=iotkit-edge-node-backup`, `RuntimeDirectoryMode=0700`,
`Environment=TMPDIR=/run/iotkit-edge-node-backup`, and the exact Task 9
`ExecStart`. The timer is opt-in with `OnCalendar=daily`,
`RandomizedDelaySec=2h`, `Persistent=true`, and `WantedBy=timers.target`.

## Documentation authority

`docs/okf/en/contracts/edge-node-recovery-v1.md` and its Japanese pair define:

- `IOTKNDB1`, exact big-endian framing, closed/bounded header and manifest,
  Argon2id/XChaCha20-Poly1305 parameters, nonce/AAD construction, record and
  terminal invariants;
- sanitized manifest fields/counts and the `target_registry` deployment-token
  sanitizer (credential hashes may remain protected encrypted state);
- closed handoff/receipt schemas, candidate-row provenance binding, absent-path
  restore, fenced startup, exact post-rename replay, and conflict/uncertainty
  behavior;
- machine schema, golden fixture, binary vector, and conformance-test paths;
- default-off scheduling, owner-only files, capability-tested mount identity,
  `/run` tmpfs parent versus exact `RuntimeDirectory=` leaf, manual commands,
  escrow, and restore-drill limits.

The bilingual runbook distinguishes Edge Node sections 7.1/8.1 from existing
IoTKit Edge sections 7.2/8.2. The ingest contract now says that encrypted
custody-complete sanitized backup and local fenced-candidate restore are
implemented, while Broker fencing, remote permit, reconciliation, dedup-risk
resolution, reactivation, and same-ID new epoch remain deferred/default-off.
It also states that backup candidates carry claims only through the authenticated
snapshot boundary and that no-backup replacement restores neither.

Round 1 staging behavior is now:

- `configure` opens every existing parent component without following symlinks,
  requires an euid-owned, non-group/other-writable tmpfs parent (0755 `/run`
  accepted; world-writable `/dev/shm` root rejected), records the exact leaf,
  and creates no parent tree;
- `create` uses that parent descriptor to `mkdirat` an absent exact leaf as
  owner-only `0700`, accepts an existing owner-only tmpfs directory only after
  type/link-count checks, and removes only a leaf it created when preflight
  fails;
- the persistent passphrase setup is guarded with `test -e` before creating an
  empty file, so rerunning the runbook cannot truncate an existing secret.

The generation field is recorded and bound to candidate provenance/receipt but
is not compared with live authority in slice 1. Architecture and ingest wording
now keeps same-ID new-epoch/production reconciliation deferred, while exact
local same-request replay remains shipped. Checked-in handoff fixtures are
conformance-only with their matching test-generated artifact; no real-backup
operator restore success is claimed without later authority.

## Round 2 RED (before cross-platform helper fix)

The Windows library check reproduced the six-closure regression before the
minimal helper change:

```text
cargo check -p iotkit-core-recovery --lib
error[E0425]: cannot find value `valid_pair_hash` in this scope
error[E0425]: cannot find function `valid_pair_txid` in this scope
error[E0425]: cannot find function `pair_path_hash` in this scope
error[E0425]: cannot find function `valid_pair_config_temp_name` in this scope
... 10 previous errors; 1 warning
```

The helpers were Linux-gated even though `BackupPairRecord`'s public validators
compile on every target. The destination `Component` import was also unused on
Windows.

## Round 2 GREEN

The pure validators and SHA-256 path helper are now cross-platform; the path
helper uses `OsStr::as_encoded_bytes()` (identical raw bytes on Linux), and the
Linux-only `Component` import is scoped accordingly. A platform-neutral helper
test exercises hashes, txids, temporary names, and path digest binding.

The systemd test skip now states that the product invocation is Linux-only and
must run with a Linux binary (WSL CI); the Rust CLI suite is not presented as a
Windows product substitute. The recovery contract now says that a real artifact
may be inspected and verified off-host in slice 1, while restore conformance is
limited to the matching test-generated artifact; real-backup RPO restore-drill
verification waits for a later recovery authority.

## Verification

- `node --test scripts/tests/edge-node-backup-systemd.test.mjs` (Windows:
  2 passed, 1 host skip with the Linux-only-product wording; WSL: 3 passed).
- Windows `cargo check -p iotkit-core-recovery --lib` passed after Round 2;
  the pre-fix RED had the ten E0425 helper errors above. The recovery crate is
  warning-free; only the unrelated `iotkit-core-ops` `mode` warning remains.
- Windows `cargo check -p iotkit-edge-nodectl` reaches the known pre-existing
  Unix-only `cmd/passphrase.rs` `std::os::fd`/`libc` blocker (40 errors); no
  Round 2 recovery E0425 or import warning remains.
- WSL focused `backup_pair_helpers_are_platform_neutral`: 1 passed; full
  `cargo test -j1 -p iotkit-core-recovery`: 130 unit tests passed, 4
  backup-contract tests passed, 1 ignored fixture generator.
- WSL strict clippy for `iotkit-core-recovery` and `iotkit-edge-nodectl`
  (`--all-targets -- -D warnings`) passed; Windows `cargo fmt --all -- --check`
  and `git diff --check` passed.
- WSL focused staging tests: 6 passed; configure non-tmpfs unit: 1 passed;
  nodectl non-tmpfs configure: 1 passed; nodectl manual configure→absent leaf
  create→artifact happy path: 1 passed.
- WSL `cargo test -j1 -p iotkit-core-recovery`: 130 unit tests passed, 4
  backup-contract tests passed, 1 ignored fixture generator.
- WSL `cargo test -j1 -p iotkit-edge-nodectl --test backup_cli --test cli`:
  24 backup CLI tests and 69 CLI tests passed.
- `node scripts/check-okf-docs.mjs` — passed (10 bilingual concepts).
- `scripts/check-layers` — passed on Windows.
- `scripts/check-source-layout` — passed on Windows.
- `node scripts/battle-tested-review.mjs check` — passed (5 entries).
- `node scripts/battle-tested-review.mjs select --base origin/master` selected
  BT-001, BT-002, BT-003, and BT-004 because the broad branch paths route to
  all four entries; unmatched paths remain explicitly listed for review.
- `git diff --check` — passed.
- `systemd-analyze verify` was attempted in WSL; it cannot validate this
  repository copy because Windows checkout permissions mark units executable/
  world-writable and `/usr/local/bin/iotkit-edge-nodectl` is not installed.
  This is deployment evidence still required on a target host, not a template
  test failure.

## Residual / deferred

Physical power-cut, SD-card/filesystem-controller durability, target-host
systemd/mount capability, and a root-run `/run` RuntimeDirectory exercise
remain release/field gates (the WSL `/run` staging test is guarded for its
non-root test account). Slice 1 intentionally has no
production handoff creator, Broker fencing, permit, reconciliation, dedup
activation, reactivation, same-ID new epoch, or usable replacement procedure.
The candidate stays fenced and cannot collect or publish. No push or merge was
performed.
