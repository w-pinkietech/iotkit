import assert from "node:assert/strict";
import test from "node:test";

import {
  evaluateFreshnessSoft,
  formatSelection,
  formatSoftCheck,
  hasNoProductDocsReason,
  loadRules,
  parseNameStatus,
  selectImpact,
  validateRules,
} from "../product-docs-impact.mjs";

const rules = loadRules();

test("repository impact rules are structurally valid", () => {
  assert.deepEqual(validateRules(rules), []);
});

test("ingest paths select ingest contract and terminology", () => {
  const result = selectImpact(rules, [
    "edge-node/ingest/http/src/admission.rs",
  ]);
  assert.deepEqual(
    result.candidates.map((c) => c.docPath).sort(),
    ["concepts/terminology.md", "contracts/ingest-v1.md"],
  );
  assert.ok(result.candidates.every((c) => c.fullPaths.length === 2));
  assert.ok(
    result.candidates
      .find((c) => c.docPath === "contracts/ingest-v1.md")
      .fullPaths.includes("docs/product/en/contracts/ingest-v1.md"),
  );
  assert.deepEqual(result.unmatchedPaths, []);
});

test("input adapter and driver paths select input-adapter contract", () => {
  const result = selectImpact(rules, [
    "edge-node/adapters/rpi-local/src/lib.rs",
    "edge-node/input/runtimes/polling/src/lib.rs",
  ]);
  assert.ok(
    result.candidates.some((c) => c.docPath === "contracts/input-adapter-v1.md"),
  );
  assert.ok(
    result.candidates
      .find((c) => c.docPath === "contracts/input-adapter-v1.md")
      .ruleIds.includes("input-adapter-contract"),
  );
});

test("custody paths select custody contract and architecture", () => {
  const result = selectImpact(rules, [
    "edge-node/core/ledger/src/lib.rs",
    "edge/src/mqtt/ingest/runtime.rs",
  ]);
  const docs = result.candidates.map((c) => c.docPath).sort();
  assert.ok(docs.includes("contracts/edge-node-custody-v1.md"));
  assert.ok(docs.includes("architecture/system-overview.md"));
});

test("output adapter package paths select output-adapter contract", () => {
  const result = selectImpact(rules, [
    "edge/output-adapters/generic-mqtt-json-v1/src/lib.rs",
    "edge/src/mqtt/output/runtime.rs",
  ]);
  assert.ok(
    result.candidates.some((c) => c.docPath === "contracts/output-adapter-v1.md"),
  );
  assert.equal(
    result.candidates.find((c) => c.docPath === "contracts/output-adapter-v1.md")
      .ruleIds.includes("output-adapter-contract"),
    true,
  );
});

test("semantic_output storage also hits storage-capacity (coarse lower bound)", () => {
  const result = selectImpact(rules, [
    "edge/src/storage/semantic_output/operations.rs",
  ]);
  const docs = result.candidates.map((c) => c.docPath);
  assert.ok(docs.includes("contracts/output-adapter-v1.md"));
  assert.ok(docs.includes("operations/storage-capacity.md"));
});

test("console frontend paths select architecture and installation ops", () => {
  const result = selectImpact(rules, ["edge/frontend/src/navigation.ts"]);
  const docs = result.candidates.map((c) => c.docPath).sort();
  assert.deepEqual(docs, [
    "architecture/system-overview.md",
    "operations/installation-and-recovery.md",
  ]);
});

test("backup and deploy paths select recovery operations docs", () => {
  const result = selectImpact(rules, [
    "edge/src/backup/mod.rs",
    "deploy/edge/Dockerfile",
  ]);
  const docs = result.candidates.map((c) => c.docPath).sort();
  assert.ok(docs.includes("operations/installation-and-recovery.md"));
  assert.ok(docs.includes("operations/edge-node-hardware-recovery.md"));
});

test("trial scripts select trial-profile only", () => {
  const result = selectImpact(rules, ["scripts/iotkit_trial.py"]);
  assert.deepEqual(
    result.candidates.map((c) => c.docPath),
    ["operations/trial-profile.md"],
  );
});

test("product corpus paths set corpus_touched without inventing candidates", () => {
  const result = selectImpact(rules, [
    "docs/product/en/contracts/ingest-v1.md",
    "docs/product/ja/contracts/ingest-v1.md",
  ]);
  assert.ok(result.flags.includes("corpus_touched"));
  assert.deepEqual(result.candidates, []);
  assert.deepEqual(result.alreadyTouchedProductDocs, [
    "docs/product/en/contracts/ingest-v1.md",
    "docs/product/ja/contracts/ingest-v1.md",
  ]);
});

