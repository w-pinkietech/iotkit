import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import test from "node:test";

import { selectCiJobs } from "../select-ci-jobs.mjs";

const root = new URL("../../", import.meta.url);
const file = (path) => new URL(path, root);
const read = (path) => readFileSync(file(path), "utf8");

const ci = () => read(".github/workflows/ci.yml");
const selectableJobs = ["lightweight", "check", "console", "edge", "trial"];
const ciOwnerJobs = new Map([
  ["CI lightweight", "lightweight"],
  ["CI Rust", "check"],
  ["CI Console", "console"],
  ["CI Edge", "edge"],
  ["CI trial", "trial"],
]);
const hostReleaseGate = "test-edge-host-release-gate.sh";

function jobSection(workflow, job) {
  const after = workflow.split(`\n  ${job}:\n`)[1];
  assert.ok(after, `missing ${job} job`);
  return after.split(/\n  [a-z][a-z-]*:\n/)[0];
}

function workflowRunBlock(workflow, job) {
  const run = jobSection(workflow, job).split("        run: |\n")[1];
  assert.ok(run, `${job} must have an executable run block`);
  return run
    .split("\n")
    .map((line) => (line.startsWith("          ") ? line.slice(10) : line))
    .join("\n");
}

function jobsRunning(workflow, command) {
  return selectableJobs.filter((job) => command.test(jobSection(workflow, job)));
}

function tableRows(source, heading) {
  const section = source.split(`## ${heading}\n`)[1];
  assert.ok(section, `missing ${heading} matrix section`);
  const table = section.split("\n\n")[0];
  const lines = table.split("\n").filter((line) => line.startsWith("|"));
  return lines
    .slice(2)
    .map((line) => line.split("|").slice(1, -1).map((cell) => cell.trim()));
}

function selectorLanes() {
  return Object.entries(selectCiJobs([]))
    .filter(([, value]) => typeof value === "boolean")
    .map(([output]) => {
      const job = output === "rust" ? "check" : output;
      return {
        output,
        job,
        selectedEnv: `${output.toUpperCase()}_SELECTED`,
        resultEnv: `${job.toUpperCase()}_RESULT`,
      };
    });
}

function aggregateRunBlock(workflow) {
  return workflowRunBlock(workflow, "required-ci");
}

function runRequiredCi(overrides = {}) {
  const laneEnv = Object.fromEntries(
    selectorLanes().flatMap(({ selectedEnv, resultEnv }) => [
      [selectedEnv, "false"],
      [resultEnv, "skipped"],
    ]),
  );
  return spawnSync("bash", ["-c", aggregateRunBlock(ci())], {
    encoding: "utf8",
    env: {
      ...process.env,
      CHANGES_RESULT: "success",
      LIGHTWEIGHT_RESULT: "success",
      ...laneEnv,
      ...overrides,
    },
  });
}

function testScripts() {
  return readdirSync(file("scripts"), { withFileTypes: true })
    .filter((entry) => entry.isFile() && /^test-[a-z0-9-]+\.sh$/.test(entry.name))
    .map((entry) => entry.name)
    .sort();
}

function scriptCalls(script) {
  return [
    ...read(`scripts/${script}`).matchAll(
      /\$\{?(?:repo_root|ROOT)\}?\/scripts\/(test-[a-z0-9-]+\.sh)/g,
    ),
  ].map(([, called]) => called);
}

function hostReleaseCalls() {
  const calls = [];
  const visit = (parent, ancestors) => {
    for (const script of scriptCalls(parent)) {
      assert.ok(existsSync(file(`scripts/${script}`)), `${parent} calls ${script}`);
      assert.ok(!ancestors.includes(script), `release composite cycle: ${[...ancestors, script].join(" -> ")}`);
      calls.push({ parent, script });
      visit(script, [...ancestors, script]);
    }
  };
  visit(hostReleaseGate, [hostReleaseGate]);
  return calls;
}

