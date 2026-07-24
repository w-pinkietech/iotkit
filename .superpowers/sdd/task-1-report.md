# Task 1 report: Console commissioning projection

## Outcome

Implemented the pure Console commissioning projection and attached it to every
storage-backed `ConsoleView`. The projection:

- preserves the required priority:
  `recovery_hold` > `activating` > `discovered` > unconfigured device >
  unconfigured signal > missing active semantic rule > complete;
- exposes the stable stages `recovery`, `activation-in-progress`,
  `activate-edge-node`, `setup-device`, `setup-sensor`, `setup-rule`, and
  `complete`;
- chooses the first affected Edge Node, device, or signal detail URL;
- derives pending counts and four-step progress from the already assembled
  Console models;
- uses the authoritative `EdgeNodeState` on `ConsoleEdgeNode`, rather than
  inferring activation state from presentation labels or querying storage from
  the web template;
- treats the absence of an active semantic rule as setup work and does not
  inspect Output Adapter state.

No HTML template, CSS, or browser behavior was changed. The Task 1 brief's
rendering assertion conflicted with Task 2's explicit rendering ownership; the
coordinator confirmed that Task 1 should replace it with pure projection
contracts and storage-backed `ConsoleView` assertions.

## TDD evidence

### RED

Command:

```bash
TMPDIR=$PWD/target/test-tmp/issue-98 cargo test -p iotkit-edge \
  --test console_contract --test web_application_contract commissioning
```

Result: failed as expected with exit code 101. The compiler reported the
intentionally missing `web::console::commissioning` module,
`ConsoleEdgeNode.state`, and `ConsoleView.commissioning` field.

### GREEN

Final focused command:

```bash
TMPDIR=$PWD/target/test-tmp/issue-98 cargo test -p iotkit-edge \
  --test console_contract --test web_application_contract commissioning
```

Result: passed with exit code 0:

- `console_contract`: 3 passed, 0 failed;
- `web_application_contract`: 1 passed, 0 failed.

The pure contracts cover recovery/activation priority, discovered activation,
active plus unconfigured device, configured device plus unconfigured signal,
missing rule, completion, first-resource URLs, progress, and pending counts.
The storage-backed contract observes `activate-edge-node`, `setup-device`, and
`setup-sensor` through the real `StorageWebApplication`.

## Additional verification

Commands:

```bash
cargo clippy -p iotkit-edge --tests -- -D warnings
cargo fmt --all --check
git diff --check
node scripts/battle-tested-review.mjs select --base HEAD~1
```

Results:

- focused Edge Clippy: passed with exit code 0;
- formatting check: passed with exit code 0;
- diff whitespace check: passed with exit code 0;
- battle-tested selector chose BT-005. The selected operational-layer
  diagnostic concern is unchanged by this pure setup projection; Task 1 adds
  no health claims or rendering.

An initial `scripts/verify.sh` attempt failed while compiling SQLite because
the host `/tmp` quota was exhausted. Rerunning with the worktree TMPDIR allowed
the workspace test phase to pass through the affected Edge tests. The
coordinator then directed that the broad gate be stopped because Task 1
requires focused verification and the complete broad gate belongs to Task 5.

## Self-review

- The projection is pure and has no storage, template, output-adapter, or
  mutation dependency.
- Activation priority uses the authoritative enum carried from storage.
- Device and signal readiness use existing profile revision/rule projections.
- Only active semantic rules are visible in `ConsoleSignal.rules`, so inactive
  rules correctly remain setup work.
- Existing activation, raw reception, profile mutation, and Console content
  assertions remain in the storage-backed contract.
- No product contract, template, stylesheet, or historical plan was modified.

## Concerns

None for Task 1. Rendering and role-specific action presentation remain
intentionally deferred to Task 2.
