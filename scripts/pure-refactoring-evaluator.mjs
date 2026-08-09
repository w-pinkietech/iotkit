#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const defaultRepoRoot = resolve(dirname(scriptPath), "..");
const assetPaths = {
  1: {
    schema: "review/pure-refactoring/schema.v1.json",
    rubric: "review/pure-refactoring/rubric.v1.json",
    corpus: "review/pure-refactoring/corpus.v1.json",
  },
  2: {
    schema: "review/pure-refactoring/schema.v1.json",
    rubric: "review/pure-refactoring/rubric.v1.json",
    corpus: "review/pure-refactoring/corpus.v2.json",
    provenance: "review/pure-refactoring/historical-provenance.v2.json",
    selectionPolicy: "review/pure-refactoring/historical-selection-policy.v1.md",
  },
};
const supportedVersion = 1;
const historicalCorpusVersion = 2;
const classifications = ["proven", "not_proven"];
const classificationSet = new Set(classifications);
const caseKinds = new Set(["positive", "negative", "adversarial"]);
const requiredRiskCategories = [
  "auth_secrets",
  "custody_data_loss",
  "database_migration",
  "backup_restore",
  "concurrency_timing",
  "configuration_deployment",
  "dependencies",
  "generated_artifacts",
  "public_wire_api_contract",
  "product_documentation_authority",
  "operator_visible_behavior",
];
const expectedReasonCodes = [
  { code: "structural_only", classification: "proven", hard_exclusion: false, risk_category: null },
  { code: "insufficient_evidence", classification: "not_proven", hard_exclusion: false, risk_category: null },
  { code: "auth_secrets", classification: "not_proven", hard_exclusion: true, risk_category: "auth_secrets" },
  { code: "custody_data_loss", classification: "not_proven", hard_exclusion: true, risk_category: "custody_data_loss" },
  { code: "database_migration", classification: "not_proven", hard_exclusion: true, risk_category: "database_migration" },
  { code: "backup_restore", classification: "not_proven", hard_exclusion: true, risk_category: "backup_restore" },
  { code: "concurrency_timing", classification: "not_proven", hard_exclusion: true, risk_category: "concurrency_timing" },
  { code: "configuration_deployment", classification: "not_proven", hard_exclusion: true, risk_category: "configuration_deployment" },
  { code: "dependencies", classification: "not_proven", hard_exclusion: true, risk_category: "dependencies" },
  { code: "generated_artifacts", classification: "not_proven", hard_exclusion: true, risk_category: "generated_artifacts" },
  { code: "public_wire_api_contract", classification: "not_proven", hard_exclusion: true, risk_category: "public_wire_api_contract" },
  { code: "product_documentation_authority", classification: "not_proven", hard_exclusion: true, risk_category: "product_documentation_authority" },
  { code: "operator_visible_behavior", classification: "not_proven", hard_exclusion: true, risk_category: "operator_visible_behavior" },
];
const schemaKeys = ["schema_version", "rubric_version", "prompt_version", "result_format"];
const schemaResultFormatKeys = ["top_level_keys", "run_keys", "case_keys", "run_statuses"];
const resultTopLevelKeys = ["schema_version", "rubric_version", "prompt_version", "bundle_sha256", "runs"];
const resultRunKeys = ["run_id", "model_id", "status", "cases"];
const resultCaseKeys = ["case_id", "classification", "reason_codes", "evidence"];
const bundleKeys = [
  "schema_version",
  "rubric_version",
  "prompt_version",
  "rubric_sha256",
  "corpus_sha256",
  "instructions",
  "response_contract",
  "cases",
  "bundle_sha256",
];
const historicalProvenanceKeys = [
  "provenance_version",
  "corpus_version",
  "cutoff_commit_sha",
  "selection_policy_sha256",
  "cases",
];
const historicalProvenanceCaseKeys = [
  "case_id",
  "pull_request",
  "merge_commit_sha",
  "source_commit_sha",
  "source_parent_sha",
  "pathspec",
  "selection_mode",
  "source_diff_sha256",
  "model_diff_sha256",
  "sanitization",
  "answer_rationale",
];
const historicalSanitizationKeys = [
  "line_endings",
  "removed_lines",
  "removed_index_lines",
  "excluded_metadata",
  "content_redaction",
];
const historicalRemovedIndexLineKeys = ["raw_line", "text"];
const historicalMetadata = ["pull_request", "commit", "title", "label"];
const sha256Pattern = /^[a-f0-9]{64}$/;
const gitShaPattern = /^[a-f0-9]{40}$/;
const gitIndexLinePattern = /^index [0-9a-f]+\.\.[0-9a-f]+(?: \d+)?$/;

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function hasExactKeys(value, allowedKeys, requiredKeys, path, errors) {
  if (!isRecord(value)) {
    errors.push(`${path} must be an object`);
    return false;
  }
  const allowed = new Set(allowedKeys);
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) errors.push(`${path} has unknown key: ${key}`);
  }
  for (const key of requiredKeys) {
    if (!(key in value)) errors.push(`${path} missing required key: ${key}`);
  }
  return true;
}

function nonEmptyString(value, path, errors) {
  if (typeof value !== "string" || value.trim() === "") {
    errors.push(`${path} must be a non-empty string`);
    return false;
  }
  return true;
}

function uniqueStringArray(value, path, errors, { allowEmpty = false } = {}) {
  if (!Array.isArray(value) || (!allowEmpty && value.length === 0)) {
    errors.push(`${path} must be ${allowEmpty ? "an array" : "a non-empty array"}`);
    return [];
  }
  const values = [];
  const seen = new Set();
  for (const [index, item] of value.entries()) {
    if (!nonEmptyString(item, `${path}[${index}]`, errors)) continue;
    if (seen.has(item)) {
      errors.push(`${path} duplicates value: ${item}`);
      continue;
    }
    seen.add(item);
    values.push(item);
  }
  return values;
}

function exactStringArray(value, expected, path, errors) {
  uniqueStringArray(value, path, errors);
  if (!Array.isArray(value) || JSON.stringify(value) !== JSON.stringify(expected)) {
    errors.push(`${path} must equal the version 1 contract`);
  }
}

function validateVersion(value, name, expected, path, errors) {
  if (value?.[name] !== expected) {
    errors.push(`${path}.${name} must be ${expected}`);
  }
}

function validateMatchingVersions(value, expected, path, errors) {
  for (const name of ["schema_version", "rubric_version", "prompt_version"]) {
    if (value?.[name] !== expected?.[name]) {
      errors.push(`${path}.${name} must match the checked-in version`);
    }
  }
}

