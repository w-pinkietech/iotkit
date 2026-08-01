import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { selectCiJobs, selectRustPackages } from "../select-ci-jobs.mjs";

const none = { rust: false, edge: false, trial: false, packages: "" };
const rustOnly = (packages) => ({
  rust: true,
  edge: false,
  trial: false,
  packages,
});
const rustTrial = (packages) => ({
  rust: true,
  edge: false,
  trial: true,
  packages,
});
const rustEdge = (packages) => ({
  rust: true,
  edge: true,
  trial: false,
  packages,
});
const rustEdgeTrial = (packages = "all") => ({
  rust: true,
  edge: true,
  trial: true,
  packages,
});
const trialOnly = {
  rust: false,
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
      "scripts/tests/adapter-author-docs.test.mjs",
    ],
    expected: none,
  },
  {
    name: "shared core ledger expands beyond the focus threshold to full suite",
    paths: ["edge-node/core/ledger/src/lib.rs"],
    expected: rustOnly("all"),
  },
  {
    name: "workspace dependency changes select Rust, Edge, and trial image rebuild",
    paths: ["Cargo.lock"],
    expected: rustEdgeTrial("all"),
  },
  {
    name: "Rust IoTKit Edge changes focus clippy/test on iotkit-edge",
    paths: ["edge/src/storage/mod.rs"],
    expected: rustEdge("iotkit-edge"),
  },
  {
    name: "Output Adapter example focuses example package and reverse consumers",
    paths: ["edge/output-adapters/example/src/lib.rs"],
    expected: rustEdge("iotkit-output-adapter-example"),
  },
  {
    name: "IoTKit Edge verification scripts select Rust and integration",
    paths: ["scripts/test-edge-console-e2e.sh"],
    expected: rustEdge("all"),
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
    expected: rustEdge("all"),
  },
  {
    name: "trial-related Edge runtime/config selects Rust, Edge, and trial Docker",
    paths: ["edge/src/composition/runtime_config.rs"],
    expected: rustEdgeTrial("iotkit-edge"),
  },
  {
    name: "trial banner template/CSS select Rust, Edge, and trial Docker",
    paths: [
      "edge/src/web/templates/console.html",
      "edge/frontend/static/edge.css",
    ],
    expected: rustEdgeTrial("iotkit-edge"),
  },
  {
    name: "shared contract fixtures select Rust, Edge, and trial",
    paths: ["testdata/egress/v1/record-batch.json"],
    expected: rustEdgeTrial("all"),
  },
  {
    name: "workflow changes select all heavy jobs",
    paths: [".github/workflows/ci.yml"],
    expected: rustEdgeTrial("all"),
  },
  {
    name: "classifier changes select all heavy jobs",
    paths: ["scripts/select-ci-jobs.mjs"],
    expected: rustEdgeTrial("all"),
  },
  {
    name: "unknown paths select all heavy jobs",
    paths: ["new-component/file.txt"],
    expected: rustEdgeTrial("all"),
  },
  {
    name: "empty input selects all heavy jobs",
    paths: [],
    expected: rustEdgeTrial("all"),
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
    name: "Edge Dockerfile rebuilds Edge and trial images",
    paths: ["edge/Dockerfile"],
    expected: rustEdgeTrial("all"),
  },
  {
    name: "field deploy compose selects Rust and Edge without trial",
    paths: ["deploy/compose.edge.yaml"],
    expected: rustEdge("all"),
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
  assert.match(workflow, /needs\.changes\.outputs\.edge == 'true'/);
  assert.match(workflow, /needs\.changes\.outputs\.trial == 'true'/);
  assert.match(workflow, /needs\.changes\.outputs\.packages/);
  assert.match(workflow, /name: lightweight repository checks/);
  assert.match(
    workflow,
    /name: Trial profile first-run and custody journey/,
  );
  assert.match(
    workflow,
    /node --test scripts\/tests\/adapter-author-docs\.test\.mjs/,
  );
  // Focused package selection drives clippy/nextest when not "all".
  assert.match(workflow, /cargo nextest run/);
  assert.match(workflow, /PACKAGES/);
  assert.doesNotMatch(
    workflow,
    /cargo build --workspace --all-targets/,
    "redundant workspace build should stay out of the rust job",
  );
  // Trial journey must not remain only inside the Edge integration job.
  const edgeJob =
    workflow.split(/name: Rust Edge \/ Console integration/)[1] ?? "";
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
    "rust=false\nedge=false\ntrial=false\npackages=\n",
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
    "rust=false\nedge=false\ntrial=true\npackages=\n",
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
    "rust=true\nedge=false\ntrial=true\npackages=iotkit-edge-node,trial-sample-adapter\n",
  );
});
