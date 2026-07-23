# Selective CI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep lightweight repository checks on every CI run while skipping unrelated Rust and IoTKit Edge test jobs.

**Architecture:** A dependency-free Node.js classifier maps changed repository paths to `rust` and `edge` booleans. The existing GitHub Actions workflow computes the event diff once, always runs lightweight checks, and conditionally runs the unchanged heavy suites from classifier outputs.

**Tech Stack:** Node.js 22 built-ins, `node:test`, Bash, GitHub Actions YAML.

## Global Constraints

- Do not add a third-party changed-files action.
- Documentation-only changes select neither heavy job.
- Unknown paths, workflow changes, missing bases, and empty input select both jobs.
- Preserve the existing Rust and IoTKit Edge test commands.
- Keep the `CI` workflow and stable heavy-job display names.

---

### Task 1: Test and implement changed-path classification

**Files:**
- Create: `scripts/select-ci-jobs.mjs`
- Create: `scripts/tests/select-ci-jobs.test.mjs`

**Interfaces:**
- Consumes: newline-delimited repository-relative paths on standard input.
- Produces: `rust=true|false` and `edge=true|false`, one per line on standard output.
- Exports: `selectCiJobs(paths: string[]): { rust: boolean, edge: boolean }`.

- [ ] **Step 1: Write the failing classifier tests**

Create table-driven `node:test` cases for documentation-only, Rust-only,
IoTKit Edge-only, shared contract fixture, workflow, classifier, unknown, and
empty inputs:

```js
import assert from "node:assert/strict";
import test from "node:test";
import { selectCiJobs } from "../select-ci-jobs.mjs";

const cases = [
  ["documentation only", ["docs/okf/en/index.md", "AGENTS.md"], false, false],
  ["Rust only", ["core/ledger/src/lib.rs", "Cargo.lock"], true, false],
  ["IoTKit Edge only", ["iotkit-edge/internal/store/store.go"], false, true],
  ["shared fixture", ["contracts/fixtures/observation.json"], true, true],
  ["workflow", [".github/workflows/ci.yml"], true, true],
  ["classifier", ["scripts/select-ci-jobs.mjs"], true, true],
  ["unknown", ["new-component/file.txt"], true, true],
  ["empty", [], true, true],
];

for (const [name, paths, rust, edge] of cases) {
  test(name, () => assert.deepEqual(selectCiJobs(paths), { rust, edge }));
}
```

- [ ] **Step 2: Run the test and confirm the missing-module failure**

Run: `node --test scripts/tests/select-ci-jobs.test.mjs`

Expected: FAIL because `scripts/select-ci-jobs.mjs` does not exist.

- [ ] **Step 3: Implement the minimal classifier and CLI**

Implement explicit lightweight paths, Rust workspace roots, Edge roots, and
shared paths. Fall back to both jobs for any unclassified path. Read standard
input only when the module is the command entrypoint and emit GitHub-output
compatible lines.

- [ ] **Step 4: Run classifier tests**

Run: `node --test scripts/tests/select-ci-jobs.test.mjs`

Expected: all cases PASS.

- [ ] **Step 5: Commit the classifier**

```bash
git add scripts/select-ci-jobs.mjs scripts/tests/select-ci-jobs.test.mjs
git commit -m "ci: classify changes for selective jobs"
```

### Task 2: Route GitHub Actions jobs through the classifier

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `scripts/tests/select-ci-jobs.test.mjs`

**Interfaces:**
- Consumes: pull-request base/head SHAs or push before/after SHAs.
- Produces: `changes.outputs.rust` and `changes.outputs.edge`.
- `lightweight` always runs; `check` and `edge` depend on classifier outputs.

- [ ] **Step 1: Add a failing workflow-shape test**

Read `.github/workflows/ci.yml` as text and assert that it contains:

```js
assert.match(workflow, /id: select/);
assert.match(workflow, /node scripts\/select-ci-jobs\.mjs/);
assert.match(workflow, /needs\.changes\.outputs\.rust == 'true'/);
assert.match(workflow, /needs\.changes\.outputs\.edge == 'true'/);
assert.match(workflow, /name: lightweight repository checks/);
```

- [ ] **Step 2: Run the test and confirm it fails**

Run: `node --test scripts/tests/select-ci-jobs.test.mjs`

Expected: FAIL because the workflow has no selection job or conditional jobs.

- [ ] **Step 3: Refactor the workflow**

Add a `changes` job with full-history checkout. Resolve `BASE_SHA` and
`HEAD_SHA` from the event, send `git diff --name-only` to the classifier, and
append its output to `$GITHUB_OUTPUT`. If the base is missing, all zeroes, or not
a commit, send empty input so the classifier selects both jobs.

Move OKF, layer, source-layout, and battle-tested checks into `lightweight`.
Keep the Rust commands in job `check` and the Go/Console commands in job `edge`.
Add `needs: changes` and the corresponding output condition to each heavy job.

- [ ] **Step 4: Run focused and repository checks**

Run:

```bash
node --test scripts/tests/select-ci-jobs.test.mjs
node scripts/check-okf-docs.mjs
scripts/check-layers
scripts/check-source-layout
node scripts/battle-tested-review.mjs check
node --test scripts/tests/battle-tested-review.test.mjs
git diff --check
```

Expected: all commands PASS.

- [ ] **Step 5: Exercise CLI output**

Run:

```bash
printf '%s\n' docs/README.md | node scripts/select-ci-jobs.mjs
printf '%s\n' core/ledger/src/lib.rs | node scripts/select-ci-jobs.mjs
printf '%s\n' iotkit-edge/internal/store/store.go | node scripts/select-ci-jobs.mjs
```

Expected outputs, respectively:

```text
rust=false
edge=false
```

```text
rust=true
edge=false
```

```text
rust=false
edge=true
```

- [ ] **Step 6: Commit the workflow**

```bash
git add .github/workflows/ci.yml scripts/tests/select-ci-jobs.test.mjs
git commit -m "ci: skip unrelated heavy test jobs"
```

### Task 3: Review and publish

**Files:**
- Review: all issue #79 changes

**Interfaces:**
- Consumes: complete branch diff against `master`.
- Produces: a reviewed draft pull request closing issue #79.

- [ ] **Step 1: Run the battle-tested selector**

Run: `node scripts/battle-tested-review.mjs select --base master`

Expected: changed CI and classifier paths are visible; unmatched paths are not
treated as evidence of safety.

- [ ] **Step 2: Review failure behavior**

Confirm that every detection failure chooses both heavy jobs, the lightweight
job has no path condition, and no existing heavy-suite command was removed.

- [ ] **Step 3: Run final focused verification**

Repeat Task 2 Step 4 from a clean worktree and confirm every command passes.

- [ ] **Step 4: Commit any review corrections**

If review changes are required, commit only those corrections:

```bash
git add .github/workflows/ci.yml scripts/select-ci-jobs.mjs scripts/tests/select-ci-jobs.test.mjs
git commit -m "fix(ci): harden selective job routing"
```

- [ ] **Step 5: Push and open a draft pull request**

Push `agent/issue-79-selective-ci` and open a draft PR with `Closes #79`, the
classification rules, verification evidence, and any intentionally skipped
product suites.