function sameStrings(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function reasonMap(rubric) {
  return new Map((rubric?.reason_codes ?? []).map((entry) => [entry.code, entry]));
}

function validateDecision(classification, codes, reasons, path, errors) {
  if (!classificationSet.has(classification)) {
    errors.push(`${path}.classification is invalid`);
  }
  const values = uniqueStringArray(codes, `${path}.reason_codes`, errors);
  for (const code of values) {
    const reason = reasons.get(code);
    if (!reason) {
      errors.push(`${path}.reason_codes contains unknown code: ${code}`);
    } else if (classificationSet.has(classification) && reason.classification !== classification) {
      errors.push(`${path}.reason_codes is incompatible with classification: ${code}`);
    }
  }
  if (!classificationSet.has(classification)) return;
  if (classification === "proven" && !sameStrings(values, ["structural_only"])) {
    errors.push(`${path}.proven requires only structural_only`);
  }
  if (classification === "not_proven" && values.includes("structural_only")) {
    errors.push(`${path}.not_proven must not use structural_only`);
  }
}

export function canonicalJson(value) {
  if (value === null) return "null";
  if (typeof value === "string" || typeof value === "boolean") return JSON.stringify(value);
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error("canonical JSON rejects non-finite numbers");
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) return `[${value.map((item) => canonicalJson(item)).join(",")}]`;
  if (isRecord(value)) {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
      .join(",")}}`;
  }
  throw new Error("canonical JSON accepts only JSON values");
}

export function sha256Json(value) {
  return createHash("sha256").update(canonicalJson(value)).digest("hex");
}

export function sha256Bytes(value) {
  if (typeof value !== "string" && !ArrayBuffer.isView(value)) {
    throw new TypeError("SHA-256 input must be a string or byte view");
  }
  return createHash("sha256").update(value).digest("hex");
}

export function opaqueCaseId(diff) {
  if (typeof diff !== "string") throw new TypeError("opaque case IDs require a string diff");
  return `RF-${createHash("sha256").update(diff).digest("hex").slice(0, 12).toUpperCase()}`;
}

export function sanitizeHistoricalDiff(sourceDiff) {
  if (typeof sourceDiff !== "string") {
    throw new TypeError("historical diff sanitization requires a string");
  }
  return sourceDiff
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
    .split("\n")
    .filter((line) => !gitIndexLinePattern.test(line))
    .join("\n");
}

export function restoreHistoricalDiff(modelDiff, removedIndexLines) {
  if (typeof modelDiff !== "string") {
    throw new TypeError("historical diff restoration requires a string model diff");
  }
  if (!Array.isArray(removedIndexLines) || removedIndexLines.length === 0) {
    throw new TypeError("historical diff restoration requires removed index lines");
  }
  const rawLineCount = modelDiff.split("\n").length + removedIndexLines.length;
  let previousLine = 0;
  for (const entry of removedIndexLines) {
    if (!isRecord(entry) || !Number.isSafeInteger(entry.raw_line) || entry.raw_line < 1) {
      throw new TypeError("historical removed index lines require positive raw line positions");
    }
    if (entry.raw_line <= previousLine || entry.raw_line > rawLineCount) {
      throw new TypeError("historical removed index lines must be ordered raw line positions");
    }
    if (typeof entry.text !== "string" || !gitIndexLinePattern.test(entry.text)) {
      throw new TypeError("historical removed index lines must be Git index lines");
    }
    previousLine = entry.raw_line;
  }

  const modelLines = modelDiff.split("\n");
  const restored = [];
  let modelIndex = 0;
  let removedIndex = 0;
  for (let rawLine = 1; rawLine <= rawLineCount; rawLine += 1) {
    const removed = removedIndexLines[removedIndex];
    if (removed?.raw_line === rawLine) {
      restored.push(removed.text);
      removedIndex += 1;
      continue;
    }
    restored.push(modelLines[modelIndex]);
    modelIndex += 1;
  }
  if (modelIndex !== modelLines.length || removedIndex !== removedIndexLines.length) {
    throw new TypeError("historical removed index lines cannot reconstruct the raw diff");
  }
  return restored.join("\n");
}

function hasHistoricalMetadata(diff) {
  return diff.split("\n").some((line) => {
    const content = /^[+\- ]/.test(line) ? line.slice(1) : line;
    const trimmed = content.trimStart();
    if (/^(?:commit(?:\s+[0-9a-f]{7,40}|:\s|$)|(?:pull[ -]?request|pr)\s*(?:#|:)?\s*\d+)/i.test(trimmed)) {
      return true;
    }
    const titleOrLabel = /^(?:title|labels?)\s*:\s*(.*)$/i.exec(content);
    return content === trimmed && titleOrLabel !== null && (
      titleOrLabel[1] === "" || !/^["'`]/.test(titleOrLabel[1])
    );
  });
}

function hasRedactedContent(diff) {
  return /(?:\[redacted\]|<redacted>|\bredacted\b)/i.test(diff);
}

function isHistoricalCorpus(corpus) {
  return isRecord(corpus) && Object.hasOwn(corpus, "corpus_version");
}

export function validateSchema(schema) {
  const errors = [];
  if (!hasExactKeys(schema, schemaKeys, schemaKeys, "schema", errors)) return errors;
  validateVersion(schema, "schema_version", supportedVersion, "schema", errors);
  validateVersion(schema, "rubric_version", supportedVersion, "schema", errors);
  validateVersion(schema, "prompt_version", supportedVersion, "schema", errors);
  if (!hasExactKeys(
    schema.result_format,
    schemaResultFormatKeys,
    schemaResultFormatKeys,
    "schema.result_format",
    errors,
  )) return errors;
  exactStringArray(schema.result_format.top_level_keys, resultTopLevelKeys, "schema.result_format.top_level_keys", errors);
  exactStringArray(schema.result_format.run_keys, resultRunKeys, "schema.result_format.run_keys", errors);
  exactStringArray(schema.result_format.case_keys, resultCaseKeys, "schema.result_format.case_keys", errors);
  exactStringArray(
    schema.result_format.run_statuses,
    ["complete", "ambiguous", "incomplete"],
    "schema.result_format.run_statuses",
    errors,
  );
  return errors;
}

