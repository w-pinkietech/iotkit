import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { selectCiJobs, selectRustPackages } from "../select-ci-jobs.mjs";

const none = {
  rust: false,
  console: false,
  edge: false,
  trial: false,
  packages: "",
};
const rustOnly = (packages) => ({
  rust: true,
  console: false,
  edge: false,
  trial: false,
  packages,
});
const rustTrial = (packages) => ({
  rust: true,
  console: false,
  edge: false,
  trial: true,
  packages,
});
const rustConsole = (packages) => ({
  rust: true,
  console: true,
  edge: false,
  trial: false,
  packages,
});
const rustConsoleTrial = (packages) => ({
  rust: true,
  console: true,
  edge: false,
  trial: true,
  packages,
});
const rustEdge = (packages) => ({
  rust: true,
  console: false,
  edge: true,
  trial: false,
  packages,
});
const rustEdgeTrial = (packages) => ({
  rust: true,
  console: false,
  edge: true,
  trial: true,
  packages,
});
const rustConsoleEdge = (packages) => ({
  rust: true,
  console: true,
  edge: true,
  trial: false,
  packages,
});
const allHeavy = (packages = "all") => ({
  rust: true,
  console: true,
  edge: true,
  trial: true,
  packages,
});
const trialOnly = {
  rust: false,
  console: false,
  edge: false,
  trial: true,
  packages: "",
};

const cases = [
  {
    name: "documentation and repository guidance use lightweight checks only",
    paths: [
      "docs/product/en/index.md",
      "AGENTS.md",
      "CONTRIBUTING.ja.md",
      "scripts/product-docs-impact.mjs",
      "scripts/docs/product-docs-impact-rules.json",
      "scripts/tests/adapter-author-docs.test.mjs",
      "scripts/tests/check-product-docs.test.mjs",
      "scripts/tests/product-docs-modes.test.mjs",
      "scripts/tests/product-docs-impact.test.mjs",
      "scripts/pure-refactoring-evaluator.mjs",
      "scripts/tests/pure-refactoring-evaluator.test.mjs",
    ],
    expected: none,
  },
  {
    name: "shared core ledger expands beyond the focus threshold to full suite",
    paths: ["edge-node/core/ledger/src/lib.rs"],
    expected: rustOnly("all"),
  },
  {
    name: "workspace dependency changes select Rust, Console, Edge, and trial",
    paths: ["Cargo.lock"],
    expected: allHeavy("all"),
  },
  {
    name: "mise toolchain changes select all heavy jobs",
    paths: ["mise.toml"],
    expected: allHeavy("all"),
  },
  {
    name: "Rust IoTKit Edge storage selects Edge integration without Console",
    paths: ["edge/src/storage/mod.rs"],
    expected: rustEdge("iotkit-edge"),
  },
  {
    name: "Edge Node signal composition selects the PID1 SIGTERM integration gate",
    paths: ["edge-node/apps/node/src/main.rs"],
    expected: rustEdge("iotkit-edge-node"),
  },
  {
    name: "Output Adapter example focuses example package and Edge integration",
    paths: ["edge/output-adapters/example/src/lib.rs"],
    expected: rustEdge("iotkit-output-adapter-example"),
  },
  {
    name: "Edge manifest selects both Console and Edge integration",
    paths: ["edge/Cargo.toml"],
    expected: rustConsoleEdge("iotkit-edge"),
  },
  {
    name: "Console e2e script selects Console lane without custody/output",
    paths: ["scripts/test-edge-console-e2e.sh"],
    expected: rustConsole("all"),
  },
  {
    name: "frontend-only paths select Console lane without Edge integration or trial",
    paths: [
      "edge/frontend/static/app.js",
      "edge/frontend/package.json",
    ],
    expected: rustConsole("iotkit-edge"),
  },
  {
    name: "browser composition adapter selects Console lane",
    paths: ["edge/src/composition/web.rs"],
    expected: rustConsole("iotkit-edge"),
  },
  {
    name: "Askama template configuration selects Console lane",
    paths: ["edge/askama.toml"],
    expected: rustConsole("iotkit-edge"),
  },
  {
    name: "Console fixture examples select Console lane",
    paths: ["edge/examples/console_commissioning_fixture.rs"],
    expected: rustConsole("iotkit-edge"),
  },
  {
    name: "presentation template/CSS stay on Console without trial Docker",
    paths: [
      "edge/src/web/templates/console.html",
      "edge/frontend/static/edge.css",
    ],
    expected: rustConsole("iotkit-edge"),
  },
  {
    name: "Edge script family outside console scripts selects both product lanes",
    paths: [
      "scripts/test-edge-resilience.sh",
      "scripts/test-edge-bootstrap.sh",
      "scripts/test-edge-mqtt.sh",
      "scripts/test-edge-parity.sh",
      "scripts/test-edge-node-fence.sh",
      "scripts/test-edge-node-sigterm.sh",
      "scripts/test-edge-host-release-gate.sh",
    ],
    expected: rustConsoleEdge("all"),
  },
  {
    name: "custody script selects Edge integration only",
    paths: ["scripts/test-rust-edge-custody.sh"],
    expected: rustEdge("all"),
  },
  {
    name: "trial-related Edge runtime/config selects Edge integration and trial",
    paths: ["edge/src/composition/runtime_config.rs"],
    expected: rustEdgeTrial("iotkit-edge"),
  },
  {
    name: "trial-related console_contract selects Console and trial without custody",
    paths: ["edge/tests/console_contract.rs"],
    expected: rustConsoleTrial("iotkit-edge"),
  },
  {
    name: "shared contract fixtures select all heavy jobs",
    paths: ["testdata/egress/v1/record-batch.json"],
    expected: allHeavy("all"),
  },
  {
    name: "workflow changes select all heavy jobs",
    paths: [".github/workflows/ci.yml"],
    expected: allHeavy("all"),
  },
  {
    name: "classifier changes select all heavy jobs",
    paths: ["scripts/select-ci-jobs.mjs"],
    expected: allHeavy("all"),
  },
  {
    name: "unknown paths select all heavy jobs",
    paths: ["new-component/file.txt"],
    expected: allHeavy("all"),
  },
  {
    name: "empty input selects all heavy jobs",
    paths: [],
    expected: allHeavy("all"),
  },
  {
    name: "trial launcher Python selects trial Docker only",
    paths: ["scripts/iotkit_trial.py"],
    expected: trialOnly,
  },
  {
    name: "trial unit tests stay on lightweight only",
    paths: ["scripts/tests/test_iotkit_trial.py"],
    expected: none,
  },
  {
    name: "trial journey script selects trial Docker only",
    paths: ["scripts/test-iotkit-trial.sh"],
    expected: trialOnly,
  },
  {
    name: "trial compose overlay selects trial Docker only",
    paths: ["deploy/compose.trial.yaml"],
    expected: trialOnly,
  },
  {
    name: "root trial TOML selects trial Docker only",
    paths: ["iotkit.toml"],
    expected: trialOnly,
  },
  {
    name: "trial-sample adapter focuses adapter + Edge Node factory tests",
    paths: ["edge-node/adapters/trial-sample/src/lib.rs"],
    expected: rustTrial("iotkit-edge-node,trial-sample-adapter"),
  },
  {
    name: "Edge Dockerfile rebuilds Console, Edge, and trial images",
    paths: ["edge/Dockerfile"],
    expected: allHeavy("all"),
  },
  {
    name: "field deploy compose selects Console and Edge without trial",
    paths: ["deploy/compose.edge.yaml"],
    expected: rustConsoleEdge("all"),
  },
  {
    name: "iotkit trial CLI wrapper selects trial Docker only",
    paths: ["scripts/iotkit"],
    expected: trialOnly,
  },
  {
    name: "recovery package focuses recovery + node/nodectl consumers",
    paths: ["edge-node/core/recovery/src/lib.rs"],
    expected: rustOnly(
      "iotkit-core-recovery,iotkit-edge-node,iotkit-edge-nodectl",
    ),
  },
];

