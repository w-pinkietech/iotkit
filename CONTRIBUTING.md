# Contributing to IoTKit

[日本語](CONTRIBUTING.ja.md) | English

Thank you for helping make IoT data collection easier to operate and maintain.
IoTKit values field evidence, explicit data ownership, and changes that a new
maintainer can understand without reconstructing the project's history.

## Start here

Before changing code, read these in order:

1. [Product model](docs/okf/en/concepts/product-model.md) — what IoTKit owns and
   what remains in devices or external applications.
2. [Architecture](docs/okf/en/architecture/system-overview.md) — runtime
   components, crate map, code placement, and dependency rules.
3. The relevant [current contract](docs/okf/en/index.md#contracts) — ingest,
   Input Adapter, Edge Node custody, or Output Adapter.
4. [AGENTS.md](AGENTS.md) — repository invariants and the verification lanes
   used by both people and coding agents.

`docs/okf/` is the current human-readable product authority.
`docs/redesign/` and `docs/superpowers/` preserve history; they do not override
current contracts, executable fixtures, or tests.

## Development environment

The supported contributor environment is Linux. CI currently uses:

- Rust 1.95.0, selected automatically by `rust-toolchain.toml`;
- Go 1.25, as declared by `iotkit-edge/go.mod`;
- Node.js 22 and npm for Console assets and tests;
- `pkg-config` and `libudev-dev` for Raspberry Pi transport dependencies.

Docker Compose, OpenSSL, `jq`, and `curl` are also required for the integration
scripts. No Raspberry Pi or physical sensor is needed for the normal development
loop.

On Debian or Ubuntu, the non-language packages can be installed with:

```bash
sudo apt-get update
sudo apt-get install --yes pkg-config libudev-dev docker.io docker-compose-v2 \
  openssl jq curl
```

Install Rust through [rustup](https://rustup.rs/), Go 1.25 through the official
Go distribution or a version manager, and Node.js 22 through your normal package
manager. Do not commit credentials, generated certificates, local databases, or
deployment output directories.

## First hour

### 0–10 minutes: establish the map

```bash
git clone git@github.com:w-pinkietech/iotkit-next.git
cd iotkit-next
node scripts/check-okf-docs.mjs
scripts/check-layers
scripts/check-source-layout
```

Then read the product model and the architecture document linked above. The
short version of the runtime path is:

```text
sensor / device
  -> Rust IoTKit Edge Node
  -> MQTT Broker
  -> Go IoTKit Edge
  -> Output Adapter
  -> external application
```

### 10–30 minutes: run one focused test in each area

```bash
# Rust Edge Node and Adapter side
cargo test -p bravepi-mainboard-adapter

# Go IoTKit Edge side
(cd iotkit-edge && go test ./internal/outputadapter)

# Browser behavior and generated Console types
npm ci --prefix iotkit-edge/frontend
npm run check --prefix iotkit-edge/frontend
```

### 30–45 minutes: exercise the product without hardware

These scripts create disposable environments and use synthetic records:

```bash
# Clean bootstrap, TLS, login, Broker ACL, and Edge startup
scripts/test-edge-bootstrap.sh

# Semantic Output Adapter, MQTT PUBACK, outage, and restart convergence
scripts/test-edge-output.sh
```

Both require Docker access. They must not reuse production databases,
credentials, certificates, or deployment directories.

### 45–60 minutes: trace a small change

Pick one existing test close to the area you want to change. Follow its call
path into product code, make the smallest change, and rerun that focused test.
Use the table below instead of searching the whole repository blindly.

## Where to make a change

| Goal | Start here | Focused verification |
|---|---|---|
| Change protocol-independent domain behavior | `core/*` | `cargo test -p <owning-crate>` |
| Add or change a sensor IC conversion | `iotkit-sensor-drivers/` | `cargo test -p iotkit-sensor-drivers` |
| Change BravePI UART decoding or mapping | `bravepi-mainboard-adapter/` | `cargo test -p bravepi-mainboard-adapter` |
| Add a different device family | a top-level `*-adapter` crate and the Input Adapter contract | Adapter conformance tests plus `scripts/check-layers` |
| Change Edge Node composition or CLI | `iotkit-edge-node/`, `iotkit-edge-nodectl/` | the owning package tests |
| Change raw acceptance, semantics, accounts, backup, or output | `iotkit-edge/internal/` | `(cd iotkit-edge && go test ./internal/<package>)` |
| Change Console browser behavior | `iotkit-edge/frontend/src/` | `npm run check --prefix iotkit-edge/frontend` |
| Change Console HTML or navigation | `iotkit-edge/internal/edgehttp/` | Go edgehttp tests and `scripts/test-edge-console-e2e.sh` |
| Change a browser JSON API | `iotkit-edge/openapi/edge-console-v1.yaml` first | regenerate types, then frontend and edgehttp tests |
| Change a public wire contract | paired contract docs, exported types/schema, shared fixture, and conformance tests | the complete contract gate |
| Change installation or recovery | `scripts/`, `deploy/`, paired operations docs | the relevant Docker/PostgreSQL/security script |

The complete crate map and placement rules live in the architecture document.
Do not create a new crate until it is classified there and in
`scripts/check-layers`.

## One issue, one worktree, one pull request

Every development task uses the following loop:

1. Create or select one GitHub issue with a clear outcome and exclusions.
2. Update local `master`, then create `agent/issue-<number>-<slug>`.
3. Create `.worktrees/issue-<number>-<slug>` and work only there.
4. Add or update the closest focused test before changing product behavior.
5. Keep the diff inside the issue scope. Open another issue when the scope
   changes materially.
6. Commit, push the branch, and open a draft pull request that closes the issue.
7. Stop and request human review. Do not merge the pull request yourself.
8. Apply review feedback on the same branch and pull request.

Example:

```bash
git switch master
git pull --ff-only --prune
git worktree add .worktrees/issue-123-example \
  -b agent/issue-123-example origin/master
cd .worktrees/issue-123-example
```

Merged branches may be deleted on GitHub. After merge, return to the main
checkout, then clean local references and remove the corresponding worktree:

```bash
git worktree remove .worktrees/issue-123-example
git branch -d agent/issue-123-example
git pull --prune
```

## Verification ladder

Run the smallest command that can disprove your change first, then widen only
as the risk grows.

```bash
# Documentation structure
node scripts/check-okf-docs.mjs

# Rust formatting, dependency rules, source/test placement, tests, Clippy,
# and all Go tests
scripts/verify.sh

# Console schema, generated types/assets, and unit tests
scripts/test-edge-console-frontend.sh

# Console operator journey in Chromium
scripts/test-edge-console-e2e.sh

# MQTT output and PostgreSQL variants
scripts/test-edge-output.sh
IOTKIT_TEST_STORAGE_PROFILE=postgres scripts/test-edge-output.sh
scripts/test-edge-postgres.sh
```

Use `scripts/verify.sh` for Rust product behavior or uncertain cross-component
changes. Documentation-only changes normally need the documentation checker,
link/command inspection, and `git diff --check`. Run
`scripts/test-edge-host-release-gate.sh` once for a release candidate, not for
every pull request.

## Generated files and contract changes

- Edit `iotkit-edge/openapi/edge-console-v1.yaml`, then run
  `npm run generate:api --prefix iotkit-edge/frontend`.
- Build embedded Console JavaScript with
  `npm run build --prefix iotkit-edge/frontend`.
- Update `Cargo.lock`, `go.sum`, and `package-lock.json` only through their
  package managers.
- Change Japanese and English files under `docs/okf/` together.
- Treat shared JSON under `testdata/` as normative contract data. Do not update
  a fixture merely to make one implementation pass.

## Non-negotiable boundaries

- Never expose tokens, credentials, keys, or their hashes in logs, errors,
  audits, fixtures, or pull requests.
- Never acknowledge data before its documented durability point or silently
  delete an unacknowledged original.
- Route state changes through the owning typed operation dispatcher. Do not add
  direct SQL writes from HTTP, UI, CLI, or adapters.
- Keep Rust product test implementations outside product `src/` directories,
  Go tests in `*_test.go`, and frontend unit tests under
  `iotkit-edge/frontend/tests/unit/`.
- Do not make legacy plans or extracted old code the authority for new behavior.

## Pull request checklist

- The PR links and closes exactly one issue.
- The description explains what changed, why, impact, and verification.
- Public behavior has an executable test or fixture.
- Contract changes update all representations together.
- Documentation is updated when an operator or contributor workflow changes.
- No unrelated refactor, secret, local database, generated certificate, or
  deployment artifact is included.
- The branch is ready for human review but remains unmerged.