export function validateRubric(rubric, schema) {
  const errors = [];
  const keys = [
    "schema_version",
    "rubric_version",
    "prompt_version",
    "minimum_runs",
    "minimum_safe_positives",
    "required_risk_categories",
    "reason_codes",
  ];
  if (!hasExactKeys(rubric, keys, keys, "rubric", errors)) return errors;
  validateMatchingVersions(rubric, schema, "rubric", errors);
  if (!Number.isInteger(rubric.minimum_runs) || rubric.minimum_runs < 3) {
    errors.push("rubric.minimum_runs must be an integer of at least 3");
  }
  if (!Number.isInteger(rubric.minimum_safe_positives) || rubric.minimum_safe_positives < 4) {
    errors.push("rubric.minimum_safe_positives must be an integer of at least 4");
  }
  exactStringArray(
    rubric.required_risk_categories,
    requiredRiskCategories,
    "rubric.required_risk_categories",
    errors,
  );
  if (!Array.isArray(rubric.reason_codes) || rubric.reason_codes.length === 0) {
    errors.push("rubric.reason_codes must be a non-empty array");
    return errors;
  }
  const seenCodes = new Set();
  const expectedByCode = new Map(expectedReasonCodes.map((entry) => [entry.code, entry]));
  for (const [index, entry] of rubric.reason_codes.entries()) {
    const path = `rubric.reason_codes[${index}]`;
    const keysForReason = ["code", "classification", "hard_exclusion", "risk_category", "description"];
    if (!hasExactKeys(entry, keysForReason, keysForReason, path, errors)) continue;
    if (!/^[a-z][a-z0-9_]*$/.test(entry.code ?? "")) {
      errors.push(`${path}.code must be a lowercase identifier`);
    } else if (seenCodes.has(entry.code)) {
      errors.push(`${path}.code duplicates ${entry.code}`);
    } else {
      seenCodes.add(entry.code);
    }
    if (!classificationSet.has(entry.classification)) {
      errors.push(`${path}.classification is invalid`);
    }
    if (typeof entry.hard_exclusion !== "boolean") {
      errors.push(`${path}.hard_exclusion must be boolean`);
    }
    if (entry.risk_category !== null && !requiredRiskCategories.includes(entry.risk_category)) {
      errors.push(`${path}.risk_category is invalid`);
    }
    nonEmptyString(entry.description, `${path}.description`, errors);
    const expected = expectedByCode.get(entry.code);
    if (!expected) {
      errors.push(`${path}.code is not controlled by rubric version 1: ${entry.code}`);
    } else {
      for (const property of ["classification", "hard_exclusion", "risk_category"]) {
        if (entry[property] !== expected[property]) {
          errors.push(`${path}.${property} must match controlled code ${entry.code}`);
        }
      }
    }
  }
  if (!sameStrings(rubric.reason_codes.map((entry) => entry?.code), expectedReasonCodes.map((entry) => entry.code))) {
    errors.push("rubric.reason_codes must preserve the version 1 controlled order");
  }
  return errors;
}

export function validateCorpus(corpus, rubric, schema) {
  const errors = [];
  const historical = isHistoricalCorpus(corpus);
  const keys = historical
    ? ["schema_version", "rubric_version", "prompt_version", "corpus_version", "provenance_sha256", "cases"]
    : ["schema_version", "rubric_version", "prompt_version", "cases"];
  if (!hasExactKeys(corpus, keys, keys, "corpus", errors)) return errors;
  validateMatchingVersions(corpus, schema, "corpus", errors);
  if (historical) {
    validateVersion(corpus, "corpus_version", historicalCorpusVersion, "corpus", errors);
    if (!sha256Pattern.test(corpus.provenance_sha256 ?? "")) {
      errors.push("corpus.provenance_sha256 must be a lowercase SHA-256 digest");
    }
  }
  if (corpus.rubric_version !== rubric?.rubric_version) {
    errors.push("corpus.rubric_version must match the checked-in rubric");
  }
  if (!Array.isArray(corpus.cases) || corpus.cases.length === 0) {
    errors.push("corpus.cases must be a non-empty array");
    return errors;
  }
  const reasons = reasonMap(rubric);
  const ids = new Set();
  const presentKinds = new Set();
  let safePositives = 0;
  for (const [index, entry] of corpus.cases.entries()) {
    const path = `corpus.cases[${index}]`;
    const entryKeys = [
      "id",
      "title",
      "kind",
      "risk_categories",
      "diff",
      "expected_label",
      "expected_reason_codes",
      "dangerous",
    ];
    if (!hasExactKeys(entry, entryKeys, entryKeys, path, errors)) continue;
    if (!/^RF-[0-9A-F]{12}$/.test(entry.id ?? "")) {
      errors.push(`${path}.id must match RF-XXXXXXXXXXXX`);
    } else if (ids.has(entry.id)) {
      errors.push(`${path}.id duplicates ${entry.id}`);
    } else {
      ids.add(entry.id);
    }
    nonEmptyString(entry.title, `${path}.title`, errors);
    const hasDiff = nonEmptyString(entry.diff, `${path}.diff`, errors);
    if (hasDiff && entry.id !== opaqueCaseId(entry.diff)) {
      errors.push(`${path}.id must equal the opaque SHA-256 ID derived from its diff`);
    }
    if (historical && hasDiff) {
      if (entry.diff !== sanitizeHistoricalDiff(entry.diff)) {
        errors.push(`${path}.diff must use LF and remove diff index lines`);
      }
      if (!entry.diff.startsWith("diff --git ")) {
        errors.push(`${path}.diff must be a sanitized source diff`);
      }
      if (hasHistoricalMetadata(entry.diff)) {
        errors.push(`${path}.diff must not include pull-request, commit, title, or label metadata`);
      }
      if (hasRedactedContent(entry.diff)) {
        errors.push(`${path}.diff must not contain redacted content`);
      }
    }
    if (!caseKinds.has(entry.kind)) {
      errors.push(`${path}.kind is invalid`);
    } else {
      presentKinds.add(entry.kind);
    }
    const risks = uniqueStringArray(entry.risk_categories, `${path}.risk_categories`, errors, { allowEmpty: true });
    for (const risk of risks) {
      if (!rubric?.required_risk_categories?.includes(risk)) {
        errors.push(`${path}.risk_categories contains unknown category: ${risk}`);
      }
    }
    validateDecision(entry.expected_label, entry.expected_reason_codes, reasons, `${path}.expected`, errors);
    if (typeof entry.dangerous !== "boolean") {
      errors.push(`${path}.dangerous must be boolean`);
    }
    if (entry.kind === "positive") {
      if (entry.dangerous || entry.expected_label !== "proven" || risks.length !== 0) {
        errors.push(`${path}.positive must be safe, risk-free, and proven`);
      } else {
        safePositives += 1;
      }
    } else if (entry.expected_label !== "not_proven") {
      errors.push(`${path}.${entry.kind} must be not_proven`);
    }
    if (entry.dangerous && entry.expected_label !== "not_proven") {
      errors.push(`${path}.dangerous cases must be not_proven`);
    }
    for (const code of entry.expected_reason_codes ?? []) {
      const reason = reasons.get(code);
      if (reason?.hard_exclusion && !risks.includes(reason.risk_category)) {
        errors.push(`${path}.expected reason ${code} must declare risk category ${reason.risk_category}`);
      }
    }
  }
  if (safePositives < rubric?.minimum_safe_positives) {
    errors.push(`corpus must contain at least ${rubric?.minimum_safe_positives} safe positive cases`);
  }
  const caseIds = corpus.cases.map((entry) => entry.id);
  if (!sameStrings(caseIds, [...caseIds].sort())) {
    errors.push("corpus.cases must be sorted lexicographically by opaque ID");
  }
  const midpoint = Math.ceil(corpus.cases.length / 2);
  for (const [halfName, cases] of [["first", corpus.cases.slice(0, midpoint)], ["second", corpus.cases.slice(midpoint)]]) {
    for (const label of classifications) {
      if (!cases.some((entry) => entry.expected_label === label)) {
        errors.push(`corpus ${halfName} half must contain both expected labels`);
        break;
      }
    }
  }
  for (const kind of ["negative", "adversarial"]) {
    if (!presentKinds.has(kind)) errors.push(`corpus must contain a ${kind} case`);
  }
  for (const category of rubric?.required_risk_categories ?? []) {
    if (!corpus.cases.some((entry) => entry.risk_categories?.includes(category))) {
      errors.push(`corpus is missing required risk coverage: ${category}`);
    }
  }
  for (const reason of reasons.values()) {
    if (!reason.hard_exclusion) continue;
    if (!corpus.cases.some(
      (entry) =>
        entry.dangerous &&
        (entry.kind === "negative" || entry.kind === "adversarial") &&
        entry.expected_label === "not_proven" &&
        entry.expected_reason_codes?.includes(reason.code),
    )) {
      errors.push(`corpus is missing dangerous hard-exclusion case: ${reason.code}`);
    }
  }
  return errors;
}