function scriptCommand(script) {
  return new RegExp(`scripts/${script.replaceAll(".", "\\.")}`);
}

function parseReleaseCoverage(value, id) {
  if (value === "root") return { root: true };
  const match = /^(\d+) \/ (\d+)$/.exec(value);
  assert.ok(match, `${id} release coverage must be root or DIRECT / NESTED`);
  return { direct: Number(match[1]), nested: Number(match[2]) };
}

function ownershipRows() {
  return tableRows(read(".github/verification-ownership.md"), "Suite ownership").map(
    ([id, command, owner, trigger, releaseCoverage, runtime]) => ({
      id,
      command,
      owner,
      trigger,
      releaseCoverage,
      runtime,
      script: command.match(/scripts\/(test-[a-z0-9-]+\.sh)/)?.[1],
    }),
  );
}

function autoMergeHarness() {
  return [
    "gh() {",
    '  if [[ "$1" == "api" && "$2" == "--method" && "$3" == "POST" &&',
    '        "$4" == repos/example/repo/statuses/* ]]; then',
    '    [[ "$*" == *"context=human approval"* ]] || {',
    '      echo "status context was not human approval" >&2',
    "      return 99",
    "    }",
    '    case "$*" in',
    '      *"state=pending"*) status=pending ;;',
    '      *"state=success"*) status=success ;;',
    '      *) echo "unexpected status state: $*" >&2; return 99 ;;',
    "    esac",
    '    [[ "${FAKE_STATUS_FAIL:-false}" == "true" ]] && return 1',
    '    printf "STATUS %s %s human approval\\n" "$status" "${4##*/}"',
    "    return 0",
    "  fi",
    '  case "$1:$2" in',
    '    "api:repos/example/repo/collaborators/writer/permission")',
    '      [[ "${FAKE_PERMISSION-write}" == "api-error" ]] && return 1',
    '      printf "%s\\n" "${FAKE_PERMISSION-write}"',
    "      ;;",
    '    "api:repos/example/repo") printf "%s\\n" "${FAKE_DEFAULT_BRANCH:-main}" ;;',
    '    "pr:view") printf "{}\\n" ;;',
    '    "pr:merge")',
    '      [[ "${FAKE_MERGE_FAIL:-false}" == "true" ]] && return 1',
    '      printf "MERGE %s\\n" "$*"',
    "      ;;",
    '    *) echo "unexpected gh command: $*" >&2; return 99 ;;',
    "  esac",
    "}",
    "jq() {",
    '  case "${2:-}" in',
    '    ".state") printf "%s\\n" "${FAKE_PR_STATE:-OPEN}" ;;',
    '    ".isDraft") printf "%s\\n" "${FAKE_PR_DRAFT:-false}" ;;',
    '    ".baseRefName") printf "%s\\n" "${FAKE_PR_BASE:-main}" ;;',
    '    ".headRefOid") printf "%s\\n" "${FAKE_HEAD_REF_OID:-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}" ;;',
    '    ".autoMergeRequest != null") printf "%s\\n" "${FAKE_AUTO_MERGE:-false}" ;;',
    '    *) echo "unexpected jq query: $*" >&2; return 99 ;;',
    "  esac",
    "}",
  ].join("\n");
}

function runAutoMergeStep(job, overrides = {}) {
  return spawnSync(
    "bash",
    ["-c", `${autoMergeHarness()}\n${workflowRunBlock(read(".github/workflows/auto-merge.yml"), job)}`],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        GITHUB_REPOSITORY: "example/repo",
        GH_TOKEN: "test-token",
        PR_NUMBER: "42",
        PR_HEAD_SHA: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        COMMENT_AUTHOR: "writer",
        ...overrides,
      },
    },
  );
}

