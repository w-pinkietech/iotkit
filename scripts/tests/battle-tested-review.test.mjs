import assert from "node:assert/strict";
import test from "node:test";

import {
  loadCatalog,
  parseNameStatus,
  selectEntries,
  validateCatalog,
} from "../battle-tested-review.mjs";

const catalog = loadCatalog();

test("the repository catalog is structurally valid", () => {
  assert.deepEqual(validateCatalog(catalog), []);
});

test("storage changes select power-loss, pressure, and replacement questions", () => {
  const result = selectEntries(catalog, ["core/storage/src/lib.rs"]);
  assert.deepEqual(
    result.selections.map(({ entry }) => entry.id),
    ["BT-002", "BT-003", "BT-004"],
  );
  assert.deepEqual(result.unmatchedPaths, []);
});

test("MQTT output changes select the reproduced convergence question", () => {
  const result = selectEntries(catalog, [
    "iotkit-edge/internal/outputadapter/worker.go",
  ]);
  assert.deepEqual(
    result.selections.map(({ entry }) => entry.id),
    ["BT-001"],
  );
});

test("durable output changes select convergence and storage failure questions", () => {
  const result = selectEntries(catalog, [
    "iotkit-edge/internal/store/output_v3.go",
  ]);
  assert.deepEqual(
    result.selections.map(({ entry }) => entry.id),
    ["BT-001", "BT-002", "BT-003"],
  );
});

test("HTTP admission changes select storage pressure acknowledgement review", () => {
  const result = selectEntries(catalog, [
    "iotkit-ingest-http/src/admission.rs",
  ]);
  assert.deepEqual(
    result.selections.map(({ entry }) => entry.id),
    ["BT-003"],
  );
});

test("Console changes select only the operator diagnosis question", () => {
  const result = selectEntries(catalog, [
    "iotkit-edge/frontend/src/navigation.ts",
  ]);
  assert.deepEqual(
    result.selections.map(({ entry }) => entry.id),
    ["BT-005"],
  );
});

test("a semantic concern can add a question that paths did not select", () => {
  const result = selectEntries(
    catalog,
    ["README.md"],
    ["edge-node-replacement"],
  );
  assert.deepEqual(
    result.selections.map(({ entry }) => entry.id),
    ["BT-004"],
  );
  assert.deepEqual(result.unmatchedPaths, ["README.md"]);
});

test("unknown paths and concerns remain visible", () => {
  const result = selectEntries(catalog, ["LICENSE"], ["unknown-concern"]);
  assert.equal(result.selections.length, 0);
  assert.deepEqual(result.unmatchedPaths, ["LICENSE"]);
  assert.deepEqual(result.unknownConcerns, ["unknown-concern"]);
});

test("recognized concerns without an entry remain visible without becoming unknown", () => {
  const result = selectEntries(catalog, ["README.md"], ["authentication"]);
  assert.deepEqual(result.unknownConcerns, []);
  assert.deepEqual(result.concernsWithoutEntries, ["authentication"]);
});

test("deleted and both sides of renamed paths are retained from git name status", () => {
  assert.deepEqual(
    parseNameStatus(
      "D\tcore/publish/src/wire.rs\nR100\told/path.rs\tiotkit-edge/internal/store/output_v3.go\n",
    ),
    [
      "core/publish/src/wire.rs",
      "old/path.rs",
      "iotkit-edge/internal/store/output_v3.go",
    ],
  );
});

test("catalog validation rejects catch-all routing", () => {
  const broken = structuredClone(catalog);
  broken.entries[0].path_prefixes = ["**/*"];
  assert.match(validateCatalog(broken).join("\n"), /catch-all/);
});

test("catalog validation rejects missing evidence links", () => {
  const broken = structuredClone(catalog);
  broken.entries[0].guards = ["scripts/does-not-exist.sh"];
  assert.match(validateCatalog(broken).join("\n"), /does not exist/);
});