function nonEmptyBytes(value, path, errors) {
  if (typeof value === "string" && value.length > 0) return true;
  if (ArrayBuffer.isView(value) && value.byteLength > 0) return true;
  errors.push(`${path} must be non-empty policy bytes`);
  return false;
}

function validateHistoricalSanitization(value, path, errors) {
  const startErrorCount = errors.length;
  if (!hasExactKeys(value, historicalSanitizationKeys, historicalSanitizationKeys, path, errors)) return null;
  if (value.line_endings !== "lf") errors.push(`${path}.line_endings must be lf`);
  if (!sameStrings(value.removed_lines, ["index"])) {
    errors.push(`${path}.removed_lines must equal [\"index\"]`);
  }
  if (!Array.isArray(value.removed_index_lines) || value.removed_index_lines.length === 0) {
    errors.push(`${path}.removed_index_lines must be a non-empty array`);
  } else {
    let previousLine = 0;
    for (const [index, removed] of value.removed_index_lines.entries()) {
      const removedPath = `${path}.removed_index_lines[${index}]`;
      if (!hasExactKeys(
        removed,
        historicalRemovedIndexLineKeys,
        historicalRemovedIndexLineKeys,
        removedPath,
        errors,
      )) continue;
      if (!Number.isSafeInteger(removed.raw_line) || removed.raw_line < 1) {
        errors.push(`${removedPath}.raw_line must be a positive integer`);
      } else if (removed.raw_line <= previousLine) {
        errors.push(`${removedPath}.raw_line must be strictly increasing`);
      } else {
        previousLine = removed.raw_line;
      }
      if (typeof removed.text !== "string" || !gitIndexLinePattern.test(removed.text)) {
        errors.push(`${removedPath}.text must be an exact Git index line`);
      }
    }
  }
  if (!sameStrings(value.excluded_metadata, historicalMetadata)) {
    errors.push(`${path}.excluded_metadata must exclude pull-request metadata`);
  }
  if (value.content_redaction !== "not_required") {
    errors.push(`${path}.content_redaction must be not_required`);
  }
  return errors.length === startErrorCount ? value.removed_index_lines : null;
}

