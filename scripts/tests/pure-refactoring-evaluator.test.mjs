import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  buildPromptBundle,
  opaqueCaseId,
  scoreResults,
  validateCorpus,
  validateResults,
  validateRubric,
  validateSchema,
} from "../pure-refactoring-evaluator.mjs";

const root = new URL("../../", import.meta.url);
const readJson = (path) => JSON.parse(readFileSync(new URL(path, root), "utf8"));
const schema = readJson("review/pure-refactoring/schema.v1.json");
const rubric = readJson("review/pure-refactoring/rubric.v1.json");
const corpus = readJson("review/pure-refactoring/corpus.v1.json");
const bundle = buildPromptBundle({ schema, rubric, corpus });
const context = { schema, rubric, corpus, bundle };
const script = new URL("../pure-refactoring-evaluator.mjs", import.meta.url);
const capturedResultsPath = "review/pure-refactoring/evaluations/issue-212-v1-titlefree-gpt-5.6-sol-high.json";
const capturedReportPath = "review/pure-refactoring/reports/issue-212-v1-titlefree.md";

function validResults() {
  return {
    schema_version: schema.schema_version,
    rubric_version: rubric.rubric_version,
    prompt_version: rubric.prompt_version,
    bundle_sha256: bundle.bundle_sha256,
    runs: ["run-001", "run-002", "run-003"].map((run_id) => ({
      run_id,
      model_id: "gpt-5.6-sol/high",
      status: "complete",
      cases: corpus.cases.map((entry) => ({
        case_id: entry.id,
        classification: entry.expected_label,
        reason_codes: [...entry.expected_reason_codes],
        evidence: `Synthetic diff review for ${entry.id}.`,
      })),
    })),
  };
}

function resultErrors(mutate) {
  const results = validResults();
  mutate(results);
  return validateResults(results, context).join("\n");
}

function caseWithReason(reasonCode) {
  const entry = corpus.cases.find((candidate) => candidate.expected_reason_codes.includes(reasonCode));
  assert.ok(entry, `missing corpus case for ${reasonCode}`);
  return entry;
}

function decisionFor(results, runIndex, caseId) {
  const decision = results.runs[runIndex].cases.find((candidate) => candidate.case_id === caseId);
  assert.ok(decision, `missing result decision for ${caseId}`);
  return decision;
}

function reportScoreBlock(source) {
  const blocks = [
    ...source.matchAll(/^```json[ \t]*\r?\n([\s\S]*?)^```[ \t]*$/gm),
  ];
  assert.equal(blocks.length, 1, "the report must contain exactly one json fenced block");
  return JSON.parse(blocks[0][1]);
}

test("the checked-in schema, rubric, corpus, and coverage invariants are valid", () => {
  assert.deepEqual(validateSchema(schema), []);
  assert.deepEqual(validateRubric(rubric, schema), []);
  assert.deepEqual(validateCorpus(corpus, rubric, schema), []);
  assert.ok(
    corpus.cases.filter(
      (entry) =>
        entry.kind === "positive" && !entry.dangerous && entry.expected_label === "proven",
    ).length >= 4,
    "the corpus keeps at least four safe positives",
  );
  assert.deepEqual(
    corpus.cases.map((entry) => entry.id),
    [...corpus.cases.map((entry) => entry.id)].sort(),
    "opaque case IDs are presented in deterministic lexicographic order",
  );
  for (const entry of corpus.cases) {
    assert.match(entry.id, /^RF-[0-9A-F]{12}$/);
    assert.equal(entry.id, opaqueCaseId(entry.diff));
  }
  const midpoint = corpus.cases.length / 2;
  for (const half of [corpus.cases.slice(0, midpoint), corpus.cases.slice(midpoint)]) {
    assert.deepEqual(
      new Set(half.map((entry) => entry.expected_label)),
      new Set(["proven", "not_proven"]),
      "each opaque-ID half retains both labels without using labels to order cases",
    );
  }
  assert.deepEqual(
    new Set(corpus.cases.flatMap((entry) => entry.risk_categories)),
    new Set(rubric.required_risk_categories),
    "the corpus covers every required hard-risk category",
  );
  assert.deepEqual(rubric.required_risk_categories, [
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
  ]);
  assert.ok(corpus.cases.some((entry) => entry.kind === "negative"));
  assert.ok(corpus.cases.some((entry) => entry.kind === "adversarial"));
  assert.ok(
    corpus.cases.every(
      (entry) => entry.expected_label === "proven" || entry.expected_label === "not_proven",
    ),
  );
});