test("verification ownership matrix discovers every test script and reconciles composite coverage", () => {
  const matrixPath = file(".github/verification-ownership.md");
  assert.ok(existsSync(matrixPath), "check in the verification ownership matrix");
  const ownerIds = new Set([
    "local focused",
    "workspace diagnosis",
    "CI lightweight",
    "CI Rust",
    "CI Console",
    "CI Edge",
    "CI trial",
    "CI aggregate",
    "trusted auto-merge",
    "release",
    "field/manual",
  ]);
  const ids = new Set();
  const scriptRows = new Map();
  const rows = ownershipRows();

  for (const row of rows) {
    assert.ok(
      row.id && row.command && row.owner && row.trigger && row.releaseCoverage && row.runtime,
      "every matrix row is complete",
    );
    assert.ok(!ids.has(row.id), `duplicate suite id: ${row.id}`);
    ids.add(row.id);
    assert.ok(ownerIds.has(row.owner), `${row.id} has no known owner`);
    if (!row.script) continue;

    assert.ok(existsSync(file(`scripts/${row.script}`)), `${row.id} names an existing script`);
    assert.ok(!scriptRows.has(row.script), `duplicate default owner for ${row.script}`);
    scriptRows.set(row.script, row);
  }

  assert.deepEqual(
    [...scriptRows.keys()].sort(),
    testScripts(),
    "every current test-*.sh script has one matrix owner, with no stale rows",
  );

  const workflow = ci();
  const releaseCalls = hostReleaseCalls();
  for (const parent of testScripts()) {
    for (const child of scriptCalls(parent)) {
      assert.ok(
        scriptRows.has(child),
        `${parent} calls ${child}, which must keep its canonical owner`,
      );
    }
  }
  for (const [script, row] of scriptRows) {
    const coverage = parseReleaseCoverage(row.releaseCoverage, row.id);
    const direct = releaseCalls.filter(
      (call) => call.parent === hostReleaseGate && call.script === script,
    ).length;
    const nested = releaseCalls.filter(
      (call) => call.parent !== hostReleaseGate && call.script === script,
    ).length;
    if (coverage.root) {
      assert.equal(script, hostReleaseGate, `${row.id} is the release root`);
      assert.equal(row.owner, "release", `${row.id} has the release root owner`);
      assert.equal(direct + nested, 0, `${row.id} is not recursively invoked`);
    } else {
      assert.equal(direct, coverage.direct, `${row.id} direct release coverage`);
      assert.equal(nested, coverage.nested, `${row.id} nested release coverage`);
    }

    const ciJob = ciOwnerJobs.get(row.owner);
    assert.deepEqual(
      jobsRunning(workflow, scriptCommand(script)),
      ciJob ? [ciJob] : [],
      `${row.id} has exactly its matrix-selected CI owner`,
    );
  }

  for (const [scope, before, after, runtimeAndOwner] of tableRows(
    read(".github/verification-ownership.md"),
    "Default command comparison",
  )) {
    assert.ok(scope && runtimeAndOwner);
    assert.match(before, /^\d+$/);
    assert.match(after, /^\d+$/);
  }
  for (const [diff, before, after, runtimeAndOwner] of tableRows(
    read(".github/verification-ownership.md"),
    "Representative diffs",
  )) {
    assert.ok(diff && runtimeAndOwner);
    assert.match(before, /^\d+$/);
    assert.match(after, /^\d+$/);
  }
});

test("representative diff routing reuses the fail-closed selector", () => {
  const routes = [
    ["docs", ["README.md"], { rust: false, console: false, edge: false, trial: false }],
    ["Rust crate", ["edge-node/apps/node/src/lib.rs"], { rust: true, console: false, edge: false, trial: false }],
    ["shared core", ["edge-node/core/ledger/src/lib.rs"], { rust: true, console: false, edge: false, trial: false, packages: "all" }],
    ["Console", ["edge/frontend/src/app.ts"], { rust: true, console: true, edge: false, trial: false }],
    ["custody", ["edge/src/storage/mod.rs"], { rust: true, console: false, edge: true, trial: false }],
    ["trial", ["scripts/iotkit_trial.py"], { rust: false, console: false, edge: false, trial: true }],
    ["unknown", ["new-component/file.txt"], { rust: true, console: true, edge: true, trial: true, packages: "all" }],
    ["CI infrastructure", [".github/workflows/new-check.yml"], { rust: true, console: true, edge: true, trial: true, packages: "all" }],
  ];

  for (const [name, paths, expected] of routes) {
    const actual = selectCiJobs(paths);
    for (const [key, value] of Object.entries(expected)) {
      assert.equal(actual[key], value, name);
    }
  }
});