export function validateHistoricalAssets(assets) {
  const { schema, rubric, corpus, provenance, selectionPolicy } = assets ?? {};
  const errors = [
    ...validateSchema(schema),
    ...validateRubric(rubric, schema),
    ...validateCorpus(corpus, rubric, schema),
  ];
  if (!isHistoricalCorpus(corpus)) {
    errors.push("historical assets require corpus_version 2");
    return errors;
  }
  if (!hasExactKeys(
    provenance,
    historicalProvenanceKeys,
    historicalProvenanceKeys,
    "historical provenance",
    errors,
  )) return errors;
  validateVersion(provenance, "provenance_version", historicalCorpusVersion, "historical provenance", errors);
  validateVersion(provenance, "corpus_version", historicalCorpusVersion, "historical provenance", errors);
  if (!gitShaPattern.test(provenance.cutoff_commit_sha ?? "")) {
    errors.push("historical provenance.cutoff_commit_sha must be a full lowercase Git SHA");
  }
  if (!sha256Pattern.test(provenance.selection_policy_sha256 ?? "")) {
    errors.push("historical provenance.selection_policy_sha256 must be a lowercase SHA-256 digest");
  }
  const hasPolicy = nonEmptyBytes(selectionPolicy, "historical selection policy", errors);
  if (hasPolicy && provenance.selection_policy_sha256 !== sha256Bytes(selectionPolicy)) {
    errors.push("historical provenance.selection_policy_sha256 must match the raw selection policy bytes");
  }
  if (hasPolicy) {
    const policyText = typeof selectionPolicy === "string"
      ? selectionPolicy
      : Buffer.from(selectionPolicy.buffer, selectionPolicy.byteOffset, selectionPolicy.byteLength).toString("utf8");
    if (!policyText.includes(provenance.cutoff_commit_sha)) {
      errors.push("historical selection policy must name the provenance cutoff commit");
    }
  }
  if (corpus.provenance_sha256 !== sha256Json(provenance)) {
    errors.push("corpus.provenance_sha256 must match the canonical historical provenance");
  }
  if (!Array.isArray(provenance.cases) || provenance.cases.length === 0) {
    errors.push("historical provenance.cases must be a non-empty array");
    return errors;
  }
  const corpusById = new Map((corpus.cases ?? []).map((entry) => [entry.id, entry]));
  const provenanceIds = new Set();
  const pullRequestCounts = new Map();
  for (const [index, entry] of provenance.cases.entries()) {
    const path = `historical provenance.cases[${index}]`;
    if (!hasExactKeys(entry, historicalProvenanceCaseKeys, historicalProvenanceCaseKeys, path, errors)) continue;
    if (!/^RF-[0-9A-F]{12}$/.test(entry.case_id ?? "")) {
      errors.push(`${path}.case_id must match RF-XXXXXXXXXXXX`);
    } else if (provenanceIds.has(entry.case_id)) {
      errors.push(`${path}.case_id duplicates ${entry.case_id}`);
    } else {
      provenanceIds.add(entry.case_id);
    }
    if (!Number.isSafeInteger(entry.pull_request) || entry.pull_request < 1) {
      errors.push(`${path}.pull_request must be a positive integer`);
    } else {
      pullRequestCounts.set(entry.pull_request, (pullRequestCounts.get(entry.pull_request) ?? 0) + 1);
    }
    for (const field of ["merge_commit_sha", "source_commit_sha", "source_parent_sha"]) {
      if (!gitShaPattern.test(entry[field] ?? "")) {
        errors.push(`${path}.${field} must be a full lowercase Git SHA`);
      }
    }
    const pathspec = uniqueStringArray(entry.pathspec, `${path}.pathspec`, errors);
    for (const item of pathspec) {
      if (item.startsWith("/") || item.includes("\0") || item.split("/").includes("..")) {
        errors.push(`${path}.pathspec must contain repository-relative paths`);
      }
    }
    if (!["complete_source_commit", "path_family"].includes(entry.selection_mode)) {
      errors.push(`${path}.selection_mode is invalid`);
    }
    for (const field of ["source_diff_sha256", "model_diff_sha256"]) {
      if (!sha256Pattern.test(entry[field] ?? "")) {
        errors.push(`${path}.${field} must be a lowercase SHA-256 digest`);
      }
    }
    const removedIndexLines = validateHistoricalSanitization(entry.sanitization, `${path}.sanitization`, errors);
    nonEmptyString(entry.answer_rationale, `${path}.answer_rationale`, errors);
    const corpusCase = corpusById.get(entry.case_id);
    if (!corpusCase) {
      errors.push(`${path}.case_id has no matching corpus case: ${entry.case_id}`);
    } else {
      if (entry.model_diff_sha256 !== sha256Bytes(corpusCase.diff)) {
        errors.push(`${path}.model_diff_sha256 must match its corpus case diff`);
      }
      if (removedIndexLines) {
        try {
          const sourceDiff = restoreHistoricalDiff(corpusCase.diff, removedIndexLines);
          if (entry.source_diff_sha256 !== sha256Bytes(sourceDiff)) {
            errors.push(`${path}.source_diff_sha256 must match the deterministically reconstructed raw diff`);
          }
          if (sanitizeHistoricalDiff(sourceDiff) !== corpusCase.diff) {
            errors.push(`${path}.sanitization must reconstruct a source diff that sanitizes to its corpus diff`);
          }
        } catch (error) {
          errors.push(`${path}.sanitization.removed_index_lines cannot reconstruct the raw source diff`);
        }
      }
    }
  }
  for (const [pullRequest, count] of pullRequestCounts) {
    if (count > 2) errors.push(`historical provenance has more than two cases for PR ${pullRequest}`);
  }
  for (const caseId of corpusById.keys()) {
    if (!provenanceIds.has(caseId)) {
      errors.push(`historical provenance is missing corpus case: ${caseId}`);
    }
  }
  return errors;
}

export function validateAssets(assets) {
  if (isHistoricalCorpus(assets?.corpus)) return validateHistoricalAssets(assets);
  const { schema, rubric, corpus } = assets ?? {};
  return [
    ...validateSchema(schema),
    ...validateRubric(rubric, schema),
    ...validateCorpus(corpus, rubric, schema),
  ];
}

function requireValidAssets(assets) {
  const errors = validateAssets(assets);
  if (errors.length > 0) throw new Error(errors.join("\n"));
}

function promptPayload({ schema, rubric, corpus }) {
  return {
    schema_version: schema.schema_version,
    rubric_version: rubric.rubric_version,
    prompt_version: rubric.prompt_version,
    rubric_sha256: sha256Json(rubric),
    corpus_sha256: sha256Json(corpus),
    instructions: [
      "This is an offline, report-only pure-refactoring evaluation. It cannot approve, merge, or change a pull request.",
      "Classify each supplied synthetic diff as proven only when its bounded evidence establishes a structural-only change. Otherwise classify it as not_proven.",
      "Hard-exclusion surfaces and uncertainty must be not_proven. Do not infer missing context, use tools, or rely on unstated repository behavior.",
      "Each independent evaluator returns exactly one run object matching single_run_keys and case_keys.",
      "The recorder combines at least three complete run objects into the result container matching result_container_keys, using one exact pinned non-empty model_id for every run.",
      "Do not include secrets, customer data, or claims that this result is a safety proof.",
    ],
    response_contract: {
      evaluator_output: "exactly_one_run_object",
      recorder_output: "combine_at_least_three_run_objects",
      result_container_keys: schema.result_format.top_level_keys,
      single_run_keys: schema.result_format.run_keys,
      case_keys: schema.result_format.case_keys,
      run_status: "complete",
      classifications,
      reason_codes: rubric.reason_codes.map(({ code, classification, description }) => ({
        code,
        classification,
        description,
      })),
    },
    cases: corpus.cases.map(({ id, diff }) => ({ case_id: id, diff })),
  };
}

export function buildPromptBundle(assets) {
  requireValidAssets(assets);
  const payload = promptPayload(assets);
  return { ...payload, bundle_sha256: sha256Json(payload) };
}

function validateBundle(bundle, assets, errors) {
  if (!hasExactKeys(bundle, bundleKeys, bundleKeys, "prompt bundle", errors)) return;
  const expected = buildPromptBundle(assets);
  if (bundle.bundle_sha256 !== sha256Json(Object.fromEntries(
    Object.entries(bundle).filter(([key]) => key !== "bundle_sha256"),
  ))) {
    errors.push("prompt bundle.bundle_sha256 does not match its input");
  }
  if (bundle.bundle_sha256 !== expected.bundle_sha256) {
    errors.push("prompt bundle must match the checked-in schema, rubric, and corpus");
  }
}