test("unmatched paths remain visible and empty is not claimed safe", () => {
  const result = selectImpact(rules, ["LICENSE", "README.md"]);
  assert.equal(result.candidates.length, 0);
  assert.deepEqual(result.unmatchedPaths, ["LICENSE", "README.md"]);
  const text = formatSelection(result, ["LICENSE", "README.md"]);
  assert.match(text, /not proof that product docs need no update/i);
  assert.match(text, /Candidates: none/);
  assert.match(text, /Unmatched paths/);
});

test("combined code + docs paths merge candidates and touch list", () => {
  const result = selectImpact(rules, [
    "edge-node/ingest/contract/src/lib.rs",
    "docs/product/en/contracts/ingest-v1.md",
  ]);
  assert.ok(
    result.candidates.some((c) => c.docPath === "contracts/ingest-v1.md"),
  );
  assert.ok(
    result.alreadyTouchedProductDocs.includes(
      "docs/product/en/contracts/ingest-v1.md",
    ),
  );
});

test("name-status parse keeps renames and deletes", () => {
  assert.deepEqual(
    parseNameStatus(
      "D\tedge-node/core/publish/src/wire.rs\nR100\told/path.rs\tedge/src/storage/mod.rs\n",
    ),
    [
      "edge-node/core/publish/src/wire.rs",
      "old/path.rs",
      "edge/src/storage/mod.rs",
    ],
  );
});

test("validation rejects catch-all path prefixes", () => {
  const broken = structuredClone(rules);
  broken.rules[0].path_prefixes = ["**/*"];
  assert.match(validateRules(broken).join("\n"), /catch-all/);
});

test("validation rejects missing product doc targets", () => {
  const broken = structuredClone(rules);
  broken.rules[0].doc_paths = ["contracts/does-not-exist-v9.md"];
  assert.match(validateRules(broken).join("\n"), /does-not-exist/);
});

test("formatSelection lists bilingual full paths and empty-not-safe note", () => {
  const result = selectImpact(rules, ["scripts/iotkit"]);
  const text = formatSelection(result, ["scripts/iotkit"]);
  assert.match(text, /docs\/product\/en\/operations\/trial-profile\.md/);
  assert.match(text, /docs\/product\/ja\/operations\/trial-profile\.md/);
  assert.match(text, /Empty selection is not proof/i);
  assert.match(text, /Authority: docs\/product\//);
});

test("soft-check warns when impact exists without docs or PR reason", () => {
  const result = evaluateFreshnessSoft({
    rules,
    paths: ["edge-node/ingest/http/src/admission.rs"],
    prBody: "## Summary\n\nChanged admission only.\n",
  });
  assert.equal(result.status, "warn");
  assert.equal(result.code, "missing_docs_and_reason");
  assert.match(formatSoftCheck(result), /soft warning only/i);
});

test("soft-check ok when docs/product is updated", () => {
  const result = evaluateFreshnessSoft({
    rules,
    paths: [
      "edge-node/ingest/http/src/admission.rs",
      "docs/product/en/contracts/ingest-v1.md",
      "docs/product/ja/contracts/ingest-v1.md",
    ],
    prBody: "",
  });
  assert.equal(result.status, "ok");
  assert.equal(result.code, "product_docs_updated");
});

test("soft-check ok when PR records no-update reason", () => {
  const prBody = `## Product docs impact / 正本への影響

- Updated product-doc paths / 更新した正本:
  none
- No product-docs update reason / 更新しない理由:
  Internal refactor of admission helpers; public ingest contract and operator steps unchanged.
`;
  const result = evaluateFreshnessSoft({
    rules,
    paths: ["edge-node/ingest/http/src/admission.rs"],
    prBody,
  });
  assert.equal(result.status, "ok");
  assert.equal(result.code, "no_update_reason_recorded");
  assert.equal(hasNoProductDocsReason(prBody), true);
});

test("soft-check does not treat empty template comments as a reason", () => {
  const prBody = `## Product docs impact / 正本への影響

- Updated product-doc paths / 更新した正本:
  <!-- e.g. docs/product/... -->
- No product-docs update reason / 更新しない理由:
  <!-- Required when paths are "none" -->
`;
  assert.equal(hasNoProductDocsReason(prBody), false);
  const result = evaluateFreshnessSoft({
    rules,
    paths: ["edge-node/core/ledger/src/lib.rs"],
    prBody,
  });
  assert.equal(result.status, "warn");
});

test("soft-check ok with no impact candidates (still not a safety proof)", () => {
  const result = evaluateFreshnessSoft({
    rules,
    paths: ["LICENSE"],
    prBody: "",
  });
  assert.equal(result.status, "ok");
  assert.equal(result.code, "no_impact_candidates");
  assert.match(result.message, /not a safety proof/i);
});