test("the corpus fails closed when a hard-risk category loses its adversarial case", () => {
  const broken = structuredClone(corpus);
  broken.cases = broken.cases.filter(
    (entry) => !entry.risk_categories.includes("generated_artifacts"),
  );
  assert.match(
    validateCorpus(broken, rubric, schema).join("\n"),
    /generated_artifacts/,
  );
});

test("the corpus fails closed for opaque-ID drift and non-opaque ordering", () => {
  const mismatchedId = structuredClone(corpus);
  mismatchedId.cases[0].id = "RF-000000000000";
  assert.match(
    validateCorpus(mismatchedId, rubric, schema).join("\n"),
    /opaque SHA-256 ID/,
  );

  const unsorted = structuredClone(corpus);
  [unsorted.cases[0], unsorted.cases[1]] = [unsorted.cases[1], unsorted.cases[0]];
  assert.match(
    validateCorpus(unsorted, rubric, schema).join("\n"),
    /sorted lexicographically by opaque ID/,
  );
});

test("the prompt bundle is deterministic, hashes its blinded input, and excludes expected labels", () => {
  const again = buildPromptBundle({ schema, rubric, corpus });
  assert.deepEqual(again, bundle);
  const serialized = JSON.stringify(bundle);
  assert.doesNotMatch(serialized, /expected_label|expected_reason_codes|"dangerous"/);
  assert.ok(/^[a-f0-9]{64}$/.test(bundle.bundle_sha256));
  assert.ok(/^[a-f0-9]{64}$/.test(bundle.corpus_sha256));
  assert.equal(bundle.response_contract.evaluator_output, "exactly_one_run_object");
  assert.equal(
    bundle.response_contract.recorder_output,
    "combine_at_least_three_run_objects",
  );
  assert.ok(
    bundle.instructions.includes(
      "Each independent evaluator returns exactly one run object matching single_run_keys and case_keys.",
    ),
  );
  assert.deepEqual(
    bundle.cases.map((entry) => Object.keys(entry).sort()),
    corpus.cases.map(() => ["case_id", "diff"]),
  );
  assert.deepEqual(bundle.cases.map((entry) => entry.case_id), corpus.cases.map((entry) => entry.id));
  for (const entry of bundle.cases) {
    assert.equal(entry.case_id, opaqueCaseId(entry.diff));
  }
  for (const entry of corpus.cases) {
    assert.equal(serialized.includes(entry.title), false, `prompt must not expose corpus title: ${entry.id}`);
  }
});

test("the prompt CLI remains deterministic and blind", () => {
  const first = spawnSync(process.execPath, [script.pathname, "prompt"], {
    encoding: "utf8",
  });
  const second = spawnSync(process.execPath, [script.pathname, "prompt"], {
    encoding: "utf8",
  });
  assert.equal(first.status, 0, first.stderr);
  assert.equal(second.status, 0, second.stderr);
  assert.equal(first.stdout, second.stdout);
  assert.doesNotMatch(first.stdout, /expected_label|expected_reason_codes|"dangerous"/);
});

