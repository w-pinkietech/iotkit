# Repository Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reorganize IoTKit by product component and extension role without changing package, binary, wire, persisted, or runtime behavior.

**Architecture:** Keep the root Cargo workspace, but move all collection-side Rust code beneath `edge-node/` and the Go service beneath `edge/`. Within `edge-node/`, distinguish composition roots, core services, ingest boundaries, input infrastructure, concrete adapters, and tools by directory while preserving every Cargo package and target name.

**Tech Stack:** Git path moves, Cargo workspace manifests, Go module, Node.js repository checks, GitHub Actions, Markdown/OKF documentation.

## Global Constraints

- This is a path-only architecture change; do not modify product behavior.
- Preserve Cargo package names, Rust library names, Go module path, binary names, MQTT/TLS/audit identifiers, schemas, and database values.
- Keep English and Japanese current documentation synchronized.
- Unknown or moved paths must remain covered by CI and battle-tested review routing.
- Run the full repository verification once after the final path state.

---

### Task 1: Move product components

**Files:**
- Move all Rust workspace crates into `edge-node/`
- Move `iotkit-edge/` to `edge/`
- Move `rewrite-prep.md` to `docs/redesign/rewrite-prep.md`

**Target mapping:**

```text
core/                                      -> edge-node/core/
iotkit-edge-node/                          -> edge-node/apps/node/
iotkit-edge-nodectl/                       -> edge-node/apps/nodectl/
iotkit-ingest-{contract,client,http}/       -> edge-node/ingest/{contract,client,http}/
iotkit-input-adapter-host-api/              -> edge-node/input/host-api/
iotkit-input-adapter-testkit/               -> edge-node/input/testkit/
iotkit-polling-adapter-runtime/             -> edge-node/input/runtimes/polling/
iotkit-sensor-drivers/                      -> edge-node/input/hardware/sensor-drivers/
rpi4b-transport/                            -> edge-node/input/hardware/transports/rpi/
bravepi-mainboard-adapter/                  -> edge-node/adapters/bravepi-mainboard/
rpi-local-adapter/                          -> edge-node/adapters/rpi-local/
bravepi-mainboard-adapter/poc/              -> edge-node/tools/bravepi-poc/
iotkit-edge/                                -> edge/
rewrite-prep.md                             -> docs/redesign/rewrite-prep.md
```

- [ ] Move paths with Git-aware mechanical operations.
- [ ] Confirm no old tracked source directory remains.
- [ ] Confirm package names in every moved `Cargo.toml` are unchanged.

### Task 2: Repair build and executable path references

**Files:**
- Modify: `Cargo.toml`
- Modify: moved `Cargo.toml` files
- Modify: `scripts/layer-fixtures/**/Cargo.toml`
- Modify: moved Rust tests with workspace-root assumptions
- Modify: `edge/frontend/scripts/*.mjs`
- Modify: `deploy/*.yaml`, `compose.dev.yaml`, `scripts/*.sh`

- [ ] Update workspace members to the target tree.
- [ ] Update every Cargo path dependency relative to its new manifest.
- [ ] Update docs/testdata discovery that previously assumed a crate parent was the repository root.
- [ ] Update Go/Console/Docker working paths from `iotkit-edge/` to `edge/`.
- [ ] Run `cargo metadata --no-deps` and fix only path-resolution errors.
- [ ] Run `(cd edge && go list ./...)` and frontend generated-file checks.

### Task 3: Repair repository policy and review routing

**Files:**
- Modify: `scripts/check-layers`
- Modify: `scripts/select-ci-jobs.mjs`
- Modify: `scripts/tests/select-ci-jobs.test.mjs`
- Modify: `review/battle-tested/catalog.json`
- Modify: `scripts/tests/battle-tested-review.test.mjs`

- [ ] Update layer classifications to the new Rust paths.
- [ ] Update selective CI to classify `edge-node/` as Rust and `edge/` as Go/Console.
- [ ] Update routing tests before changing their implementation expectations.
- [ ] Update battle-tested path prefixes and provenance links.
- [ ] Run layer, source-layout, CI selector, and battle-tested tests.

### Task 4: Make the new structure discoverable

**Files:**
- Modify: `README.md`, `README.ja.md`
- Modify: `CONTRIBUTING.md`, `CONTRIBUTING.ja.md`
- Modify: `AGENTS.md`
- Modify: `docs/okf/en/architecture/system-overview.md`
- Modify: `docs/okf/ja/architecture/system-overview.md`
- Modify: affected current contracts in both languages
- Create: `edge-node/README.md`
- Create: `edge-node/adapters/README.md`
- Create: `edge/README.md`

- [ ] Update every current path reference.
- [ ] Add the three-way integration decision: authenticated HTTP device, existing direct-I2C model, or new Adapter family.
- [ ] Explain Transport → Driver → Input Adapter → ingest without duplicating contract semantics.
- [ ] Keep local READMEs thin: purpose, non-purpose, first code entry, focused test, canonical links.
- [ ] Run the bilingual OKF checker and search current docs for stale old paths.

### Task 5: Verify and review

**Files:**
- Review all issue #81 changes.

- [ ] Run `cargo metadata --no-deps`.
- [ ] Run `scripts/check-layers` and `scripts/check-source-layout`.
- [ ] Run selective CI and battle-tested tests.
- [ ] Run Console frontend and E2E checks.
- [ ] Run `TMPDIR="$PWD/target/tmp" scripts/verify.sh`.
- [ ] Review `git diff --summary` to confirm moves dominate and package/binary/protocol names did not change.
- [ ] Run an independent fresh review and correct all Critical/Important findings.
- [ ] Push the branch and open a draft PR closing issue #81.