for (const { name, paths, expected } of cases) {
  test(name, () => {
    assert.deepEqual(selectCiJobs(paths), expected);
  });
}

test("selectRustPackages keeps trial-sample narrowly focused", () => {
  assert.equal(
    selectRustPackages(["edge-node/adapters/trial-sample/src/lib.rs"]),
    "iotkit-edge-node,trial-sample-adapter",
  );
});

test("unlisted nested crate under edge/ forces the full suite", () => {
  assert.equal(
    selectRustPackages(["edge/output-adapters/acme/Cargo.toml"]),
    "all",
  );
  assert.equal(
    selectRustPackages(["edge/output-adapters/acme/src/lib.rs"]),
    "all",
  );
});

test("listed edge sources still focus iotkit-edge only", () => {
  assert.equal(selectRustPackages(["edge/src/storage/mod.rs"]), "iotkit-edge");
  assert.equal(selectRustPackages(["edge/Cargo.toml"]), "iotkit-edge");
});

test("CI workflow routes heavy jobs through the classifier", () => {
  const workflow = readFileSync(
    new URL("../../.github/workflows/ci.yml", import.meta.url),
    "utf8",
  );

  assert.match(workflow, /id: select/);
  assert.match(workflow, /node scripts\/select-ci-jobs\.mjs/);
  assert.match(workflow, /needs\.changes\.outputs\.rust == 'true'/);
  assert.match(workflow, /needs\.changes\.outputs\.console == 'true'/);
  assert.match(workflow, /needs\.changes\.outputs\.edge == 'true'/);
  assert.match(workflow, /needs\.changes\.outputs\.trial == 'true'/);
  assert.match(workflow, /needs\.changes\.outputs\.packages/);
  assert.match(workflow, /name: lightweight repository checks/);
  assert.match(workflow, /name: Console frontend and browser journey/);
  assert.match(workflow, /name: Edge custody and output integration/);
  assert.match(
    workflow,
    /name: Trial profile first-run and custody journey/,
  );
  assert.match(
    workflow,
    /node --test scripts\/tests\/adapter-author-docs\.test\.mjs/,
  );
  assert.match(
    workflow,
    /node --test scripts\/tests\/check-product-docs\.test\.mjs/,
  );
  assert.match(workflow, /node scripts\/product-docs-impact\.mjs check/);
  assert.match(
    workflow,
    /node --test scripts\/tests\/product-docs-impact\.test\.mjs/,
  );
  assert.match(workflow, /node scripts\/product-docs-impact\.mjs soft-check/);
  assert.match(workflow, /Product-docs freshness soft gate/);
  assert.match(
    workflow,
    /node --test scripts\/tests\/rust-edge-release-gates\.test\.mjs/,
  );
  // Focused package selection drives clippy/nextest when not "all".
  assert.match(workflow, /cargo nextest run/);
  assert.match(workflow, /PACKAGES/);
  assert.doesNotMatch(
    workflow,
    /cargo build --workspace --all-targets/,
    "redundant workspace build should stay out of the rust job",
  );

  const consoleJob =
    workflow.split(/name: Console frontend and browser journey/)[1] ?? "";
  const consoleSection = consoleJob.split(/^  [a-z]/m)[0] ?? consoleJob;
  assert.match(consoleSection, /scripts\/test-edge-console-frontend\.sh/);
  assert.match(consoleSection, /scripts\/test-edge-console-e2e\.sh/);
  assert.doesNotMatch(
    consoleSection,
    /scripts\/test-rust-edge-custody\.sh/,
    "custody must not run in the Console job",
  );
  assert.doesNotMatch(
    consoleSection,
    /scripts\/test-edge-output\.sh/,
    "output delivery must not run in the Console job",
  );
  assert.doesNotMatch(
    consoleSection,
    /scripts\/test-iotkit-trial\.sh/,
    "trial journey should live in the dedicated trial job",
  );

  const edgeJob =
    workflow.split(/name: Edge custody and output integration/)[1] ?? "";
  const edgeSection = edgeJob.split(/^  [a-z]/m)[0] ?? edgeJob;
  assert.match(edgeSection, /scripts\/test-rust-edge-custody\.sh/);
  assert.match(edgeSection, /scripts\/test-edge-output\.sh/);
  assert.match(edgeSection, /scripts\/test-edge-node-sigterm\.sh/);
  assert.doesNotMatch(
    edgeSection,
    /scripts\/test-edge-console-e2e\.sh/,
    "Console e2e must not run in the Edge integration job",
  );
  assert.doesNotMatch(
    edgeSection,
    /scripts\/test-iotkit-trial\.sh/,
    "trial journey should live in the dedicated trial job",
  );
  assert.doesNotMatch(
    edgeSection,
    /cargo test -p iotkit-edge/,
    "iotkit-edge unit/integration should not be duplicated in the edge job",
  );
});