export function validateResults(results, assets) {
  const errors = validateAssets(assets ?? {});
  if (errors.length > 0) return errors;
  const { schema, rubric, corpus, bundle } = assets;
  validateBundle(bundle, assets, errors);
  if (!isRecord(bundle)) return errors;
  if (!hasExactKeys(results, resultTopLevelKeys, resultTopLevelKeys, "results", errors)) return errors;
  for (const name of ["schema_version", "rubric_version", "prompt_version"]) {
    if (results[name] !== bundle[name]) {
      errors.push(`results.${name} must match the prompt bundle`);
    }
  }
  if (results.bundle_sha256 !== bundle.bundle_sha256) {
    errors.push("results.bundle_sha256 must match the prompt bundle");
  }
  if (!Array.isArray(results.runs) || results.runs.length < rubric.minimum_runs) {
    errors.push(`results.runs must contain at least ${rubric.minimum_runs} runs`);
    return errors;
  }
  const expectedCaseIds = new Set(corpus.cases.map((entry) => entry.id));
  const runIds = new Set();
  let pinnedModelId = null;
  const statuses = new Set(schema.result_format.run_statuses);
  const reasons = reasonMap(rubric);
  for (const [runIndex, run] of results.runs.entries()) {
    const path = `results.runs[${runIndex}]`;
    if (!hasExactKeys(run, resultRunKeys, resultRunKeys, path, errors)) continue;
    if (!nonEmptyString(run.run_id, `${path}.run_id`, errors)) {
      // Continue validating independent fields, but no duplicate comparison.
    } else if (runIds.has(run.run_id)) {
      errors.push(`${path}.run_id duplicates ${run.run_id}`);
    } else {
      runIds.add(run.run_id);
    }
    if (nonEmptyString(run.model_id, `${path}.model_id`, errors)) {
      if (pinnedModelId === null) {
        pinnedModelId = run.model_id;
      } else if (run.model_id !== pinnedModelId) {
        errors.push(`${path}.model_id must match the first complete run`);
      }
    }
    if (!statuses.has(run.status)) {
      errors.push(`${path}.status is invalid`);
    } else if (run.status !== "complete") {
      errors.push(`${path}.status must be complete`);
    }
    if (!Array.isArray(run.cases)) {
      errors.push(`${path}.cases must be an array`);
      continue;
    }
    const receivedCaseIds = new Set();
    for (const [caseIndex, decision] of run.cases.entries()) {
      const decisionPath = `${path}.cases[${caseIndex}]`;
      if (!hasExactKeys(decision, resultCaseKeys, resultCaseKeys, decisionPath, errors)) continue;
      if (!nonEmptyString(decision.case_id, `${decisionPath}.case_id`, errors)) continue;
      if (receivedCaseIds.has(decision.case_id)) {
        errors.push(`${decisionPath} duplicates case_id: ${decision.case_id}`);
      } else {
        receivedCaseIds.add(decision.case_id);
      }
      if (!expectedCaseIds.has(decision.case_id)) {
        errors.push(`${decisionPath} has extra case result: ${decision.case_id}`);
      }
      validateDecision(
        decision.classification,
        decision.reason_codes,
        reasons,
        decisionPath,
        errors,
      );
      nonEmptyString(decision.evidence, `${decisionPath}.evidence`, errors);
    }
    for (const caseId of expectedCaseIds) {
      if (!receivedCaseIds.has(caseId)) {
        errors.push(`${path} is missing case result: ${caseId}`);
      }
    }
  }
  return errors;
}

function decisionMaps(results) {
  return results.runs.map((run) => new Map(run.cases.map((decision) => [decision.case_id, decision])));
}

function metric(decisions, caseIds, eligibleDecisions) {
  return {
    decisions,
    eligible_decisions: eligibleDecisions,
    rate: eligibleDecisions === 0 ? 0 : decisions / eligibleDecisions,
    case_ids: [...caseIds].sort(),
  };
}

function normalizedReasonCodeSet(decision) {
  return JSON.stringify([...new Set(decision.reason_codes)].sort());
}

