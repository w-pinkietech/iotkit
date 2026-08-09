#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const defaultRepoRoot = resolve(dirname(scriptPath), "..");
const assetPaths = {
  schema: "review/pure-refactoring/schema.v1.json",
  rubric: "review/pure-refactoring/rubric.v1.json",
  corpus: "review/pure-refactoring/corpus.v1.json",
};
const supportedVersion = 1;
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

export function opaqueCaseId(diff) {
  if (typeof diff !== "string") throw new TypeError("opaque case IDs require a string diff");
  return `RF-${createHash("sha256").update(diff).digest("hex").slice(0, 12).toUpperCase()}`;
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
  const keys = ["schema_version", "rubric_version", "prompt_version", "cases"];
  if (!hasExactKeys(corpus, keys, keys, "corpus", errors)) return errors;
  validateMatchingVersions(corpus, schema, "corpus", errors);
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
    if (nonEmptyString(entry.diff, `${path}.diff`, errors) && entry.id !== opaqueCaseId(entry.diff)) {
      errors.push(`${path}.id must equal the opaque SHA-256 ID derived from its diff`);
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

function assetErrors({ schema, rubric, corpus }) {
  return [
    ...validateSchema(schema),
    ...validateRubric(rubric, schema),
    ...validateCorpus(corpus, rubric, schema),
  ];
}

function requireValidAssets(assets) {
  const errors = assetErrors(assets);
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
  const errors = assetErrors(assets ?? {});
  if (errors.length > 0) return errors;
  const { schema, rubric, corpus, bundle } = assets;
  validateBundle(bundle, { schema, rubric, corpus }, errors);
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

function loadJson(repoRoot, relativePath) {
  return JSON.parse(readFileSync(resolve(repoRoot, relativePath), "utf8"));
}

function loadAssets(repoRoot = defaultRepoRoot) {
  return {
    schema: loadJson(repoRoot, assetPaths.schema),
    rubric: loadJson(repoRoot, assetPaths.rubric),
    corpus: loadJson(repoRoot, assetPaths.corpus),
  };
}

function printErrors(errors) {
  for (const error of errors) console.error(`pure-refactoring evaluator: ${error}`);
}

function usage() {
  console.error("usage: node scripts/pure-refactoring-evaluator.mjs check | prompt | score --results FILE");
}

function main(args) {
  const command = args[0];
  if (!command || (command === "check" && args.length !== 1) || (command === "prompt" && args.length !== 1)) {
    usage();
    process.exitCode = 2;
    return;
  }
  if (command !== "check" && command !== "prompt" && command !== "score") {
    usage();
    process.exitCode = 2;
    return;
  }
  if (command === "score" && (args.length !== 3 || args[1] !== "--results" || !args[2])) {
    usage();
    process.exitCode = 2;
    return;
  }
  const assets = loadAssets();
  const errors = assetErrors(assets);
  if (errors.length > 0) {
    printErrors(errors);
    process.exitCode = 2;
    return;
  }
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
    results = JSON.parse(readFileSync(resolve(process.cwd(), args[2]), "utf8"));
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
