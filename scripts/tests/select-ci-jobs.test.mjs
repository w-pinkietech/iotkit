import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { selectCiJobs } from "../select-ci-jobs.mjs";

const none = { rust: false, edge: false, trial: false };
const rustOnly = { rust: true, edge: false, trial: false };
const rustTrial = { rust: true, edge: false, trial: true };
const rustEdge = { rust: true, edge: true, trial: false };
const rustEdgeTrial = { rust: true, edge: true, trial: true };
const trialOnly = { rust: false, edge: false, trial: true };

const cases = [
  {
    name: "documentation and repository guidance use lightweight checks only",
    paths: [
      "docs/okf/en/index.md",
      "AGENTS.md",
      "CONTRIBUTING.ja.md",
      "scripts/tests/adapter-author-docs.test.mjs",
    ],
    expected: none,
  },
  {
    name: "Edge Node-only changes select only Rust",
    paths: ["edge-node/core/ledger/src/lib.rs"],
    expected: rustOnly,
  },
  {
    name: "workspace dependency changes select Rust, Edge, and trial image rebuild",
    paths: ["Cargo.lock"],
    expected: rustEdgeTrial,
  },
  {
    name: "Rust IoTKit Edge changes select Rust and Edge integration",
    paths: ["edge/src/storage/mod.rs"],
    expected: rustEdge,
  },
  {
    name: "Output Adapter changes select Rust and Edge integration",
    paths: ["edge/output-adapters/example/src/lib.rs"],
    expected: rustEdge,
  },
  {
    name: "IoTKit Edge verification scripts select Rust and integration",
    paths: ["scripts/test-edge-console-e2e.sh"],
    expected: rustEdge,
  },
  {
    name: "Edge script family outside the old allowlist still selects Rust+Edge only",
    paths: [
      "scripts/test-edge-resilience.sh",
      "scripts/test-edge-bootstrap.sh",
      "scripts/test-edge-mqtt.sh",
      "scripts/test-edge-parity.sh",
      "scripts/test-edge-node-fence.sh",
      "scripts/test-edge-host-release-gate.sh",
    ],
    expected: rustEdge,
  },
  {
    name: "trial-related Edge runtime/config selects Rust, Edge, and trial Docker",
    paths: ["edge/src/composition/runtime_config.rs"],
    expected: rustEdgeTrial,
  },
  {
    name: "trial banner template/CSS select Rust, Edge, and trial Docker",
    paths: [
      "edge/src/web/templates/console.html",
      "edge/frontend/static/edge.css",
    ],
    expected: rustEdgeTrial,
  },

  {
    name: "shared contract fixtures select Rust, Edge, and trial",
    paths: ["testdata/egress/v1/record-batch.json"],
    expected: rustEdgeTrial,
  },
  {
    name: "workflow changes select all heavy jobs",
    paths: [".github/workflows/ci.yml"],
    expected: rustEdgeTrial,
  },
  {
    name: "classifier changes select all heavy jobs",
    paths: ["scripts/select-ci-jobs.mjs"],
    expected: rustEdgeTrial,
  },
  {
    name: "unknown paths select all heavy jobs",
    paths: ["new-component/file.txt"],
    expected: rustEdgeTrial,
  },
  {
    name: "empty input selects all heavy jobs",
    paths: [],
    expected: rustEdgeTrial,
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
    name: "trial-sample adapter selects Rust and trial Docker",
    paths: ["edge-node/adapters/trial-sample/src/lib.rs"],
    expected: rustTrial,
  },
  {
    name: "Edge Dockerfile rebuilds Edge and trial images",
    paths: ["edge/Dockerfile"],
    expected: rustEdgeTrial,
  },
  {
    name: "field deploy compose selects Rust and Edge without trial",
    paths: ["deploy/compose.edge.yaml"],
    expected: rustEdge,
  },
  {
    name: "iotkit trial CLI wrapper selects trial Docker only",
    paths: ["scripts/iotkit"],
    expected: trialOnly,
  },
];

for (const { name, paths, expected } of cases) {
  test(name, () => {
    assert.deepEqual(selectCiJobs(paths), expected);
  });
}

test("CI workflow routes heavy jobs through the classifier", () => {
  const workflow = readFileSync(
    new URL("../../.github/workflows/ci.yml", import.meta.url),
    "utf8",
  );

  assert.match(workflow, /id: select/);
  assert.match(workflow, /node scripts\/select-ci-jobs\.mjs/);
  assert.match(workflow, /needs\.changes\.outputs\.rust == 'true'/);
  assert.match(workflow, /needs\.changes\.outputs\.edge == 'true'/);
  assert.match(workflow, /needs\.changes\.outputs\.trial == 'true'/);
  assert.match(workflow, /name: lightweight repository checks/);
  assert.match(
    workflow,
    /name: Trial profile first-run and custody journey/,
  );
  assert.match(
    workflow,
    /node --test scripts\/tests\/adapter-author-docs\.test\.mjs/,
  );
  // Rust job uses nextest for the workspace suite; no redundant full build step.
  assert.match(workflow, /cargo nextest run --workspace/);
  assert.match(workflow, /cargo test --workspace --doc/);
  assert.doesNotMatch(
    workflow,
    /cargo build --workspace --all-targets/,
    "redundant workspace build should stay out of the rust job",
  );
  // Trial journey must not remain only inside the Edge integration job.
  const edgeJob = workflow.split(/name: Rust Edge \/ Console integration/)[1] ?? "";
  const edgeSection = edgeJob.split(/^  [a-z]/m)[0] ?? edgeJob;
  assert.doesNotMatch(
    edgeSection,
    /scripts\/test-iotkit-trial\.sh/,
    "trial journey should live in the dedicated trial job",
  );
  // Edge product tests stay in the rust workspace job; edge job is Console/e2e/custody.
  assert.doesNotMatch(
    edgeSection,
    /cargo test -p iotkit-edge/,
    "iotkit-edge unit/integration should not be duplicated in the edge job",
  );
});

test("CLI reads changed paths from standard input", () => {
  const script = fileURLToPath(new URL("../select-ci-jobs.mjs", import.meta.url));
  const result = spawnSync(process.execPath, [script], {
    input: "docs/README.md\n",
    encoding: "utf8",
  });

  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, "rust=false\nedge=false\ntrial=false\n");
});

test("CLI reports trial-only selection for the launcher", () => {
  const script = fileURLToPath(new URL("../select-ci-jobs.mjs", import.meta.url));
  const result = spawnSync(process.execPath, [script], {
    input: "scripts/iotkit_trial.py\n",
    encoding: "utf8",
  });

  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, "rust=false\nedge=false\ntrial=true\n");
});