export function scoreResults(results, assets) {
  const errors = validateResults(results, assets);
  if (errors.length > 0) throw new Error(errors.join("\n"));
  const { schema, rubric, corpus, bundle } = assets;
  const decisionsByRun = decisionMaps(results);
  const runCount = results.runs.length;
  let falseSafe = 0;
  let falseReject = 0;
  let dangerousFalseSafe = 0;
  let adversarialFalseSafe = 0;
  const falseSafeCaseIds = new Set();
  const falseRejectCaseIds = new Set();
  const dangerousFalseSafeCaseIds = new Set();
  const adversarialFalseSafeCaseIds = new Set();
  let expectedReasonMisses = 0;
  const expectedReasonMissCaseIds = new Set();
  const unstableClassificationCaseIds = [];
  const unstableReasonCodeSetCaseIds = [];
  let unanimousClassificationCases = 0;
  let unanimousReasonCodeSetCases = 0;
  let classificationPairwiseAgreements = 0;
  let classificationPairwiseComparisons = 0;
  let reasonCodeSetPairwiseAgreements = 0;
  let reasonCodeSetPairwiseComparisons = 0;

  for (const entry of corpus.cases) {
    const decisions = decisionsByRun.map((run) => run.get(entry.id));
    const actualClassifications = decisions.map((decision) => decision.classification);
    const actualReasonCodeSets = decisions.map(normalizedReasonCodeSet);
    if (new Set(actualClassifications).size === 1) unanimousClassificationCases += 1;
    else unstableClassificationCaseIds.push(entry.id);
    if (new Set(actualReasonCodeSets).size === 1) unanimousReasonCodeSetCases += 1;
    else unstableReasonCodeSetCaseIds.push(entry.id);
    for (let left = 0; left < decisions.length; left += 1) {
      for (let right = left + 1; right < decisions.length; right += 1) {
        classificationPairwiseComparisons += 1;
        if (actualClassifications[left] === actualClassifications[right]) {
          classificationPairwiseAgreements += 1;
        }
        reasonCodeSetPairwiseComparisons += 1;
        if (actualReasonCodeSets[left] === actualReasonCodeSets[right]) {
          reasonCodeSetPairwiseAgreements += 1;
        }
      }
    }
    for (const [index, classification] of actualClassifications.entries()) {
      if (!decisions[index].reason_codes.some((code) => entry.expected_reason_codes.includes(code))) {
        expectedReasonMisses += 1;
        expectedReasonMissCaseIds.add(entry.id);
      }
      if (entry.expected_label === "not_proven" && classification === "proven") {
        falseSafe += 1;
        falseSafeCaseIds.add(entry.id);
        if (entry.dangerous) {
          dangerousFalseSafe += 1;
          dangerousFalseSafeCaseIds.add(entry.id);
        }
        if (entry.kind === "adversarial") {
          adversarialFalseSafe += 1;
          adversarialFalseSafeCaseIds.add(entry.id);
        }
      }
      if (entry.expected_label === "proven" && classification === "not_proven") {
        falseReject += 1;
        falseRejectCaseIds.add(entry.id);
      }
    }
  }

  const eligible = (predicate) => corpus.cases.filter(predicate).length * runCount;
  const expectedNotProvenDecisions = eligible((entry) => entry.expected_label === "not_proven");
  const expectedProvenDecisions = eligible((entry) => entry.expected_label === "proven");
  const dangerousExpectedNotProvenDecisions = eligible(
    (entry) => entry.dangerous && entry.expected_label === "not_proven",
  );
  const adversarialExpectedNotProvenDecisions = eligible(
    (entry) => entry.kind === "adversarial" && entry.expected_label === "not_proven",
  );

  return {
    authority: "report_only",
    schema_version: schema.schema_version,
    rubric_version: rubric.rubric_version,
    prompt_version: rubric.prompt_version,
    bundle_sha256: bundle.bundle_sha256,
    metrics: {
      runs: runCount,
      cases: corpus.cases.length,
      decisions: runCount * corpus.cases.length,
      false_safe: metric(falseSafe, falseSafeCaseIds, expectedNotProvenDecisions),
      false_reject: metric(falseReject, falseRejectCaseIds, expectedProvenDecisions),
      dangerous_false_safe: metric(
        dangerousFalseSafe,
        dangerousFalseSafeCaseIds,
        dangerousExpectedNotProvenDecisions,
      ),
      adversarial_false_safe: metric(
        adversarialFalseSafe,
        adversarialFalseSafeCaseIds,
        adversarialExpectedNotProvenDecisions,
      ),
      expected_reason_misses: metric(
        expectedReasonMisses,
        expectedReasonMissCaseIds,
        runCount * corpus.cases.length,
      ),
      classification_agreement: {
        unanimous_cases: unanimousClassificationCases,
        total_cases: corpus.cases.length,
        unstable_case_ids: unstableClassificationCaseIds,
        pairwise_agreements: classificationPairwiseAgreements,
        pairwise_comparisons: classificationPairwiseComparisons,
      },
      reason_code_set_agreement: {
        unanimous_cases: unanimousReasonCodeSetCases,
        total_cases: corpus.cases.length,
        unstable_case_ids: unstableReasonCodeSetCaseIds,
        pairwise_agreements: reasonCodeSetPairwiseAgreements,
        pairwise_comparisons: reasonCodeSetPairwiseComparisons,
      },
    },
  };
}

function errorMetricSummary(value) {
  return {
    decisions: value.decisions,
    eligible_decisions: value.eligible_decisions,
    rate: value.rate,
  };
}

function agreementMetricSummary(value) {
  return {
    unanimous_cases: value.unanimous_cases,
    total_cases: value.total_cases,
    unanimous_rate: value.total_cases === 0 ? 0 : value.unanimous_cases / value.total_cases,
    pairwise_agreements: value.pairwise_agreements,
    pairwise_comparisons: value.pairwise_comparisons,
    pairwise_rate: value.pairwise_comparisons === 0
      ? 0
      : value.pairwise_agreements / value.pairwise_comparisons,
  };
}

function comparisonSummary(score) {
  const { metrics } = score;
  return {
    runs: metrics.runs,
    cases: metrics.cases,
    decisions: metrics.decisions,
    false_safe: errorMetricSummary(metrics.false_safe),
    false_reject: errorMetricSummary(metrics.false_reject),
    dangerous_false_safe: errorMetricSummary(metrics.dangerous_false_safe),
    adversarial_false_safe: errorMetricSummary(metrics.adversarial_false_safe),
    expected_reason_misses: errorMetricSummary(metrics.expected_reason_misses),
    classification_agreement: agreementMetricSummary(metrics.classification_agreement),
    reason_code_set_agreement: agreementMetricSummary(metrics.reason_code_set_agreement),
  };
}

function numericDeltas(baseline, historical) {
  return Object.fromEntries(
    Object.keys(baseline).map((key) => [key, historical[key] - baseline[key]]),
  );
}

function comparisonDeltas(baseline, historical) {
  return {
    runs: historical.runs - baseline.runs,
    cases: historical.cases - baseline.cases,
    decisions: historical.decisions - baseline.decisions,
    false_safe: numericDeltas(baseline.false_safe, historical.false_safe),
    false_reject: numericDeltas(baseline.false_reject, historical.false_reject),
    dangerous_false_safe: numericDeltas(
      baseline.dangerous_false_safe,
      historical.dangerous_false_safe,
    ),
    adversarial_false_safe: numericDeltas(
      baseline.adversarial_false_safe,
      historical.adversarial_false_safe,
    ),
    expected_reason_misses: numericDeltas(
      baseline.expected_reason_misses,
      historical.expected_reason_misses,
    ),
    classification_agreement: numericDeltas(
      baseline.classification_agreement,
      historical.classification_agreement,
    ),
    reason_code_set_agreement: numericDeltas(
      baseline.reason_code_set_agreement,
      historical.reason_code_set_agreement,
    ),
  };
}

