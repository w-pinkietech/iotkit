# AGENTS.md

This is the common repository guidance for coding agents and human maintainers.
Agent-specific files may point here but must not redefine these rules.

## Project overview

`iotkit-next` is an on-premises-first IoT data collection foundation rebuilt from
the former IoTKit. It consists of:

- **IoTKit Edge Node:** Rust + Tokio collection software for Raspberry Pi-class
  computers. Sensor-specific behavior stays in Input Adapters.
- **MQTT Broker:** standard infrastructure between Edge Nodes, IoTKit Edge, and
  external applications. IoTKit does not implement its own Broker.
- **IoTKit Edge:** Go service that accepts durable raw records, applies generic
  meanings, serves the Console, and invokes Output Adapters.

```text
sensor -> Input Adapter -> IoTKit Edge Node -> MQTT Broker
       -> IoTKit Edge -> Output Adapter -> external application

contract-native device -> authenticated HTTP ingest -> IoTKit Edge Node
```

Input Adapters use `iotkit-ingest-client`; they do not depend on `edge-node/core/engine`.
`AdapterEvent` is a frozen engine/supervision vocabulary, not a new adapter API.
The HTTP ingest listener is a separate, default-off path for contract-native
devices. Both paths converge at the Edge Node collector. The complete and
enforced dependency map is in the architecture document and `scripts/check-layers`.

## Documentation authority

Start at `docs/README.md`. The current human-readable product corpus is
`docs/okf/`. Choose either `ja` or `en` to read; edit both language files together.

A versioned contract is one artifact made from its paired contract documents,
machine-readable schema or exported wire types, shared fixtures, and conformance
tests. None silently overrides the others.

All of `docs/redesign/` and `docs/superpowers/` is historical or rationale-only.
It never overrides the current corpus. Old IoTKit code is not an authority. If
the task conflicts with a current contract, stop and report the conflict.

## Before changing code

Read `docs/README.md`, then only the rows relevant to the task. Do not load every
historical plan.

