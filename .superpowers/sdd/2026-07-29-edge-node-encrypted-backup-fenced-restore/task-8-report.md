# Task 8 — Process-wide startup fence

## Scope

The Edge Node composition root now probes the configured database with the
read-only `probe_startup_path()` fence immediately after configuration parsing.
Fenced candidates and invalid recovery state exit with status 3 and fixed,
non-identifying text before catalog/effective-config logging, migration,
identity/provenance writes, runtime creation, or service setup. Normal startup
uses the shared `all_edge_node_migrations()` set and rechecks `startup_mode()`
after migration before any identity/provenance mutation.

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

## GREEN

Focused WSL run after implementation:

```text
cargo test -p iotkit-edge-node --test recovery_startup --test cutover
```

Result: 6 tests passed (4 binary fence cases and 2 cutover/normal migration
cases). The binary cases cover a valid fenced candidate, malformed recovery
schema, malformed duplicate recovery row, and rotated recovery authority. Each
asserts exit 3, fixed generic stderr, no sentinel/service/listener activity,
and unchanged database bytes. The normal cases cover pre-v23 migration and a
current v23 database; both retain normal startup/migration behavior and fail
only at the intentionally missing MQTT password after migration.

## Verification

- `cargo fmt --all` (WSL): passed.
- `cargo test -p iotkit-edge-node --test recovery_startup --test cutover` (WSL): passed.
- `cargo test -p iotkit-edge-node` (WSL): passed (all unit/integration/doc tests).
- `cargo clippy -p iotkit-edge-node --all-targets -- -D warnings` (WSL): passed.
- `scripts/check-layers` and `scripts/check-source-layout` (WSL): passed.
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
