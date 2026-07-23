import assert from "node:assert/strict";
import test from "node:test";

import { selectCiJobs } from "../select-ci-jobs.mjs";

const cases = [
  {
    name: "documentation and repository guidance use lightweight checks only",
    paths: ["docs/okf/en/index.md", "AGENTS.md", "CONTRIBUTING.ja.md"],
    expected: { rust: false, edge: false },
  },
  {
    name: "Rust workspace changes select only Rust",
    paths: ["core/ledger/src/lib.rs", "Cargo.lock"],
    expected: { rust: true, edge: false },
  },
  {
    name: "IoTKit Edge changes select only Edge",
    paths: ["iotkit-edge/internal/store/store.go"],
    expected: { rust: false, edge: true },
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