| Change area | Read before editing | Start in | Focused verification |
|---|---|---|---|
| Product boundary or component ownership | `docs/okf/<lang>/concepts/product-model.md`, `docs/okf/<lang>/architecture/system-overview.md` | owning component from the architecture map | affected contract and package tests |
| Crate, package, dependency, or source placement | `docs/okf/<lang>/architecture/system-overview.md` | `Cargo.toml`, `edge/go.mod`, `scripts/check-layers` | `scripts/check-layers`, `scripts/check-source-layout` |
| Sensor, driver, polling, UART, or Input Adapter host | `docs/okf/<lang>/contracts/input-adapter-v1.md` | `edge-node/adapters/bravepi-mainboard/src/`, `edge-node/adapters/rpi-local/src/`, `edge-node/input/hardware/sensor-drivers/src/`, `edge-node/input/runtimes/polling/src/` | `cargo test -p <owning-crate>` plus adapter conformance tests when the contract changes |
| Envelope/Ack, authenticated device ingest, admission, or principal mapping | `docs/okf/<lang>/contracts/ingest-v1.md` and product model | `edge-node/ingest/contract/`, `edge-node/ingest/client/`, `edge-node/ingest/http/`, `edge-node/core/collector/`; BravePI envelope mapping starts at `edge-node/adapters/bravepi-mainboard/src/task/ingest_map.rs` | owning crate tests; use ingest conformance fixtures when wire behavior changes |
| Edge Node activation, MQTT delivery, ack, retention, or data loss | `docs/okf/<lang>/contracts/edge-node-custody-v1.md`, `docs/okf/<lang>/contracts/ingest-v1.md` | `edge-node/core/ledger/`, `edge-node/core/publish/`, `edge-node/core/storage/`, `edge-node/core/timeseries/`, `edge-node/apps/node/` | owning crate tests, then `scripts/verify.sh` |
| IoTKit Edge raw storage, meanings, history, or CSV | product model and architecture; for browser JSON also `edge/openapi/edge-console-v1.yaml` | `edge/internal/store/`, `edge/internal/semantic/`, `edge/internal/semantics/` | `(cd edge && go test ./internal/<owning-package>)` |
| Output Adapter or external application contract | `docs/okf/<lang>/contracts/output-adapter-v1.md` | `edge/internal/outputadapter/`, `edge/internal/applicationcontract/` | `(cd edge && go test ./internal/outputadapter ./internal/applicationcontract)` |
| Console HTML, navigation, or browser behavior | architecture; OpenAPI only for endpoints and schemas represented there | `edge/internal/edgehttp/`, `edge/frontend/` | `scripts/test-edge-console-frontend.sh`, then `scripts/test-edge-console-e2e.sh` for journeys |
| Account bootstrap or recovery | `docs/okf/<lang>/operations/installation-and-recovery.md` sections 1 and 4 | `edge/cmd/iotkit-edge/main.go`, `edge/internal/store/accounts.go`; Console account management starts at `edge/internal/edgehttp/console_accounts.go` | `(cd edge && go test ./cmd/iotkit-edge ./internal/edgehttp ./internal/store)` |
| Login, password, session, cookie, CSRF, or authorization | relevant current contract plus `edge/internal/edgehttp/server.go` route registration; session endpoints are not currently represented in the Console OpenAPI | `edge/internal/edgehttp/server_console_auth_test.go`, `edge/internal/edgesession/`, `edge/internal/edgeauth/` | `(cd edge && go test ./internal/edgehttp ./internal/edgesession ./internal/edgeauth)` |
| TLS, certificate, or deployment credentials | `docs/okf/<lang>/operations/installation-and-recovery.md` sections 1 and 3 | owning service and `deploy/` | focused security or deployment script for the changed path |
| Encrypted backup or restore | `docs/okf/<lang>/operations/installation-and-recovery.md` section 7 for backup or section 8 for restore | `edge/cmd/iotkit-edge/main.go`, `edge/internal/store/backup_encrypted.go`, `edge/internal/store/postgres_backup.go` | `(cd edge && go test ./cmd/iotkit-edge ./internal/store)`; `scripts/test-edge-postgres.sh` covers its named PostgreSQL cases, not the entire operator journey |
| Device retirement or hardware replacement | `docs/okf/<lang>/operations/installation-and-recovery.md` section 9 and Edge Node custody contract | owning Edge Node identity/custody path | focused replacement and custody tests |
| SQLite-to-PostgreSQL migration | `docs/okf/<lang>/operations/installation-and-recovery.md` section 10 and storage capacity document | `edge/cmd/iotkit-edge/main.go`, `edge/internal/store/postgres_migrations.go` | `scripts/test-edge-postgres.sh` plus affected command tests |
| Manual update or rollback | `docs/okf/<lang>/operations/installation-and-recovery.md` section 11 | `deploy/`, affected startup and migration code | changed update/rollback journey |
| Capacity, retention, or storage profile selection | `docs/okf/<lang>/operations/storage-capacity.md` | `edge/internal/store/`, `scripts/test-edge-capacity.sh`, `scripts/test-edge-postgres.sh` | relevant capacity or PostgreSQL case only |
| Vulnerability report or accidental secret exposure | `SECURITY.md` | reporting and containment path; do not copy secrets into repository artifacts | follow reporting policy; do not create a public reproducer with secrets |
| Contract or documentation change | `docs/README.md`, both language files, schemas/types, fixtures, conformance tests | current authority; never a historical plan | `node scripts/check-okf-docs.mjs` plus affected conformance tests |
| PR review or field report triage | `.agents/skills/iotkit-battle-tested-review/SKILL.md`, `review/battle-tested/README.md` | selector output and linked evidence | `node scripts/battle-tested-review.mjs check` |

## Common commands

Run the smallest command that can disprove the change, then widen for risk.

```bash
# Documentation, dependency, and source/test structure
node scripts/check-okf-docs.mjs
scripts/check-layers
scripts/check-source-layout

# Battle-tested review routing
node scripts/battle-tested-review.mjs select --base origin/master

# Rust focused / full
cargo test -p <crate-name>
scripts/verify.sh

# IoTKit Edge focused / full
(cd edge && go test ./internal/<package>)
(cd edge && go test ./...)

# Console schema, generated assets, and browser journey
scripts/test-edge-console-frontend.sh
scripts/test-edge-console-e2e.sh

# Release-candidate host integration only
scripts/test-edge-host-release-gate.sh NEW_REPORT_DIRECTORY
```

