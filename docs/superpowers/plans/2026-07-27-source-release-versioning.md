# Source Release Versioning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish one visible `0.1.0` IoTKit product version and a repeatable source-only GitHub Release procedure.

**Architecture:** The root Cargo workspace package version is the only product-version authority, and every workspace crate inherits it. Compile-time Cargo metadata feeds the three shipped CLIs, the Edge Node status API, and the authenticated Console; a read-only repository checker prevents Cargo, README, changelog, repository URL, and release-tag drift.

**Tech Stack:** Rust 1.95.0, Cargo workspaces, Clap 4, Askama, Node.js 22, Git, GitHub CLI, WSL Ubuntu 26.04

## Global Constraints

- The first product version is exactly `0.1.0`; its Git tag is exactly `v0.1.0`.
- All Cargo workspace members use the same product version.
- Layer fixtures under `scripts/layer-fixtures/` remain explicit `0.0.0` non-product packages.
- `0.MINOR.0` may add features or intentionally change compatibility.
- `0.MINOR.PATCH` does not intentionally change compatibility.
- Product versioning remains separate from API, MQTT, disk, snapshot, adapter, configuration, and OKF format versions.
- The first GitHub Release contains only GitHub-generated source archives.
- Do not add binary, container, OS image, signature, checksum, SBOM, or A/B-update publishing.
- Do not create, push, move, or delete `v0.1.0` while implementing this plan.
- Do not publish a GitHub Release before the implementation PR is merged and publication is explicitly approved.
- CLI version paths must not initialize storage, read credentials, contact a Broker, or start a listener.

---

### Task 1: Prepare a reproducible local verification environment

**Files:**
- Read: `rust-toolchain.toml`
- Read: `scripts/cloud-setup.sh`
- No repository files change in this task.

**Interfaces:**
- Consumes: WSL distribution `Ubuntu-26.04` and the pinned Rust toolchain in `rust-toolchain.toml`.
- Produces: Rust 1.95.0, rustfmt, Clippy, Node.js, native build dependencies, and a clean baseline result for this worktree.

- [x] **Step 1: Install Linux build prerequisites in the existing WSL distribution**

Run from PowerShell:

```powershell
wsl.exe -d Ubuntu-26.04 -u root -- bash -lc 'apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y build-essential pkg-config libudev-dev curl ca-certificates git nodejs npm'
```

Expected: exit 0; `pkg-config`, `libudev-dev`, `curl`, `git`, `node`, and `npm` are installed.

- [x] **Step 2: Install rustup for the normal WSL user**

Run:

```powershell
wsl.exe -d Ubuntu-26.04 -- bash -lc 'if ! command -v rustup >/dev/null 2>&1; then curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/iotkit-rustup-init.sh && sh /tmp/iotkit-rustup-init.sh -y --profile minimal --default-toolchain none; fi'
```

Expected: exit 0; the installer is stored only in `/tmp`, and rustup is installed below the WSL user's standard Cargo directory.

- [x] **Step 3: Install the repository-pinned toolchain and fetch locked dependencies**

Run:

```powershell
wsl.exe -d Ubuntu-26.04 -- bash -lc 'export PATH="$HOME/.cargo/bin:$PATH"; cd /mnt/c/Users/watak/Documents/iotkit/.worktrees/issue-102-source-release-versioning; rustup show active-toolchain; bash scripts/cloud-setup.sh'
```

Expected: the active toolchain is `1.95.0`; `cargo fetch --locked` succeeds.

- [x] **Step 4: Run the clean baseline**

Run:

```powershell
wsl.exe -d Ubuntu-26.04 -- bash -lc 'export PATH="$HOME/.cargo/bin:$PATH"; export CARGO_TARGET_DIR="$HOME/.cache/iotkit-target/issue-102"; cd /mnt/c/Users/watak/Documents/iotkit/.worktrees/issue-102-source-release-versioning; cargo test --workspace'
```

Expected: all workspace tests pass; hardware-only tests remain ignored by their existing annotations.

