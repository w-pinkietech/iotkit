# Contributing to IoTKit

[日本語](CONTRIBUTING.ja.md) | English

Thank you for helping make IoT data collection easier to operate and maintain.
IoTKit values field evidence, explicit data ownership, and changes that a new
maintainer can understand without reconstructing the project's history.

## Start here

For initial repository orientation, read these in order once:

1. [Product model](docs/product/en/concepts/product-model.md) — what IoTKit owns and
   what remains in devices or external applications.
2. [Architecture](docs/product/en/architecture/system-overview.md) — runtime
   components, crate map, code placement, and dependency rules.
3. The relevant [current contract](docs/product/en/index.md#contracts) — ingest,
   Input Adapter, Edge Node custody, or Output Adapter.
4. [AGENTS.md](AGENTS.md) — index of repository rules for people and coding
   agents (issue-driven workflow, invariants, lanes). Details live under
   [`.agents/`](.agents/).

`docs/product/` is the current human-readable product authority. It is packaged
as an OKF v0.2 bundle (format, not a second corpus). `docs/okf/` is only a
compatibility stub. `docs/redesign/` and `docs/superpowers/` preserve history;
they do not override current contracts, executable fixtures, or tests.

Keep product docs current in the same change that alters lasting product facts.
Temporary investigation notes stay on the issue or PR. Details:
[`.agents/documentation-authority.md`](.agents/documentation-authority.md).

For a lower-bound list of product docs that may need a freshness update:

```bash
node scripts/product-docs-impact.mjs select --base origin/master
```

Empty selection is not proof that no product-doc update is needed. After edits,
run `node scripts/check-product-docs.mjs`.

Pull-request CI runs a **soft** freshness warning when the selector finds
candidates but the PR neither updates `docs/product/` nor fills the template’s
“No product-docs update reason”. The step never fails the job. If you see the
warning: update the matching product docs (ja+en) or write a concrete reason in
the PR body, then re-run checks.

For each later task, use the **Before changing code** table in
[`.agents/change-map.md`](.agents/change-map.md) and read only the rows relevant
to that change. Work is issue-driven; see [`.agents/workflow.md`](.agents/workflow.md).

## Development environment

The supported contributor environment is Linux. CI currently uses:

- Rust 1.95.0, selected automatically by `rust-toolchain.toml`;
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

Install Rust through [rustup](https://rustup.rs/) and Node.js 22 through your
normal package manager. Do not commit credentials, generated certificates,
local databases, or deployment output directories.

## First hour

### 0–10 minutes: establish the map

```bash
git clone git@github.com:w-pinkietech/iotkit-next.git
cd iotkit-next
node scripts/check-product-docs.mjs
scripts/check-layers
scripts/check-source-layout
```

Then read the product model and the architecture document linked above. The
short version of the runtime path is:

```text
sensor / device
  -> Rust IoTKit Edge Node
  -> MQTT Broker
  -> Rust IoTKit Edge
  -> Output Adapter
  -> external application
```

### 10–30 minutes: run one focused test in each area

```bash
# Edge Node and Input Adapter
cargo test -p bravepi-mainboard-adapter

# IoTKit Edge and Output Adapters
cargo test -p iotkit-edge
cargo test -p iotkit-output-adapter-testkit

# Browser behavior and generated Console types
npm ci --prefix edge/frontend
npm run check --prefix edge/frontend
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
Use the **Before changing code** table in
[`.agents/change-map.md`](.agents/change-map.md) instead of searching the whole
repository blindly.

## Where to make a change

The task-routing table in [`.agents/change-map.md`](.agents/change-map.md) is the
single repository map for required reading, code entry points, authenticated
HTTP ingest, Console authentication, operations, and contracts. The complete
crate map and placement rules live in the architecture document. Do not create a
new crate until it is classified there and in `scripts/check-layers`.

Use the component entry points for a shorter local tour:

- [`edge-node/README.md`](edge-node/README.md) for collection and custody;
- [`edge-node/adapters/README.md`](edge-node/adapters/README.md) for Transport,
  Driver, and Input Adapter work;
- [`edge/README.md`](edge/README.md) for raw acceptance, semantics, output, and
  Console work.

For a new sensor, first decide whether it can use authenticated HTTP ingest,
fits the existing direct-I2C adapter, or needs a genuinely new adapter family.
Do not create a new family merely to add another supported IC.

## One issue, one worktree, one pull request

Every development task uses the following loop:

1. Create or select one GitHub issue with a clear outcome and exclusions.
2. Update local `master`, then create `agent/issue-<number>-<slug>`.
3. Create `.worktrees/issue-<number>-<slug>` and work only there.
4. Add or update the closest focused test before changing product behavior.
5. Keep the diff inside the issue scope. Open another issue when the scope
   changes materially.
6. Commit, push the branch, and open a draft pull request that closes the issue.
7. Stop and request human review.
8. Apply review feedback on the same branch and pull request.
9. Merge only after explicit approval.

For the final review, start at the [review suite](review/README.md) and pick
matching perspectives. Always consider the battle-tested perspective for product
or operations-touching diffs; select only the field failure questions related to
the change:

```bash
node scripts/battle-tested-review.mjs select --base origin/master
```

See the [battle-tested perspective](review/battle-tested/README.md) for selection,
redaction, triage, and promotion rules. Use the GitHub
`Field report / 現場報告` issue form for problems found in a real installation.
Do not attach raw logs, configuration, databases, credentials, or customer,
factory, network, or device identifiers.

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
node scripts/check-product-docs.mjs

# Rust formatting, dependency rules, source/test placement, tests, and Clippy
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

- Edit `edge/openapi/edge-console-v1.yaml`, then run
  `npm run generate:api --prefix edge/frontend`.
- Build embedded Console JavaScript with
  `npm run build --prefix edge/frontend`.
- Update `Cargo.lock` and `package-lock.json` only through their package
  managers.
- Change Japanese and English files under `docs/product/` together and bump the
  shared `revision` when concept content changes.
- Treat shared JSON under `testdata/` as normative contract data. Do not update
  a fixture merely to make one implementation pass.

## Non-negotiable boundaries

- Never expose tokens, credentials, keys, or their hashes in logs, errors,
  audits, fixtures, or pull requests.
- Never acknowledge data before its documented durability point or silently
  delete an unacknowledged original.
- Route state changes through the owning typed operation dispatcher. Do not add
  direct SQL writes from HTTP, UI, CLI, or adapters.
- Keep Rust product test implementations outside product `src/` directories
  and frontend unit tests under `edge/frontend/tests/unit/`.
- Do not make legacy plans or extracted old code the authority for new behavior.

## Pull request checklist

- The PR links and closes exactly one issue.
- The description explains what changed, why, impact, and verification.
- Public behavior has an executable test or fixture.
- Contract changes update all representations together.
- Documentation is updated when an operator or contributor workflow changes.
- Related battle-tested IDs are recorded, or the PR explains why none apply.
- No unrelated refactor, secret, local database, generated certificate, or
  deployment artifact is included.
- The branch is ready for human review but remains unmerged.