test("lightweight CI owns the evaluator check, focused test, and recorded score", () => {
  const workflow = readFileSync(new URL(".github/workflows/ci.yml", root), "utf8");
  const ownership = readFileSync(new URL(".github/verification-ownership.md", root), "utf8");
  assert.match(
    workflow,
    /name: Pure-refactoring evaluator foundation[\s\S]*?node scripts\/pure-refactoring-evaluator\.mjs check[\s\S]*?node --test scripts\/tests\/pure-refactoring-evaluator\.test\.mjs/,
  );
  assert.match(
    workflow,
    /node scripts\/pure-refactoring-evaluator\.mjs score --results review\/pure-refactoring\/evaluations\/issue-212-v1-titlefree-gpt-5\.6-sol-high\.json/,
  );
  assert.match(ownership, /\| pure-refactoring-evaluator \|/);
});

test("the recorded report embeds exactly the score of its checked-in raw result", () => {
  const report = readFileSync(new URL(capturedReportPath, root), "utf8");
  const results = readJson(capturedResultsPath);
  assert.deepEqual(reportScoreBlock(report), scoreResults(results, context));
  assert.throws(() => reportScoreBlock("no json block"), /exactly one json fenced block/);
  assert.throws(
    () => reportScoreBlock("```json\n{}\n```\n\n```json\n{}\n```"),
    /exactly one json fenced block/,
  );
  assert.throws(() => reportScoreBlock("```json\nnot json\n```"), SyntaxError);
});

test("zero score metrics expose exact eligible decisions and zero rates", () => {
  const score = scoreResults(validResults(), context);
  assert.deepEqual(score.metrics.false_safe, {
    decisions: 0,
    eligible_decisions: 36,
    rate: 0,
    case_ids: [],
  });
  assert.deepEqual(score.metrics.false_reject, {
    decisions: 0,
    eligible_decisions: 12,
    rate: 0,
    case_ids: [],
  });
  assert.deepEqual(score.metrics.dangerous_false_safe, {
    decisions: 0,
    eligible_decisions: 33,
    rate: 0,
    case_ids: [],
  });
  assert.deepEqual(score.metrics.adversarial_false_safe, {
    decisions: 0,
    eligible_decisions: 21,
    rate: 0,
    case_ids: [],
  });
  assert.deepEqual(score.metrics.expected_reason_misses, {
    decisions: 0,
    eligible_decisions: 48,
    rate: 0,
    case_ids: [],
  });
});

test("scoring reports false-safe, false-reject, dangerous/adversarial false-safe, and classification agreement separately", () => {
  const results = validResults();
  const adversarial = corpus.cases.find(
    (entry) => entry.kind === "adversarial" && entry.dangerous,
  );
  const safe = corpus.cases.find((entry) => entry.expected_label === "proven");
  assert.ok(adversarial);
  assert.ok(safe);

  const falseSafe = results.runs[0].cases.find(
    (entry) => entry.case_id === adversarial.id,
  );
  falseSafe.classification = "proven";
  falseSafe.reason_codes = ["structural_only"];

  const falseReject = results.runs[1].cases.find(
    (entry) => entry.case_id === safe.id,
  );
  falseReject.classification = "not_proven";
  falseReject.reason_codes = ["insufficient_evidence"];

  assert.deepEqual(validateResults(results, context), []);
  const score = scoreResults(results, context);
  assert.equal(score.authority, "report_only");
  assert.equal(score.metrics.false_safe.decisions, 1);
  assert.equal(score.metrics.false_reject.decisions, 1);
  assert.equal(score.metrics.dangerous_false_safe.decisions, 1);
  assert.equal(score.metrics.adversarial_false_safe.decisions, 1);
  assert.equal(score.metrics.expected_reason_misses.decisions, 2);
  assert.equal(score.metrics.false_safe.eligible_decisions, 36);
  assert.equal(score.metrics.false_safe.rate, 1 / 36);
  assert.equal(score.metrics.false_reject.eligible_decisions, 12);
  assert.equal(score.metrics.false_reject.rate, 1 / 12);
  assert.equal(score.metrics.dangerous_false_safe.eligible_decisions, 33);
  assert.equal(score.metrics.dangerous_false_safe.rate, 1 / 33);
  assert.equal(score.metrics.adversarial_false_safe.eligible_decisions, 21);
  assert.equal(score.metrics.adversarial_false_safe.rate, 1 / 21);
  assert.equal(score.metrics.expected_reason_misses.eligible_decisions, 48);
  assert.equal(score.metrics.expected_reason_misses.rate, 2 / 48);
  assert.ok(score.metrics.classification_agreement.unstable_case_ids.includes(adversarial.id));
  assert.ok(score.metrics.classification_agreement.unstable_case_ids.includes(safe.id));
  assert.ok(
    score.metrics.classification_agreement.pairwise_agreements <
      score.metrics.classification_agreement.pairwise_comparisons,
  );
});

