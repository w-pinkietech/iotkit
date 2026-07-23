import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { selectCiJobs } from "../select-ci-jobs.mjs";

const cases = [
  {
    name: "documentation and repository guidance use lightweight checks only",
    paths: ["docs/okf/en/index.md", "AGENTS.md", "CONTRIBUTING.ja.md"],
    expected: { rust: false, edge: false },
  },
  {
    name: "Edge Node-only changes select only Rust",
    paths: ["edge-node/core/ledger/src/lib.rs"],
    expected: { rust: true, edge: false },
  },
  {
    name: "workspace dependency changes also select Edge integration",
    paths: ["Cargo.lock"],
    expected: { rust: true, edge: true },
  },
  {
    name: "Rust IoTKit Edge changes select Rust and Edge integration",
    paths: ["edge/src/storage/mod.rs"],
    expected: { rust: true, edge: true },
  },
  {
    name: "Output Adapter changes select Rust and Edge integration",
    paths: ["edge/output-adapters/example/src/lib.rs"],
    expected: { rust: true, edge: true },
  },
  {
    name: "IoTKit Edge verification scripts select Rust and integration",
    paths: ["scripts/test-edge-console-e2e.sh"],
    expected: { rust: true, edge: true },
  },
  {
    name: "shared contract fixtures select both",
    paths: ["testdata/egress/v1/record-batch.json"],
    expected: { rust: true, edge: true },
  },
  {
    name: "workflow changes select both",
    paths: [".github/workflows/ci.yml"],
    expected: { rust: true, edge: true },
  },
  {
    name: "classifier changes select both",
    paths: ["scripts/select-ci-jobs.mjs"],
    expected: { rust: true, edge: true },
  },
  {
    name: "unknown paths select both",
    paths: ["new-component/file.txt"],
    expected: { rust: true, edge: true },
  },
  {
    name: "empty input selects both",
    paths: [],
    expected: { rust: true, edge: true },
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
  assert.match(workflow, /name: lightweight repository checks/);
});

test("CLI reads changed paths from standard input", () => {
  const script = fileURLToPath(new URL("../select-ci-jobs.mjs", import.meta.url));
  const result = spawnSync(process.execPath, [script], {
    input: "docs/README.md\n",
    encoding: "utf8",
  });

  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, "rust=false\nedge=false\n");
});