export function compareResults({
  baselineResults,
  historicalResults,
  baselineAssets,
  historicalAssets,
}) {
  if (isHistoricalCorpus(baselineAssets?.corpus)) {
    throw new Error("comparison baseline must use corpus version 1");
  }
  if (historicalAssets?.corpus?.corpus_version !== historicalCorpusVersion) {
    throw new Error("comparison historical input must use corpus version 2");
  }
  const baselineErrors = validateResults(baselineResults, baselineAssets);
  if (baselineErrors.length > 0) throw new Error(baselineErrors.join("\n"));
  const historicalErrors = validateResults(historicalResults, historicalAssets);
  if (historicalErrors.length > 0) throw new Error(historicalErrors.join("\n"));
  const baselineRubricSha256 = sha256Json(baselineAssets.rubric);
  const historicalRubricSha256 = sha256Json(historicalAssets.rubric);
  if (baselineRubricSha256 !== historicalRubricSha256) {
    throw new Error("comparison results must use the same rubric");
  }
  const baselineModelId = baselineResults.runs[0].model_id;
  const historicalModelId = historicalResults.runs[0].model_id;
  if (baselineModelId !== historicalModelId) {
    throw new Error("comparison results must use the same model_id");
  }
  const baseline = comparisonSummary(scoreResults(baselineResults, baselineAssets));
  const historical = comparisonSummary(scoreResults(historicalResults, historicalAssets));
  return {
    authority: "report_only",
    comparison: "unpaired_descriptive",
    model_id: baselineModelId,
    rubric_sha256: baselineRubricSha256,
    baseline: {
      corpus_version: 1,
      bundle_sha256: baselineAssets.bundle.bundle_sha256,
      metrics: baseline,
    },
    historical: {
      corpus_version: historicalCorpusVersion,
      bundle_sha256: historicalAssets.bundle.bundle_sha256,
      metrics: historical,
    },
    deltas: comparisonDeltas(baseline, historical),
  };
}

function loadJson(repoRoot, relativePath) {
  return JSON.parse(readFileSync(resolve(repoRoot, relativePath), "utf8"));
}

function loadAssets(repoRoot = defaultRepoRoot, corpusVersion = 1) {
  const paths = assetPaths[corpusVersion];
  if (!paths) throw new Error(`unsupported corpus version: ${corpusVersion}`);
  const assets = {
    schema: loadJson(repoRoot, paths.schema),
    rubric: loadJson(repoRoot, paths.rubric),
    corpus: loadJson(repoRoot, paths.corpus),
  };
  if (paths.provenance) {
    assets.provenance = loadJson(repoRoot, paths.provenance);
    assets.selectionPolicy = readFileSync(resolve(repoRoot, paths.selectionPolicy));
  }
  return assets;
}

function printErrors(errors) {
  for (const error of errors) console.error(`pure-refactoring evaluator: ${error}`);
}

function usage() {
  console.error("usage: node scripts/pure-refactoring-evaluator.mjs check [--corpus-version 1|2] | prompt [--corpus-version 1|2] | score [--corpus-version 1|2] --results FILE | compare --baseline-results FILE --historical-results FILE");
}

function parseCorpusVersion(args) {
  let corpusVersion = 1;
  let seen = false;
  const remaining = [];
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] !== "--corpus-version") {
      remaining.push(args[index]);
      continue;
    }
    const value = args[index + 1];
    if (seen || !["1", "2"].includes(value)) return null;
    seen = true;
    corpusVersion = Number(value);
    index += 1;
  }
  return { corpusVersion, remaining };
}

function parseFileOptions(args, names) {
  if (args.length !== names.length * 2) return null;
  const values = {};
  for (let index = 0; index < args.length; index += 2) {
    const name = args[index];
    const value = args[index + 1];
    if (!names.includes(name) || !value || Object.hasOwn(values, name)) return null;
    values[name] = value;
  }
  return names.every((name) => Object.hasOwn(values, name)) ? values : null;
}

function readResults(path) {
  return JSON.parse(readFileSync(resolve(process.cwd(), path), "utf8"));
}

function validateLoadedAssets(assets) {
  const errors = validateAssets(assets);
  if (errors.length > 0) {
    printErrors(errors);
    process.exitCode = 2;
    return false;
  }
  return true;
}

function main(args) {
  const command = args[0];
  if (!command || !["check", "prompt", "score", "compare"].includes(command)) {
    usage();
    process.exitCode = 2;
    return;
  }
  if (command === "compare") {
    const files = parseFileOptions(args.slice(1), ["--baseline-results", "--historical-results"]);
    if (!files) {
      usage();
      process.exitCode = 2;
      return;
    }
    const baselineAssets = loadAssets(defaultRepoRoot, 1);
    const historicalAssets = loadAssets(defaultRepoRoot, 2);
    if (!validateLoadedAssets(baselineAssets) || !validateLoadedAssets(historicalAssets)) return;
    const baseline = { ...baselineAssets, bundle: buildPromptBundle(baselineAssets) };
    const historical = { ...historicalAssets, bundle: buildPromptBundle(historicalAssets) };
    let baselineResults;
    let historicalResults;
    try {
      baselineResults = readResults(files["--baseline-results"]);
      historicalResults = readResults(files["--historical-results"]);
    } catch (error) {
      console.error(`pure-refactoring evaluator: cannot read results: ${error instanceof Error ? error.message : String(error)}`);
      process.exitCode = 2;
      return;
    }
    console.log(JSON.stringify(compareResults({
      baselineResults,
      historicalResults,
      baselineAssets: baseline,
      historicalAssets: historical,
    }), null, 2));
    return;
  }
  const parsed = parseCorpusVersion(args.slice(1));
  if (!parsed) {
    usage();
    process.exitCode = 2;
    return;
  }
  if ((command === "check" || command === "prompt") && parsed.remaining.length !== 0) {
    usage();
    process.exitCode = 2;
    return;
  }
  const files = command === "score" ? parseFileOptions(parsed.remaining, ["--results"]) : null;
  if (command === "score" && !files) {
    usage();
    process.exitCode = 2;
    return;
  }
  const assets = loadAssets(defaultRepoRoot, parsed.corpusVersion);
  if (!validateLoadedAssets(assets)) return;
  const bundle = buildPromptBundle(assets);
  if (command === "check") {
    console.log(`pure-refactoring evaluator: OK (${assets.corpus.cases.length} cases, ${assets.rubric.minimum_runs} required runs)`);
    return;
  }
  if (command === "prompt") {
    console.log(JSON.stringify(bundle, null, 2));
    return;
  }
  let results;
  try {
    results = readResults(files["--results"]);
  } catch (error) {
    console.error(`pure-refactoring evaluator: cannot read results: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 2;
    return;
  }
  const resultErrors = validateResults(results, { ...assets, bundle });
  if (resultErrors.length > 0) {
    printErrors(resultErrors);
    process.exitCode = 2;
    return;
  }
  console.log(JSON.stringify(scoreResults(results, { ...assets, bundle }), null, 2));
}

if (process.argv[1] && resolve(process.argv[1]) === scriptPath) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(`pure-refactoring evaluator: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 2;
  }
}
