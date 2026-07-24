# Task 2 report: Render the commissioning path

## Outcome

Rendered `ConsoleView.commissioning` on `/status` and `/equipment` as a compact,
responsive four-step panel with stable `data-commissioning-stage`. The panel:

- keeps `収集ノードを登録`, `機器を確認`, `センサーを設定`, and
  `計測を開始` in a fixed order;
- puts one contextual primary navigation action first for admins;
- gives viewers a read-only explanation without a mutation affordance;
- identifies current, completed, and pending steps with symbols and text in
  addition to color;
- retains visible keyboard focus and the existing 44px Console control height;
- places the panel before health metrics on `/status` and before the node list
  on `/equipment`.

Discovered Edge Node detail now shows the exact Edge Node ID, ledger epoch,
descriptor receipt time, descriptor-derived device and sensor counts, and the
post-activation formal-history boundary. The existing Edge Node storage read
now carries `last_descriptor_at` through the read model; it adds no query,
schema, mutation, or custody behavior.

Active unconfigured devices and sensors are labeled `設定が必要` while their
raw reception state and received values remain visible. Recovery never renders
an activation form or button. An empty installation now projects
`waiting-edge-node` instead of incorrectly claiming commissioning is complete.

## TDD evidence

### RED

The rendering test was added first and run with:

```bash
TMPDIR=$PWD/target/test-tmp/issue-98-task2 \
  cargo test -p iotkit-edge --test console_contract commissioning
```

It failed with exit code 101 at:

```text
/status must expose the stable commissioning stage
```

The discovered-detail test then failed with exit code 101 at:

```text
missing discovered-node fact: Edge Node ID
```

The role/recovery and unconfigured-resource slices initially failed to compile
because their deliberately requested `StubApplication::viewer`,
`StubApplication::recovery`, and `StubApplication::unconfigured` scenarios did
not exist. The empty-installation projection test failed with:

```text
left: "complete"
right: "waiting-edge-node"
```

### GREEN

Each focused test was rerun after its minimal implementation and passed.
The final focused Rust rendering command:

```bash
TMPDIR=$PWD/target/test-tmp/issue-98-task2 \
  cargo test -p iotkit-edge --test console_contract
```

passed 17 tests with 0 failures. Coverage includes panel placement and order,
exactly one admin primary action, viewer read-only rendering, recovery without
activation, discovered facts and history boundary, unconfigured raw-value
visibility, and the empty-installation waiting state.

The storage-backed commissioning transition test also passed:

```bash
TMPDIR=$PWD/target/test-tmp/issue-98-task2 \
  cargo test -p iotkit-edge --test web_application_contract \
  console_commissioning_distinguishes_discovery_registration_and_setup -- --exact
```

Result: 1 passed, 0 failed.

## Files

- `edge/src/web/templates/console.html`
- `edge/frontend/static/edge.css`
- `edge/src/web/mod.rs`
- `edge/src/web/console/commissioning.rs`
- `edge/src/composition/web.rs`
- `edge/src/storage/activation.rs`
- `edge/tests/console_contract.rs`

The two storage/composition files only expose the already persisted ledger
epoch and descriptor receipt timestamp to the existing Console read model.

## Decisions

- Render only on the exact `/status` and `/equipment` collection pages, not as
  a repeated wizard on resource details.
- Use ordinary detail-page links as next actions; the panel performs no
  mutation and owns no client-side state.
- Keep recovery's admin action as “収集ノードを確認”; the resource model's
  `can_activate` remains false, so neither the panel nor detail renders an
  activation affordance.
- Preserve `受信中` and the raw value independently from the new
  `設定が必要` configuration badge.
- Display the persisted descriptor receipt timestamp exactly and label its
  production representation as Unix milliseconds rather than implying a
  localized wall-clock conversion.

## Verification

Required frontend command:

```bash
TMPDIR=$PWD/target/test-tmp/issue-98-task2 \
  scripts/test-edge-console-frontend.sh
```

Result:

- generated asset check and TypeScript compile: passed;
- frontend unit tests: 6 files, 21 tests passed;
- Rust `console_contract`: 17 passed, 0 failed.

Additional scoped checks:

```bash
cargo fmt --all -- --check
git diff --check
```

Both passed with exit code 0.

## Concerns

The Console currently has no shared server-side localized timestamp formatter,
so production descriptor receipt time is presented precisely as Unix
milliseconds. Adding locale-aware date/time formatting would be a separate
cross-Console concern and was not broadened into Task 2.
