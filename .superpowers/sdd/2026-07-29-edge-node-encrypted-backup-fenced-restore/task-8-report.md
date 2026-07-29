# Task 8 — Process-wide startup fence

## Scope

The Edge Node composition root now parses only raw TOML and environment
overrides, obtains the minimal `db_path`, and probes it with the read-only
`probe_startup_path()` fence before adapter/catalog resolution. Fenced
candidates and invalid recovery state exit with status 3 and fixed,
non-identifying text before catalog/effective-config logging, migration,
identity/provenance writes, runtime construction, or service setup. Normal
startup uses the shared `all_edge_node_migrations()` set and rechecks
`startup_mode()` after migration before any identity/provenance mutation.

`config::UnresolvedConfig` carries the one parsed raw config and source across
the fence, so the process does not reread the config file or change source
precedence between the pre-fence and post-fence phases.

The fence messages use `eprintln!` only on the two early-exit paths. This keeps
the existing normal startup log stream unchanged while making the fixed fence
messages observable on stderr.

## RED (before implementation)

On WSL Ubuntu, with only the new binary test and recovery dependency present:

```text
cargo test -p iotkit-edge-node --test recovery_startup
```

The test failed as intended. The pre-fence binary logged the sentinel effective
configuration and then attempted the old migration set, exiting with
`schema version 23 is ahead of latest known 22`; it did not emit the fence
message and therefore did not satisfy the no-leak/no-start assertions.

The same command on the Windows host cannot compile the existing Raspberry Pi
`rppal` dependency (`std::os::unix`/Unix libc symbols are unavailable), so
behavior evidence was collected in WSL.

## Round 1 RED (review findings)

The reviewer’s invalid-adapter regression was run before moving resolution
behind the fence. It failed with exit status 1 and exposed the configured
unknown adapter in normal config diagnostics instead of returning the fence:

```text
assertion `left == right` failed
stdout=... invalid config: adapter sentinel has unknown type "unknown-adapter"
left: Some(1)
right: Some(3)
```

The startup fixtures were then tightened to use the canonical source → backup
→ encrypted artifact → `restore_candidate` path. The old direct migration plus
raw candidate-row fixture had no evidence for publication, receipt, authority,
or sidecar cleanup; the replacement performs those operations and asserts a
`DurablyFencedCandidate` receipt before launching the binary. The tests also
snapshot the database, WAL/SHM/journal sidecars, and initialization marker.

Cutover coverage was extended with missing and empty database launches. Each
must reach the post-migration normal-state oracle and then terminate at the
intentional missing-MQTT-password boundary. All subprocess launches now have a
10-second polling timeout that kills and waits for a stuck child.

## GREEN

Focused WSL run after implementation:

```text
cargo test -p iotkit-edge-node --test recovery_startup --test cutover
```

Result: 8 tests passed (5 binary fence cases and 3 cutover/normal migration
cases). The binary cases cover a canonical restored candidate, malformed
recovery schema, malformed duplicate recovery row, rotated recovery authority,
and an invalid adapter catalog. Each asserts exit 3, fixed generic stderr, no
sentinel/service/listener activity, and unchanged database plus sidecars. The
normal cases cover pre-v23, current v23, missing, and empty databases; each
reaches migration with `startup_mode = normal` and fails only at the
intentional missing-MQTT-password boundary.

## Verification

- `cargo fmt --all` (WSL): passed.
- `cargo test -p iotkit-edge-node --test recovery_startup --test cutover` (WSL): passed (8 tests).
- `cargo test -p iotkit-edge-node` (WSL): passed (all unit/integration/doc tests).
- Focused `iotkit-core-recovery` state tests for normal missing/pre-recovery and
  malformed recovery schema/row: passed.
- `cargo clippy -p iotkit-edge-node --all-targets -- -D warnings` (WSL): passed.
- `cargo fmt --all -- --check` (WSL): passed.
- `scripts/check-layers` and `scripts/check-source-layout` (WSL): passed.
- `node scripts/battle-tested-review.mjs check` (WSL): passed; selector selected
  BT-001/002/003/004 from the broader branch diff, with no additional
  startup-fence-specific routing match.
- `git diff --check`: passed.
- `node scripts/battle-tested-review.mjs select --base origin/master` selected
  BT-001/002/003/004 from the broader Task 1–7 branch diff; none add a new
  startup-fence-specific concern beyond the custody/replacement risks already
  covered by the focused tests and the deferred Slice 2 transition.
- Existing core recovery unit tests already cover read-only/no-create probing,
  exact schema/trigger validation, candidate-field validation, and normal
  missing/pre-v23 paths (`edge-node/core/recovery/tests/unit/state_tests.rs`).
- Existing gateway cutover test remains passing and verifies the preflight
  refusal is still before migration (`edge-node/apps/node/tests/cutover.rs`).

## Residual / deferred

Slice 1 intentionally exits for `durably_fenced_candidate`; the restricted
recovery runtime and typed transition to `fenced_waiting_permit` remain Slice 2
work. Hardware and MQTT broker evidence are outside this process-fence change;
the tests use a missing password file so normal startup terminates without an
external service.