test("CLI reads changed paths from standard input", () => {
  const script = fileURLToPath(
    new URL("../select-ci-jobs.mjs", import.meta.url),
  );
  const result = spawnSync(process.execPath, [script], {
    input: "docs/README.md\n",
    encoding: "utf8",
  });

  assert.equal(result.status, 0, result.stderr);
  assert.equal(
    result.stdout,
    "rust=false\nconsole=false\nedge=false\ntrial=false\npackages=\n",
  );
});

test("CLI reports trial-only selection for the launcher", () => {
  const script = fileURLToPath(
    new URL("../select-ci-jobs.mjs", import.meta.url),
  );
  const result = spawnSync(process.execPath, [script], {
    input: "scripts/iotkit_trial.py\n",
    encoding: "utf8",
  });

  assert.equal(result.status, 0, result.stderr);
  assert.equal(
    result.stdout,
    "rust=false\nconsole=false\nedge=false\ntrial=true\npackages=\n",
  );
});

test("CLI reports focused packages for trial-sample", () => {
  const script = fileURLToPath(
    new URL("../select-ci-jobs.mjs", import.meta.url),
  );
  const result = spawnSync(process.execPath, [script], {
    input: "edge-node/adapters/trial-sample/src/lib.rs\n",
    encoding: "utf8",
  });

  assert.equal(result.status, 0, result.stderr);
  assert.equal(
    result.stdout,
    "rust=true\nconsole=false\nedge=false\ntrial=true\npackages=iotkit-edge-node,trial-sample-adapter\n",
  );
});

test("CLI reports Console-only selection for frontend paths", () => {
  const script = fileURLToPath(
    new URL("../select-ci-jobs.mjs", import.meta.url),
  );
  const result = spawnSync(process.execPath, [script], {
    input: "edge/frontend/static/edge.css\n",
    encoding: "utf8",
  });

  assert.equal(result.status, 0, result.stderr);
  assert.equal(
    result.stdout,
    "rust=true\nconsole=true\nedge=false\ntrial=false\npackages=iotkit-edge\n",
  );
});