- [x] **Step 5: Record environment evidence in the implementation notes**

Run:

```powershell
wsl.exe -d Ubuntu-26.04 -- bash -lc 'export PATH="$HOME/.cargo/bin:$PATH"; rustc --version; cargo --version; cargo fmt --version; cargo clippy --version; node --version'
```

Expected: Rust commands report 1.95.0-compatible tools and Node reports a version capable of running the repository's `.mjs` tests.

No commit is created because this task changes only the local development environment.

### Task 2: Make the Cargo workspace version authoritative

**Files:**
- Create: `scripts/check-release-version.mjs`
- Create: `scripts/tests/release-version.test.mjs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: every workspace member manifest listed in the root `Cargo.toml`

Workspace member manifests:

```text
edge/Cargo.toml
edge/custody-contract/Cargo.toml
edge/output-adapters/api/Cargo.toml
edge/output-adapters/testkit/Cargo.toml
edge/output-adapters/example/Cargo.toml
edge/output-adapters/generic-mqtt-json-v1/Cargo.toml
edge/output-adapters/pinikiet-mqtt-v1/Cargo.toml
edge-node/ingest/contract/Cargo.toml
edge-node/ingest/client/Cargo.toml
edge-node/ingest/http/Cargo.toml
edge-node/input/host-api/Cargo.toml
edge-node/input/testkit/Cargo.toml
edge-node/core/types/Cargo.toml
edge-node/core/engine/Cargo.toml
edge-node/core/supervision/Cargo.toml
edge-node/core/storage/Cargo.toml
edge-node/core/ledger/Cargo.toml
edge-node/core/timeseries/Cargo.toml
edge-node/core/publish/Cargo.toml
edge-node/core/collector/Cargo.toml
edge-node/core/registry/Cargo.toml
edge-node/core/ops/Cargo.toml
edge-node/input/hardware/transports/rpi/Cargo.toml
edge-node/adapters/bravepi-mainboard/Cargo.toml
edge-node/adapters/bravepi-mainboard/codec/Cargo.toml
edge-node/input/hardware/sensor-drivers/Cargo.toml
edge-node/tools/bravepi-poc/Cargo.toml
edge-node/adapters/rpi-local/Cargo.toml
edge-node/apps/node/Cargo.toml
edge-node/apps/nodectl/Cargo.toml
edge-node/input/runtimes/polling/Cargo.toml
```

**Interfaces:**
- Consumes: root `[workspace.package]` metadata and `cargo metadata --no-deps --format-version 1`.
- Produces: `extractWorkspaceVersion(text) -> string`, `validateReleaseState(input) -> string[]`, and a CLI that exits 0 only when release metadata is consistent.

- [x] **Step 1: Write failing unit tests for workspace and tag consistency**

Create `scripts/tests/release-version.test.mjs` with these tests:

```javascript
import assert from "node:assert/strict";
import test from "node:test";
import {
  extractWorkspaceVersion,
  validateReleaseState,
} from "../check-release-version.mjs";

test("extracts the workspace package version", () => {
  assert.equal(
    extractWorkspaceVersion(`[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.1.0"\n`),
    "0.1.0",
  );
});

test("accepts one inherited 0.1.0 product version", () => {
  assert.deepEqual(
    validateReleaseState({
      version: "0.1.0",
      packages: [{ name: "iotkit-edge", version: "0.1.0", inheritsVersion: true }],
      repository: "https://github.com/w-pinkietech/iotkit",
      tag: "v0.1.0",
    }),
    [],
  );
});

