# Task 2 report: Render the commissioning path

## Outcome

Rendered `ConsoleView.commissioning` on `/status` and `/equipment` as a compact,
responsive four-step panel with stable `data-commissioning-stage`. The panel:

- keeps `収集ノードを登録`, `機器を確認`, `センサーを設定`, and
  `計測を開始` in a fixed order;
- puts one contextual primary navigation action before progress details for
  admins;
- gives viewers a read-only explanation without a mutation affordance;
- identifies current, completed, and pending steps with symbols and text in
  addition to color;
- retains visible keyboard focus and the existing 44px Console control height;
- places the panel before health metrics on `/status` and before the node list
  on `/equipment`.

Discovered Edge Node detail now shows the exact Edge Node ID, ledger epoch,
truthfully labeled first-detected time, current-descriptor device and sensor
counts, and the post-activation formal-history boundary. Historical inventory
rows remain durable in storage, but the current equipment/sensor presentation
and commissioning work list include only resources whose existing presence is
`current`. The existing Edge Node storage read
carries its original `created_at` through the read model; this adds no query,
schema, mutation, or custody behavior.

Active unconfigured devices and sensors are labeled `設定が必要` while their
raw reception state and received values remain visible. Recovery never renders
an activation form or button and now presents runbook-aligned preservation and
investigation guidance on the affected detail. An empty installation projects
`waiting-edge-node` instead of incorrectly claiming commissioning is complete;
a completed installation omits onboarding and preserves the normal monitor.

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

Review follow-up RED evidence:

- current-descriptor count assertions failed to compile because the read model
  had no snapshot-specific counts or first-detected field;
- the complete-stage rendering contract failed to compile because the test
  application had no complete commissioning scenario;
- the recovery-detail contract failed at runtime with
  `missing recovery help: 両方のデータベースを保全`;
- inspection and the new DOM-order assertion identified that
  `.onboarding-next` followed `.onboarding-steps`; the complete-scenario compile
  failure blocked that focused test binary until the scenario was added.

The final stale-resource review added two more RED cases:

- the pure projection selected `setup-device` instead of `complete` for an
  unconfigured device and signal whose `descriptor_current` was false;
- after applying a newer complete descriptor that removed the still-
  unconfigured resource, the storage-backed Console continued returning it in
  `reduced.devices`.

### GREEN

Each focused test was rerun after its minimal implementation and passed.
The final focused Rust rendering command:

```bash
TMPDIR=$PWD/target/test-tmp/issue-98-task2 \
  cargo test -p iotkit-edge --test console_contract
```

passed 19 tests with 0 failures. Coverage includes panel placement and order,
exactly one admin primary action, viewer read-only rendering, recovery without
activation, discovered facts and history boundary, unconfigured raw-value
visibility, stale-resource exclusion, empty-installation waiting, and
completed-installation suppression.

The storage-backed commissioning transition test also passed:

```bash
TMPDIR=$PWD/target/test-tmp/issue-98-task2 \
  cargo test -p iotkit-edge --test web_application_contract \
  console_commissioning_distinguishes_discovery_registration_and_setup -- --exact
```

Result: 1 passed, 0 failed.

The review follow-up extended this storage-backed test with a newer complete
descriptor containing zero devices and signals. RED failed because no
current-snapshot count fields existed. The final GREEN regression starts while
the resource is still unconfigured, applies the newer empty descriptor, and
confirms that durable storage still contains its inventory row while the
current Console contains no device/signal, commissioning has no pending work or
stale action, descriptor counts are exactly 0/0, and first-detected time is
unchanged. A subsequent newer descriptor restores the current resource so the
existing profile journey continues to be tested.

## Files

- `edge/src/web/templates/console.html`
- `edge/frontend/static/edge.css`
- `edge/src/web/mod.rs`
- `edge/src/web/console/commissioning.rs`
- `edge/src/composition/web.rs`
- `edge/src/storage/activation.rs`
- `edge/tests/console_contract.rs`
- `edge/tests/web_application_contract.rs`

The two storage/composition files only expose the already persisted ledger
epoch, first-detected timestamp, and descriptor presence to the existing
Console read model.

## Decisions

- Render only on the exact `/status` and `/equipment` collection pages, not as
  a repeated wizard on resource details.
- Do not render onboarding after the projection reaches `complete`; the normal
  status monitor is the correct steady-state experience.
- Use ordinary detail-page links as next actions; the panel performs no
  mutation and owns no client-side state.
- Keep recovery's admin action as “収集ノードを確認”; the resource model's
  `can_activate` remains false, so neither the panel nor detail renders an
  activation affordance.
- Preserve `受信中` and the raw value independently from the new
  `設定が必要` configuration badge.
- Label the persisted creation timestamp as `初回検出時刻` and display its
  production representation as Unix milliseconds. It does not imply the
  descriptor is fresh.
- Preserve stale inventory rows durably for history/profile continuity, but
  exclude them from current Console equipment/sensor lists and every
  commissioning stage, pending count, and action. Count only
  `presence == current` for descriptor snapshot facts.
- Put recovery instructions beside the affected Edge Node and mirror the
  runbook: preserve both databases, investigate identity/restore history, and
  do not delete rows, issue a new identity, or manually edit state.

## Verification

Required frontend command:

```bash
TMPDIR=$PWD/target/test-tmp/issue-98-task2 \
  scripts/test-edge-console-frontend.sh
```

Result:

- generated asset check and TypeScript compile: passed;
- frontend unit tests: 6 files, 21 tests passed;
- Rust `console_contract`: 19 passed, 0 failed.

Additional scoped checks:

```bash
cargo fmt --all -- --check
git diff --check
```

Both passed with exit code 0.

## Concerns

The Console currently has no shared server-side localized timestamp formatter,
so the truthfully labeled first-detected time is presented precisely as Unix
milliseconds. Adding locale-aware date/time formatting would be a separate
cross-Console concern and was not broadened into Task 2.