test("reason-code variation changes exact-set agreement without changing classification agreement", () => {
  const results = validResults();
  const custody = caseWithReason("custody_data_loss");
  const target = decisionFor(results, 1, custody.id);
  target.reason_codes = ["custody_data_loss", "database_migration"];

  assert.deepEqual(validateResults(results, context), []);
  const score = scoreResults(results, context);
  assert.equal(score.metrics.classification_agreement.unanimous_cases, corpus.cases.length);
  assert.deepEqual(score.metrics.classification_agreement.unstable_case_ids, []);
  assert.deepEqual(score.metrics.reason_code_set_agreement.unstable_case_ids, [custody.id]);
  assert.ok(
    score.metrics.reason_code_set_agreement.pairwise_agreements <
      score.metrics.reason_code_set_agreement.pairwise_comparisons,
  );
  assert.equal(score.metrics.expected_reason_misses.decisions, 0);
});

test("reason-code agreement normalizes order within an exact set", () => {
  const results = validResults();
  const custody = caseWithReason("custody_data_loss");
  const codeSets = [
    ["custody_data_loss", "database_migration"],
    ["database_migration", "custody_data_loss"],
    ["custody_data_loss", "database_migration"],
  ];
  for (const [index, codes] of codeSets.entries()) {
    decisionFor(results, index, custody.id).reason_codes = codes;
  }

  assert.deepEqual(validateResults(results, context), []);
  const score = scoreResults(results, context);
  assert.equal(score.metrics.reason_code_set_agreement.unanimous_cases, corpus.cases.length);
  assert.deepEqual(score.metrics.reason_code_set_agreement.unstable_case_ids, []);
});

test("score reports missing expected reasons without rejecting another compatible controlled reason", () => {
  const results = validResults();
  const custody = caseWithReason("custody_data_loss");
  const target = decisionFor(results, 0, custody.id);
  target.reason_codes = ["database_migration"];

  assert.deepEqual(validateResults(results, context), []);
  const score = scoreResults(results, context);
  assert.deepEqual(score.metrics.expected_reason_misses, {
    decisions: 1,
    eligible_decisions: 48,
    rate: 1 / 48,
    case_ids: [custody.id],
  });
});