test("reports package, document, repository, and tag drift together", () => {
  const problems = validateReleaseState({
    version: "0.1.0",
    packages: [{ name: "iotkit-edge-node", version: "0.2.0", inheritsVersion: false }],
    repository: "https://github.com/w-pinkietech/iotkit-next",
    tag: "0.1.0",
  });
  assert.equal(problems.length, 4);
});
```

- [x] **Step 2: Run the tests and verify RED**

Run in WSL:

```bash
node --test scripts/tests/release-version.test.mjs
```

Expected: FAIL because `scripts/check-release-version.mjs` does not exist.

- [x] **Step 3: Implement the checker functions and CLI**

Create `scripts/check-release-version.mjs` with these behaviors:

```javascript
export function extractWorkspaceVersion(cargoToml) {
  const section = cargoToml.match(
    /^\[workspace\.package\]\r?\n((?:(?!^\[)[\s\S])*)/m,
  );
  const version = section?.[1].match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
  if (!version) throw new Error("Cargo.toml is missing [workspace.package] version");
  return version;
}

export function validateReleaseState(state) {
  const problems = [];
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(state.version)) {
    problems.push(`workspace version is not MAJOR.MINOR.PATCH SemVer: ${state.version}`);
  }
  for (const pkg of state.packages) {
    if (pkg.version !== state.version) problems.push(`${pkg.name} resolves to ${pkg.version}`);
    if (!pkg.inheritsVersion) problems.push(`${pkg.name} does not use version.workspace = true`);
  }
  if (state.repository !== "https://github.com/w-pinkietech/iotkit") problems.push("workspace repository URL differs");
  if (state.tag && state.tag !== `v${state.version}`) problems.push(`tag must be v${state.version}`);
  return problems;
}
```

The CLI reads repository files, runs `cargo metadata --no-deps --format-version 1`,
checks each returned workspace package manifest for `version.workspace = true`,
accepts an optional `--tag vX.Y.Z`, prints every package, repository, or tag
problem to stderr, and exits 1 when any problem exists. Task 5 extends the same
state and CLI with README and changelog validation.

- [x] **Step 4: Move product package metadata to the workspace**

Set the root authority:

```toml
[workspace.package]
version = "0.1.0"
license = "Apache-2.0"
repository = "https://github.com/w-pinkietech/iotkit"
```

In every listed workspace member manifest, replace:

```toml
version = "0.1.0"
```

with:

```toml
version.workspace = true
```

Do not modify `scripts/layer-fixtures/**/Cargo.toml`.

- [x] **Step 5: Refresh the lockfile without updating dependencies**

Run:

```bash
cargo metadata --locked --no-deps --format-version 1 >/dev/null
```

If Cargo reports that the lockfile needs an update, rerun once without
`--locked`, inspect that only workspace package metadata changed, then rerun
with `--locked`.

- [x] **Step 6: Run GREEN verification**

Run:

```bash
node --test scripts/tests/release-version.test.mjs
node scripts/check-release-version.mjs --tag v0.1.0
cargo metadata --locked --no-deps --format-version 1 >/dev/null
```

Expected: all commands exit 0.

- [x] **Step 7: Commit the authority and checker**

```bash
git add Cargo.toml Cargo.lock edge edge-node scripts/check-release-version.mjs scripts/tests/release-version.test.mjs
git commit -m "build: establish unified product version"
```

### Task 3: Expose the product version through all shipped CLIs

**Files:**
- Modify: `edge/tests/cli_contract.rs`
- Create: `edge-node/apps/node/tests/cli.rs`
- Modify: `edge-node/apps/node/src/main.rs`
- Modify: `edge-node/apps/nodectl/tests/cli.rs`
- Modify: `edge-node/apps/nodectl/src/main.rs`

**Interfaces:**
- Consumes: compile-time `env!("CARGO_PKG_VERSION")`.
- Produces: exact stdout lines for `iotkit-edge`, `iotkit-edge-node`, and `iotkit-edge-nodectl`; version paths exit 0 with empty stderr and no side effects.

- [x] **Step 1: Write failing process tests for Edge Node and nodectl**

Add:

```rust
#[test]
fn version_exits_without_starting_the_service() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_iotkit-edge-node"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("iotkit-edge-node {}\n", env!("CARGO_PKG_VERSION"))
    );
}
```

In `edge-node/apps/nodectl/tests/cli.rs`, add this assertion for
`iotkit-edge-nodectl`:

```rust
#[test]
fn version_exits_without_opening_a_database() {
    let output = edgectl().arg("--version").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("iotkit-edge-nodectl {}\n", env!("CARGO_PKG_VERSION"))
    );
}
```

- [x] **Step 2: Run the two tests and verify RED**

Run:

```bash
cargo test -p iotkit-edge-node --test cli version_exits_without_starting_the_service
cargo test -p iotkit-edge-nodectl --test cli version_exits_without_opening_a_database
```

Expected: Edge Node attempts normal startup or rejects the argument; nodectl
does not produce the required version output.

- [x] **Step 3: Implement minimal version exits**

In `edge-node/apps/node/src/main.rs`, inspect arguments before installing
crypto, logging, storage, or listeners:

```rust
fn version_requested(args: &[String]) -> bool {
    matches!(args, [_, value] if value == "--version")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if version_requested(&args) {
        println!("iotkit-edge-node {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    // existing startup follows
}
```

In `edge-node/apps/nodectl/src/main.rs`, change the Clap declaration to:

```rust
#[derive(Parser)]
#[command(name = "iotkit-edge-nodectl", version)]
struct Cli {
```

- [x] **Step 4: Add the existing IoTKit Edge behavior to the contract suite**

Add to `edge/tests/cli_contract.rs`:

```rust
#[test]
fn version_reports_the_workspace_product_version() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_iotkit-edge"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("iotkit-edge {}\n", env!("CARGO_PKG_VERSION"))
    );
}
```

- [x] **Step 5: Run GREEN verification**

Run:

```bash
cargo test -p iotkit-edge --test cli_contract version_reports_the_workspace_product_version
cargo test -p iotkit-edge-node --test cli version_exits_without_starting_the_service
cargo test -p iotkit-edge-nodectl --test cli version_exits_without_opening_a_database
```

Expected: all three tests pass with exact stdout and empty stderr.

- [x] **Step 6: Commit CLI visibility**

```bash
git add edge/tests/cli_contract.rs edge-node/apps/node/src/main.rs edge-node/apps/node/tests/cli.rs edge-node/apps/nodectl/src/main.rs edge-node/apps/nodectl/tests/cli.rs
git commit -m "feat: expose product version in CLIs"
```

### Task 4: Show the product version in the Console and Edge Node status contract

**Files:**
- Modify: `edge/src/web/mod.rs`
- Modify: `edge/src/composition/web.rs`
- Modify: `edge/src/web/templates/console.html`
- Modify: `edge/tests/console_contract.rs`
- Modify: `edge-node/apps/node/tests/api_basic.rs`

**Interfaces:**
- Consumes: compile-time `env!("CARGO_PKG_VERSION")`.
- Produces: `ConsoleView.product_version: String`, the system-page label `IoTKit Edge 0.1.0`, and an explicit Edge Node `/api/v1/box` version assertion.

- [x] **Step 1: Write the failing Console contract assertion**

Extend the `/system` expected text in `edge/tests/console_contract.rs`:

```rust
&[
    "IoTKit Edge 0.1.0",
    "保存データの状態",
    "raw受信データ",
    "確認が必要なこと",
    r#"class="storage-meter""#,
][..]
```

- [x] **Step 2: Run the Console test and verify RED**

Run:

```bash
cargo test -p iotkit-edge --test console_contract console_pages_render_the_existing_operator_content_and_form_hooks
```

Expected: FAIL because `/system` does not render `IoTKit Edge 0.1.0`.

- [x] **Step 3: Add the Console view field and template output**

Add to `ConsoleView`:

```rust
pub product_version: String,
```

Populate it in both the production view and authenticated stub view with:

```rust
product_version: env!("CARGO_PKG_VERSION").into(),
```

Add a system card before storage details:

```html
<section class="system-grid" aria-label="製品情報">
  <article class="system-card">
    <div>
      <h2>製品情報</h2>
      <strong>IoTKit Edge {{ view.product_version }}</strong>
      <p>IoTKit全体の製品バージョンです。</p>
    </div>
  </article>
</section>
```

Keep the version visible to every authenticated role and do not add a mutation.

- [x] **Step 4: Assert the existing Edge Node status version**

In `edge-node/apps/node/tests/api_basic.rs`, add after reading
`/api/v1/box`:

```rust
assert_eq!(
    box_before["version"],
    serde_json::Value::String(env!("CARGO_PKG_VERSION").into())
);
```

- [x] **Step 5: Run GREEN verification**

Run:

```bash
cargo test -p iotkit-edge --test console_contract
cargo test -p iotkit-edge-node --test api_basic box_setup_session_throttle_and_graceful_shutdown
```

Expected: both commands pass.

- [x] **Step 6: Commit runtime visibility**

```bash
git add edge/src/web/mod.rs edge/src/composition/web.rs edge/src/web/templates/console.html edge/tests/console_contract.rs edge-node/apps/node/tests/api_basic.rs
git commit -m "feat: show product version in Console"
```

### Task 5: Publish the pre-1.0 policy and source-release runbook

**Files:**
- Modify: `README.md`
- Modify: `README.ja.md`
- Create: `CHANGELOG.md`
- Create: `RELEASING.md`
- Modify: `scripts/tests/release-version.test.mjs`
- Modify: `scripts/check-release-version.mjs`

**Interfaces:**
- Consumes: workspace version `0.1.0`.
- Produces: bilingual current-version markers, a product changelog, and a release runbook that refuses tag mismatch and documents explicit publication authority.

- [x] **Step 1: Write failing checker tests for final document markers**

Add tests that pass these exact lines to the parser:

```text
> **Current product version: 0.1.0 (pre-1.0).**
> **現在の製品バージョン: 0.1.0（pre-1.0）。**
## [0.1.0] - 2026-07-27
```

Also assert that `0.1`, `v0.1.0` in a Cargo version field, and README
`0.2.0` are rejected.

- [x] **Step 2: Run the document-marker tests and verify RED**

Run:

```bash
node --test scripts/tests/release-version.test.mjs
```

Expected: FAIL until the checker recognizes and validates the final markers.

- [x] **Step 3: Replace the README v1 release-candidate copy**

Use this English status:

```markdown
> **Current product version: 0.1.0 (pre-1.0).** IoTKit is available as an
> early source release. APIs, the on-disk schema, and wire contracts may change
> during the 0.x series. See [GitHub Releases](https://github.com/w-pinkietech/iotkit/releases)
> and the [Roadmap](#roadmap).
```

Use this Japanese status:

```markdown
> **現在の製品バージョン: 0.1.0（pre-1.0）。** IoTKitは早期source releaseとして
> 公開しています。0.xの間はAPI、ディスク上のschema、wire contractが変更される可能性があります。
> [GitHub Releases](https://github.com/w-pinkietech/iotkit/releases)と
> [ロードマップ](#ロードマップ)を参照してください。
```

- [x] **Step 4: Write the initial changelog**

`CHANGELOG.md` contains:

```markdown
# Changelog

All notable user-visible and operational changes to IoTKit are recorded here.
Product versions do not replace versioned API, MQTT, disk, snapshot, adapter,
configuration, or OKF format identifiers.

## [Unreleased]

## [0.1.0] - 2026-07-27

- Initial public source release.
- Durable Edge Node collection and IoTKit Edge custody acknowledgement.
- Authenticated Console, semantic mapping, history, diagnostics, and backup.
- Durable generic MQTT JSON and Pinikiet output adapters.
```

- [x] **Step 5: Write the maintainer release runbook**

`RELEASING.md` documents:

```bash
node scripts/check-release-version.mjs --tag v0.1.0
scripts/verify.sh
git status --short --branch
git fetch origin master --tags
test "$(git rev-parse HEAD)" = "$(git rev-parse origin/master)"
git tag -a v0.1.0 -m "IoTKit v0.1.0"
git push origin refs/tags/v0.1.0
gh release create v0.1.0 \
  --repo w-pinkietech/iotkit \
  --verify-tag \
  --title "IoTKit v0.1.0" \
  --generate-notes \
  --notes "Early pre-1.0 source release. Product and public contracts may change during the 0.x series."
```

The document states that tag creation, push, and `gh release create` require
explicit approval; a tag is never force-moved or deleted by the normal
procedure; GitHub-generated source archives are the only assets.

- [x] **Step 6: Complete the checker document validation**

Update `scripts/check-release-version.mjs` to read both README markers and
all `CHANGELOG.md` version headings, then feed the extracted values into
`validateReleaseState`.

- [x] **Step 7: Run GREEN documentation verification**

Run:

```bash
node --test scripts/tests/release-version.test.mjs
node scripts/check-release-version.mjs --tag v0.1.0
node scripts/check-okf-docs.mjs
git diff --check
```

Expected: all commands exit 0.

- [x] **Step 8: Commit public release documentation**

```bash
git add README.md README.ja.md CHANGELOG.md RELEASING.md scripts/check-release-version.mjs scripts/tests/release-version.test.mjs
git commit -m "docs: define pre-1.0 source releases"
```

### Task 6: Enforce version consistency in CI and complete repository verification

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `docs/superpowers/plans/2026-07-27-source-release-versioning.md` only to check completed boxes during execution
- Read: `AGENTS.md`
- Read: `.agents/skills/iotkit-battle-tested-review/SKILL.md`

**Interfaces:**
- Consumes: `node scripts/check-release-version.mjs`.
- Produces: a lightweight CI gate, repository verification evidence, battle-tested review selection, and a draft PR that closes #102 without publishing a release.

- [x] **Step 1: Add the version gate to lightweight CI**

Add after Node setup and before documentation checks:

```yaml
      - name: Product release version consistency
        run: node scripts/check-release-version.mjs
```

- [x] **Step 2: Run focused CI checks**

Run:

```bash
node scripts/check-release-version.mjs --tag v0.1.0
node --test scripts/tests/release-version.test.mjs
node scripts/check-okf-docs.mjs
scripts/check-layers
scripts/check-source-layout
cargo test -p iotkit-edge --test cli_contract --test console_contract
cargo test -p iotkit-edge-node --test cli --test api_basic
cargo test -p iotkit-edge-nodectl --test cli version_exits_without_opening_a_database
```

Expected: all commands pass.

- [x] **Step 3: Run full Rust verification**

Run:

```bash
scripts/verify.sh
```

Expected: formatting, layer rules, workspace tests, and Clippy all pass with
warnings denied.

- [x] **Step 4: Run repository-local operational review routing**

Run:

```bash
node scripts/battle-tested-review.mjs select --base origin/master
```

Review every selected `BT-NNN` entry plus these semantic concerns:

```text
- --version exits before storage, credentials, Broker, and listeners
- one workspace product version does not replace contract/schema versions
- release instructions refuse tag mismatch and remote divergence
- no release tag, release page, binary, image, or artifact is published by the PR
```

- [x] **Step 5: Inspect the final diff and repository state**

Run:

```bash
git diff --check origin/master...HEAD
git diff --stat origin/master...HEAD
git status --short --branch
git log --oneline --decorate origin/master..HEAD
```

Expected: only #102 files are changed, the worktree is clean, and no tag exists.

- [x] **Step 6: Commit the CI gate**

```bash
git add .github/workflows/ci.yml docs/superpowers/plans/2026-07-27-source-release-versioning.md
git commit -m "ci: verify product release version"
```

- [x] **Step 7: Push and open the draft PR**

Use the repository publish workflow with:

```text
Branch: agent/issue-102-source-release-versioning
Title: build: establish 0.x source release versioning
Body:
- unify the Cargo workspace at product version 0.1.0
- expose the version through CLI, Edge Node status, Console, and README
- add changelog, source-release runbook, and CI consistency checking

Closes #102

The PR does not create v0.1.0 or publish a GitHub Release.
```

Stop for human review. Do not merge, tag, or publish.