`scripts/verify.sh` runs Rust formatting, layer rules, workspace tests, Clippy
with `-D warnings`, and Go tests. The host release gate is not a per-PR default.
Raspberry Pi and physical sensors are required only when the task explicitly
requires hardware evidence.

## Product invariants

- Never expose tokens, credentials, keys, their hashes, customer identifiers, or
  sensitive configuration in debug output, logs, errors, audit records, fixtures,
  issues, or pull requests.
- Never silently lose data. Follow the current ingest and custody contracts.
  `rejected` is only for deterministic terminal violations. A storage failure
  does not produce `rejected` or a durable success acknowledgement.
- Mutations go through the owning typed operation dispatcher: `edge-node/core/ops` on Edge
  Node and the Go application service on IoTKit Edge. Do not add API/UI/CLI paths
  that write SQL directly.
- Do not treat MQTT PUBACK as IoTKit Edge durable raw acceptance, or downstream
  business success as IoTKit output custody.

## Issue, worktree, and pull request loop

Every development task maps to one GitHub issue. Record the intended outcome
and exclusions before implementation.

1. Update `master`.
2. Create `agent/issue-<number>-<slug>` and
   `.worktrees/issue-<number>-<slug>`.
3. Work and verify only in that worktree.
4. Commit intentionally, push the branch, and open a draft PR that closes the
   issue.
5. Stop for human review. Apply feedback on the same branch and PR.
6. Merge only after explicit approval. After confirmed merge, remove the local
   worktree and branch.

Keep the diff inside the issue scope. Create a separate issue when the scope
changes materially.

Branch push and draft PR creation are pre-authorized completion steps for this
loop. Merge, release, destructive actions, paid actions, and other external
effects still require explicit approval.

## Change lanes

Choose the lightest lane that covers realistic risk.

For every product behavior change, add or update the closest focused test before
implementation.

| Lane | Use for | Required process |
|---|---|---|
| Fast | local bug, refactor, docs, configuration, or small feature without contract/security/custody/migration/restore impact | focused test when behavior changes, focused verification |
| Standard | multiple packages, a new internal boundary, or several credible implementations | concise design, one review, proportional tests |
| Full | public wire contract, auth/secrets, custody/data loss, DB migration, backup/restore/rollback, destructive or expensive compatibility decisions | explicit design, implementation plan when useful, tests first, independent review, broad verification |

Process plugins and skills are optional aids unless the user names one or this
file requires one. They do not override the user request or repository rules.
Prefer current code, executable tests, and existing authority over new process
documents. Do not create a spec only to repeat an existing decision.
Use repository-local independent review by default. Call an external review
model or service only when the user explicitly requests it.

## Source and test placement

- Rust product `src/` contains product code, not test bodies or test helpers.
  Private-API tests live under `<crate>/tests/unit/**/*_tests.rs` and are included
  by a minimal `#[cfg(test)] #[path = "..."] mod tests;`. Public integration
  tests live under `<crate>/tests/*.rs`.
- Test constructors, clocks, fixtures, mocks, and observation helpers live in
  `tests/support/` or a dedicated testkit.
- Go tests remain in separate `*_test.go` files. Split large product files by
  responsibility within the same package.
- Frontend unit tests live under `edge/frontend/tests/unit/`, not `src/`.
- `scripts/check-source-layout` enforces these boundaries.

## Review and verification

Before final review, use `$iotkit-battle-tested-review` or run the selector
directly. Review only selected `BT-NNN` entries plus semantic concerns that path
routing cannot infer. Zero selections and unmatched paths are not proof of safety.

Verification must match the changed failure paths. Run `scripts/verify.sh` when
Rust product behavior changes or cannot be excluded. Documentation-only changes
may use documentation, link, structure, and diff checks. When skipping a check
normally expected for the change, state the check and the concrete reason.

Tests passing are necessary, not sufficient: also compare the result with current
contracts and the product invariants above.