test("workspace diagnosis is explicit and no longer contains an integration bucket", () => {
  const verify = read("scripts/verify.sh");
  assert.match(verify, /usage: scripts\/verify\.sh --workspace/);
  assert.doesNotMatch(verify, /--full/);
  for (const script of testScripts()) {
    assert.doesNotMatch(verify, scriptCommand(script), script);
  }

  for (const args of [[], ["--full"], ["--workspace", "extra"]]) {
    const result = spawnSync(
      "bash",
      [file("scripts/verify.sh").pathname, ...args],
      { encoding: "utf8" },
    );
    assert.equal(result.status, 2, result.stderr);
  }
});

test("current guidance keeps workspace diagnosis explicit", () => {
  for (const path of [
    ".agents/change-map.md",
    ".agents/commands.md",
    ".agents/review-and-verification.md",
    ".agents/workflow.md",
    ".codex/README.md",
    "README.md",
    "README.ja.md",
    "CONTRIBUTING.md",
    "CONTRIBUTING.ja.md",
    "RELEASING.md",
  ]) {
    const source = read(path);
    assert.match(source, /scripts\/verify\.sh --workspace/, path);
    assert.doesNotMatch(source, /scripts\/verify\.sh(?! --workspace)/, path);
  }
  for (const path of [
    "README.md",
    "README.ja.md",
    "CONTRIBUTING.md",
    "CONTRIBUTING.ja.md",
    "RELEASING.md",
  ]) {
    assert.match(read(path), /verification-ownership\.md/, path);
  }
});