test("score CLI accepts a complete recorded multi-run result and prints deterministic JSON", () => {
  const directory = mkdtempSync(join(tmpdir(), "iotkit-pure-refactoring-"));
  const resultsPath = join(directory, "results.json");
  try {
    writeFileSync(resultsPath, `${JSON.stringify(validResults(), null, 2)}\n`);
    const first = spawnSync(
      process.execPath,
      [script.pathname, "score", "--results", resultsPath],
      { encoding: "utf8" },
    );
    const second = spawnSync(
      process.execPath,
      [script.pathname, "score", "--results", resultsPath],
      { encoding: "utf8" },
    );
    assert.equal(first.status, 0, first.stderr);
    assert.equal(first.stdout, second.stdout);
    assert.equal(JSON.parse(first.stdout).authority, "report_only");
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("result validation rejects malformed and unknown input before it can score", () => {
  assert.match(validateResults(null, context).join("\n"), /results must be an object/);
  assert.match(
    resultErrors((results) => {
      results.unexpected = true;
    }),
    /unknown key: unexpected/,
  );
});

test("result validation rejects non-complete and ambiguous runs", () => {
  for (const status of ["ambiguous", "incomplete"]) {
    assert.match(
      resultErrors((results) => {
        results.runs[0].status = status;
      }),
      /status must be complete/,
      status,
    );
  }
});

test("result validation requires one exact pinned model configuration across runs", () => {
  assert.match(
    resultErrors((results) => {
      results.runs[1].model_id = "gpt-5.6-sol/medium";
    }),
    /model_id must match the first complete run/,
  );
});

test("result validation rejects version and input-hash mismatches", () => {
  assert.match(
    resultErrors((results) => {
      results.schema_version = 2;
    }),
    /schema_version must match the prompt bundle/,
  );
  assert.match(
    resultErrors((results) => {
      results.bundle_sha256 = "0".repeat(64);
    }),
    /bundle_sha256 must match the prompt bundle/,
  );
});

test("result validation rejects missing, extra, and duplicate case decisions", () => {
  assert.match(
    resultErrors((results) => {
      results.runs[0].cases.pop();
    }),
    /missing case result/,
  );
  assert.match(
    resultErrors((results) => {
      results.runs[0].cases.push({
        case_id: "RF-000000000000",
        classification: "not_proven",
        reason_codes: ["insufficient_evidence"],
        evidence: "Synthetic extra case.",
      });
    }),
    /extra case result/,
  );
  assert.match(
    resultErrors((results) => {
      results.runs[0].cases[1].case_id = results.runs[0].cases[0].case_id;
    }),
    /duplicates case_id/,
  );
});

test("result validation rejects invalid classification/reason, empty evidence, and missing model identity", () => {
  assert.match(
    resultErrors((results) => {
      results.runs[0].cases[0].classification = "maybe";
    }),
    /classification is invalid/,
  );
  assert.match(
    resultErrors((results) => {
      results.runs[0].cases[0].reason_codes = ["not_a_reason"];
    }),
    /reason_codes contains unknown code/,
  );
  assert.match(
    resultErrors((results) => {
      results.runs[0].cases[0].evidence = "";
    }),
    /evidence must be a non-empty string/,
  );
  assert.match(
    resultErrors((results) => {
      delete results.runs[0].model_id;
    }),
    /missing required key: model_id/,
  );
});

test("score CLI fails closed for an invalid recorded result", () => {
  const directory = mkdtempSync(join(tmpdir(), "iotkit-pure-refactoring-"));
  const resultsPath = join(directory, "invalid-results.json");
  const invalid = validResults();
  invalid.runs[0].status = "ambiguous";
  try {
    writeFileSync(resultsPath, `${JSON.stringify(invalid, null, 2)}\n`);
    const result = spawnSync(
      process.execPath,
      [script.pathname, "score", "--results", resultsPath],
      { encoding: "utf8" },
    );
    assert.notEqual(result.status, 0);
    assert.doesNotMatch(result.stdout, /"false_safe"/);
    assert.match(result.stderr, /status must be complete/);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("score CLI rejects malformed JSON before it can emit metrics", () => {
  const directory = mkdtempSync(join(tmpdir(), "iotkit-pure-refactoring-"));
  const resultsPath = join(directory, "malformed-results.json");
  try {
    writeFileSync(resultsPath, "{ not json }\n");
    const result = spawnSync(
      process.execPath,
      [script.pathname, "score", "--results", resultsPath],
      { encoding: "utf8" },
    );
    assert.notEqual(result.status, 0);
    assert.doesNotMatch(result.stdout, /"metrics"/);
    assert.match(result.stderr, /cannot read results/);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