test("CI uses mise for managed tools", () => {
  const workflow = ci();
  for (const path of ["CONTRIBUTING.md", "CONTRIBUTING.ja.md"]) {
    const guidance = read(path);
    assert.match(
      guidance,
      /\(https:\/\/mise\.jdx\.dev\/getting-started\.html\)/,
      `${path} links the official mise guide`,
    );
    assert.match(
      guidance,
      /shell activation[\s\S]*shims[\s\S]*mise install[\s\S]*node --version[\s\S]*cargo --version[\s\S]*npm --version/,
      `${path} configures shell activation before direct tool checks`,
    );
  }
  for (const job of ["changes", "lightweight", "check", "console", "edge", "trial"]) {
    assert.match(jobSection(workflow, job), /jdx\/mise-action@v4/, `${job} uses mise`);
  }
  const changes = jobSection(workflow, "changes");
  assert.match(changes, /^\s+install_args:\s+node\s*$/m, "changes installs only node");

  const console = jobSection(workflow, "console");
  assert.match(console, /uses:\s+actions\/cache@v6/, "console uses the native cache action");
  assert.match(console, /^\s+path:\s+~\/\.npm\s*$/m, "console caches npm");
  assert.match(
    console,
    /^\s+key:\s+[^\n]*\$\{\{\s*runner\.os\s*\}\}[^\n]*hashFiles\(\s*['"]edge\/frontend\/package-lock\.json['"]\s*\)/m,
    "console cache key includes runner OS and lockfile hash",
  );
  assert.doesNotMatch(
    workflow,
    /actions\/setup-(?:node|python)|taiki-e\/install-action|cargo install[^\n]*(?:cargo-nextest|nextest)|apt-get install[^\n]*(?:jq|sqlite3?)/,
  );
});

test("required CI derives every selected lane from the selector and executes its real guard", () => {
  const workflow = ci();
  const aggregate = jobSection(workflow, "required-ci");
  const changes = jobSection(workflow, "changes");
  const lanes = selectorLanes();

  assert.deepEqual(
    lanes.filter(({ output, job }) => output !== job).map(({ output, job }) => ({ output, job })),
    [{ output: "rust", job: "check" }],
    "rust is the only selector-to-job rename",
  );
  assert.match(aggregate, /name: required CI/);
  assert.match(aggregate, /if: \$\{\{ always\(\) \}\}/);
  const requiredNeeds = aggregate.match(/needs: \[([^\]]+)\]/)?.[1]
    .split(",")
    .map((value) => value.trim());
  assert.deepEqual(
    requiredNeeds,
    ["changes", "lightweight", ...lanes.map(({ job }) => job)],
    "required CI needs every selector lane exactly once",
  );
  assert.match(workflow, /node scripts\/select-ci-jobs\.mjs <\/dev\/null/);
  assert.match(aggregate, /CHANGES_RESULT: \$\{\{ needs\.changes\.result \}\}/);
  assert.match(aggregate, /LIGHTWEIGHT_RESULT: \$\{\{ needs\.lightweight\.result \}\}/);
  assert.match(aggregateRunBlock(workflow), /\[\[ "\$CHANGES_RESULT" == "success" \]\]/);
  assert.match(aggregateRunBlock(workflow), /\[\[ "\$LIGHTWEIGHT_RESULT" == "success" \]\]/);
  assert.doesNotMatch(aggregateRunBlock(workflow), /PACKAGES|packages/);

  for (const lane of lanes) {
    assert.match(
      changes,
      new RegExp(
        `\\n      ${lane.output}: \\$\\{\\{ steps\\.select\\.outputs\\.${lane.output} \\}\\}`,
      ),
      `${lane.output} is an exposed selector output`,
    );
    assert.match(
      jobSection(workflow, lane.job),
      new RegExp(`if: needs\\.changes\\.outputs\\.${lane.output} == 'true'`),
      `${lane.output} selects ${lane.job}`,
    );
    assert.match(
      aggregate,
      new RegExp(
        `${lane.selectedEnv}: \\$\\{\\{ needs\\.changes\\.outputs\\.${lane.output} \\}\\}`,
      ),
    );
    assert.match(
      aggregate,
      new RegExp(
        `${lane.resultEnv}: \\$\\{\\{ needs\\.${lane.job}\\.result \\}\\}`,
      ),
    );
    assert.match(
      aggregateRunBlock(workflow),
      new RegExp(
        `check_lane ${lane.output} "\\$${lane.selectedEnv}" "\\$${lane.resultEnv}"`,
      ),
    );
  }

  for (const lane of lanes) {
    assert.equal(
      runRequiredCi({ [lane.selectedEnv]: "true", [lane.resultEnv]: "success" }).status,
      0,
      `${lane.output}: selected successful lane passes`,
    );
    for (const result of ["failure", "cancelled", "skipped", "", "malformed"]) {
      assert.notEqual(
        runRequiredCi({ [lane.selectedEnv]: "true", [lane.resultEnv]: result }).status,
        0,
        `${lane.output}: selected ${result || "empty"} lane fails`,
      );
    }

    assert.equal(
      runRequiredCi({ [lane.selectedEnv]: "false", [lane.resultEnv]: "skipped" }).status,
      0,
      `${lane.output}: unselected skipped lane passes`,
    );
    for (const result of ["success", "failure", "cancelled", "", "malformed"]) {
      assert.notEqual(
        runRequiredCi({ [lane.selectedEnv]: "false", [lane.resultEnv]: result }).status,
        0,
        `${lane.output}: unselected ${result || "empty"} lane fails`,
      );
    }
    for (const selected of ["", "malformed"]) {
      assert.notEqual(
        runRequiredCi({ [lane.selectedEnv]: selected }).status,
        0,
        `${lane.output}: ${selected || "empty"} selector fails closed`,
      );
    }
  }

  for (const mandatory of ["CHANGES_RESULT", "LIGHTWEIGHT_RESULT"]) {
    assert.equal(runRequiredCi({ [mandatory]: "success" }).status, 0, mandatory);
    for (const result of ["failure", "cancelled", "skipped", "", "malformed"]) {
      assert.notEqual(
        runRequiredCi({ [mandatory]: result }).status,
        0,
        `${mandatory}: ${result || "empty"} mandatory result fails`,
      );
    }
  }
});

test("trusted auto-merge records per-head human approval and clears stale approval", () => {
  const path = ".github/workflows/auto-merge.yml";
  assert.ok(existsSync(file(path)), "check in the trusted auto-merge workflow");
  const workflow = read(path);
  const arm = jobSection(workflow, "arm");
  const reset = jobSection(workflow, "reset");

  assert.match(workflow, /issue_comment:\n\s+types: \[created\]/);
  assert.match(
    workflow,
    /pull_request_target:\n\s+types: \[opened, reopened, ready_for_review, synchronize\]/,
  );
  assert.match(
    workflow.split("\njobs:\n")[0],
    /concurrency:\n\s+group: guarded-auto-merge-\$\{\{ github\.repository \}\}-\$\{\{ github\.event\.issue\.number \|\| github\.event\.pull_request\.number \}\}\n\s+cancel-in-progress: true/,
    "comment arm and synchronize disarm serialize by repository and PR number",
  );
  assert.doesNotMatch(workflow, /\npull_request:/);
  assert.doesNotMatch(workflow, /actions\/checkout/);
  assert.doesNotMatch(workflow, /\n\s+uses:/);
  assert.doesNotMatch(
    workflow,
    /github\.event\.pull_request\.head\.(?:ref|label|repo)|github\.head_ref/,
  );
  assert.doesNotMatch(workflow.split("\njobs:\n")[0], /\npermissions:/);

  for (const section of [arm, reset]) {
    assert.equal(
      section.match(/    permissions:\n([\s\S]*?)\n    runs-on:/)?.[1].trim(),
      "contents: write\n      pull-requests: write\n      statuses: write",
      "only merge and per-head status job-scoped permissions are granted",
    );
  }

  assert.match(arm, /github\.event_name == 'issue_comment'/);
  assert.match(arm, /github\.event\.issue\.pull_request != null/);
  assert.match(arm, /github\.event\.comment\.user\.type == 'User'/);
  assert.match(arm, /github\.event\.comment\.body == '\/auto-merge'/);
  assert.deepEqual(
    [...arm.matchAll(/author_association == '([^']+)'/g)].map(
      ([, association]) => association,
    ),
    ["OWNER", "MEMBER", "COLLABORATOR"],
  );
  assert.match(arm, /COMMENT_AUTHOR: \$\{\{ github\.event\.comment\.user\.login \}\}/);
  assert.match(arm, /collaborators\/\$COMMENT_AUTHOR\/permission/);
  assert.match(arm, /admin\|maintain\|write/);
  assert.match(arm, /gh pr view[\s\S]*--json state,isDraft,baseRefName,headRefOid/);
  assert.match(arm, /gh api "repos\/\$GITHUB_REPOSITORY" --jq '\.default_branch'/);
  assert.match(arm, /\$state" != "OPEN"/);
  assert.match(arm, /\$draft" != "false"/);
  assert.match(arm, /\$base" != "\$default_branch"/);
  assert.match(arm, /head_ref_oid="\$\(jq -r '\.headRefOid'/);
  assert.match(arm, /statuses\/\$head_ref_oid/);
  assert.match(arm, /-f state=success/);
  assert.match(arm, /-f context='human approval'/);
  assert.match(arm, /gh pr merge "\$PR_NUMBER" --repo "\$GITHUB_REPOSITORY" --auto --squash/);
  const armStatus = arm.indexOf("statuses/$head_ref_oid");
  const armMerge = arm.indexOf('gh pr merge "$PR_NUMBER"');
  for (const guard of [
    "admin|maintain|write",
    "$state\" != \"OPEN\"",
    "$draft\" != \"false\"",
    "$base\" != \"$default_branch\"",
  ]) {
    assert.ok(arm.indexOf(guard) < armStatus, `${guard} guards approval status`);
  }
  assert.ok(armStatus < armMerge, "per-head success precedes native auto-merge");
  assert.equal((arm.match(/gh pr merge/g) ?? []).length, 1);

  const approvedHead = "a".repeat(40);
  for (const permission of ["admin", "maintain", "write"]) {
    const result = runAutoMergeStep("arm", {
      FAKE_PERMISSION: permission,
      FAKE_HEAD_REF_OID: approvedHead,
    });
    assert.equal(result.status, 0, result.stderr);
    assert.match(
      result.stdout,
      new RegExp(`STATUS success ${approvedHead} human approval`),
    );
    assert.match(result.stdout, /MERGE/);
  }
  for (const permission of ["read", "triage", "unknown", "", "api-error"]) {
    const result = runAutoMergeStep("arm", { FAKE_PERMISSION: permission });
    assert.equal(result.status, 0, result.stderr);
    assert.doesNotMatch(result.stdout, /STATUS success/);
    assert.doesNotMatch(result.stdout, /MERGE/, `${permission || "empty"} fails closed`);
  }
  for (const [name, overrides] of [
    ["closed", { FAKE_PR_STATE: "CLOSED" }],
    ["draft", { FAKE_PR_DRAFT: "true" }],
    ["non-default base", { FAKE_PR_BASE: "release" }],
  ]) {
    const result = runAutoMergeStep("arm", overrides);
    assert.equal(result.status, 0, result.stderr);
    assert.doesNotMatch(result.stdout, /STATUS success/);
    assert.doesNotMatch(result.stdout, /MERGE/, `${name} PR does not arm`);
  }
  const statusFailure = runAutoMergeStep("arm", { FAKE_STATUS_FAIL: "true" });
  assert.notEqual(statusFailure.status, 0, statusFailure.stderr);
  assert.doesNotMatch(statusFailure.stdout, /MERGE/);
  const invalidArmHead = runAutoMergeStep("arm", { FAKE_HEAD_REF_OID: "invalid" });
  assert.notEqual(invalidArmHead.status, 0, invalidArmHead.stderr);
  assert.doesNotMatch(invalidArmHead.stdout, /STATUS success|MERGE/);

  assert.match(reset, /github\.event_name == 'pull_request_target'/);
  assert.match(
    reset,
    /github\.event\.pull_request\.base\.ref == github\.event\.repository\.default_branch/,
  );
  assert.match(reset, /PR_HEAD_SHA: \$\{\{ github\.event\.pull_request\.head\.sha \}\}/);
  assert.match(reset, /statuses\/\$PR_HEAD_SHA/);
  assert.match(reset, /-f state=pending/);
  assert.match(reset, /-f context='human approval'/);
  assert.match(reset, /gh pr view[\s\S]*--json autoMergeRequest/);
  assert.match(reset, /\.autoMergeRequest != null/);
  assert.match(reset, /gh pr merge "\$PR_NUMBER" --repo "\$GITHUB_REPOSITORY" --disable-auto/);
  const resetStatus = reset.indexOf("statuses/$PR_HEAD_SHA");
  const resetMerge = reset.indexOf('gh pr merge "$PR_NUMBER"');
  assert.ok(
    resetStatus < resetMerge,
    "pending status is posted before any native auto-merge disarm",
  );
  assert.equal((reset.match(/gh pr merge/g) ?? []).length, 1);

  const newHead = "b".repeat(40);
  const resetUnarmed = runAutoMergeStep("reset", {
    PR_HEAD_SHA: newHead,
    FAKE_AUTO_MERGE: "false",
  });
  assert.equal(resetUnarmed.status, 0, resetUnarmed.stderr);
  assert.match(
    resetUnarmed.stdout,
    new RegExp(`STATUS pending ${newHead} human approval`),
  );
  assert.doesNotMatch(resetUnarmed.stdout, /STATUS success/);
  assert.doesNotMatch(resetUnarmed.stdout, new RegExp(approvedHead));
  assert.doesNotMatch(resetUnarmed.stdout, /MERGE/);

  const resetArmed = runAutoMergeStep("reset", {
    PR_HEAD_SHA: newHead,
    FAKE_AUTO_MERGE: "true",
  });
  assert.equal(resetArmed.status, 0, resetArmed.stderr);
  assert.match(
    resetArmed.stdout,
    new RegExp(`STATUS pending ${newHead} human approval`),
  );
  assert.match(resetArmed.stdout, /MERGE/);

  const failedDisarm = runAutoMergeStep("reset", {
    PR_HEAD_SHA: newHead,
    FAKE_AUTO_MERGE: "true",
    FAKE_MERGE_FAIL: "true",
  });
  assert.notEqual(failedDisarm.status, 0, failedDisarm.stderr);
  assert.match(
    failedDisarm.stdout,
    new RegExp(`STATUS pending ${newHead} human approval`),
    "a failed disarm leaves the new head blocked",
  );
  const failedReset = runAutoMergeStep("reset", {
    PR_HEAD_SHA: newHead,
    FAKE_AUTO_MERGE: "true",
    FAKE_STATUS_FAIL: "true",
  });
  assert.notEqual(failedReset.status, 0, failedReset.stderr);
  assert.doesNotMatch(failedReset.stdout, /MERGE/);
  const invalidResetHead = runAutoMergeStep("reset", { PR_HEAD_SHA: "invalid" });
  assert.notEqual(invalidResetHead.status, 0, invalidResetHead.stderr);
  assert.doesNotMatch(invalidResetHead.stdout, /STATUS|MERGE/);

  for (const path of [".agents/workflow.md", "CONTRIBUTING.md"]) {
    assert.match(
      read(path),
      /New commits disarm\s+it(?:,| and) reset[\s\S]*new\s+exact comment/,
      `${path} requires fresh approval after a new commit`,
    );
    assert.match(
      read(path),
      /human `User` account[\s\S]*`OWNER`, `MEMBER`, or `COLLABORATOR`[\s\S]*`admin`, `maintain`, or `write`/,
      `${path} requires a human, trusted association, and effective write permission`,
    );
    assert.match(
      read(path),
      /`required CI`, `human approval`, and\s+CodeQL/,
      `${path} records the branch-protection requirement`,
    );
  }
  assert.match(
    read("CONTRIBUTING.ja.md"),
    /新しいcommitは[\s\S]*もう一度完全一致のcomment/,
    "Japanese contributor guidance requires fresh approval after a new commit",
  );
  assert.match(
    read("CONTRIBUTING.ja.md"),
    /`User`である人間のアカウント[\s\S]*`OWNER`、[\s\S]*`MEMBER`、`COLLABORATOR`[\s\S]*`admin`、[\s\S]*`maintain`、`write`/,
    "Japanese contributor guidance requires a human, trusted association, and effective write permission",
  );
  assert.match(
    read("CONTRIBUTING.ja.md"),
    /`required CI`、`human approval`、CodeQL/,
    "Japanese contributor guidance records the branch-protection requirement",
  );
  assert.match(
    read(".github/verification-ownership.md"),
    /`required CI`, `human approval`, and\s+CodeQL/,
    "ownership matrix records the branch-protection requirement",
  );
});
