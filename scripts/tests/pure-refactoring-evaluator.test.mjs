import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  buildPromptBundle,
  compareResults,
  opaqueCaseId,
  scoreResults,
  sha256Bytes,
  sha256Json,
  validateAssets,
  validateCorpus,
  validateHistoricalAssets,
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
const historicalResultsPath = "review/pure-refactoring/evaluations/issue-214-v2-historical-gpt-5.6-sol-high.json";
const historicalReportPath = "review/pure-refactoring/reports/issue-214-v2-historical.md";
const historicalCorpusPath = "review/pure-refactoring/corpus.v2.json";
const historicalProvenancePath = "review/pure-refactoring/historical-provenance.v2.json";
const historicalPolicyPath = "review/pure-refactoring/historical-selection-policy.v1.md";
const historicalCorpus = readJson(historicalCorpusPath);
const historicalProvenance = readJson(historicalProvenancePath);
const historicalPolicy = readFileSync(new URL(historicalPolicyPath, root));
const historicalBundle = buildPromptBundle({
  schema,
  rubric,
  corpus: historicalCorpus,
  provenance: historicalProvenance,
  selectionPolicy: historicalPolicy,
});
const historicalContext = {
  schema,
  rubric,
  corpus: historicalCorpus,
  provenance: historicalProvenance,
  selectionPolicy: historicalPolicy,
  bundle: historicalBundle,
};

function validResultsFor(context, modelId = "gpt-5.6-sol/high") {
  const { schema: resultSchema, rubric: resultRubric, corpus: resultCorpus, bundle: resultBundle } = context;
  return {
    schema_version: resultSchema.schema_version,
    rubric_version: resultRubric.rubric_version,
    prompt_version: resultRubric.prompt_version,
    bundle_sha256: resultBundle.bundle_sha256,
    runs: ["run-001", "run-002", "run-003"].map((run_id) => ({
      run_id,
      model_id: modelId,
      status: "complete",
      cases: resultCorpus.cases.map((entry) => ({
        case_id: entry.id,
        classification: entry.expected_label,
        reason_codes: [...entry.expected_reason_codes],
        evidence: `Synthetic diff review for ${entry.id}.`,
      })),
    })),
  };
}

function validResults() {
  return validResultsFor(context);
}

function validHistoricalResults(modelId = "gpt-5.6-sol/high") {
  return validResultsFor(historicalContext, modelId);
}

function resultErrors(mutate) {
  const results = validResults();
  mutate(results);
  return validateResults(results, context).join("\n");
}

function clonedHistoricalAssets() {
  return {
    schema,
    rubric,
    corpus: structuredClone(historicalCorpus),
    provenance: structuredClone(historicalProvenance),
    selectionPolicy: Buffer.from(historicalPolicy),
  };
}

function historicalErrors(mutate) {
  const assets = clonedHistoricalAssets();
  mutate(assets);
  return validateHistoricalAssets(assets).join("\n");
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

function reportJsonBlock(source) {
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

test("the historical v2 corpus has frozen policy provenance for its recorded capture", () => {
  assert.equal(historicalCorpus.corpus_version, 2);
  assert.match(historicalCorpus.provenance_sha256, /^[a-f0-9]{64}$/);
  assert.deepEqual(validateHistoricalAssets(historicalContext), []);
  assert.deepEqual(validateAssets(historicalContext), []);
  assert.equal(historicalProvenance.corpus_version, 2);
  assert.equal(historicalProvenance.provenance_version, 2);
  assert.equal(
    sha256Bytes(historicalPolicy),
    "184ed652007c09b276a3c1947141b57534263cb9c438cbff869bb04f30464a2f",
  );
  assert.equal(
    historicalProvenance.selection_policy_sha256,
    sha256Bytes(historicalPolicy),
  );
  assert.equal(
    historicalCorpus.provenance_sha256,
    "dd2d9804231f0fd53bc7ab8bd86ff2ee58b1695b304803d1d5bbafa17baaaaa4",
  );
  assert.equal(historicalCorpus.provenance_sha256, sha256Json(historicalProvenance));
  assert.equal(sha256Json(historicalCorpus), "084e0c78cc89661c6b924dae46ec9da0120c435059fb5984aa43fe8f8e99dde2");
  assert.match(historicalPolicy.toString("utf8"), /7070e73577763e893e9f23bd8456ace3799ebfd0/);
});

test("the immutable v1 assets and default prompt retain their known hashes", () => {
  assert.equal(sha256Json(schema), "fc521c54425fbb948d4db16e71e56eae759f64d0a0cad7d6fafad37363a17e69");
  assert.equal(sha256Json(rubric), "5a17d391f05978a71acd92daa7a353c00554c24d639a1351718a55cc0f97c937");
  assert.equal(sha256Json(corpus), "f49aef549fb50cc9a41f0332e832bd249221e645856e0ae96492e1357da4a20f");
  assert.equal(bundle.bundle_sha256, "fbe6dce65589d706d53c0f9c65bdb68cf9fca5030bffcd8866d8b9be281730af");
  const check = spawnSync(process.execPath, [script.pathname, "check"], { encoding: "utf8" });
  assert.equal(check.status, 0, check.stderr);
  assert.equal(check.stdout, "pure-refactoring evaluator: OK (16 cases, 3 required runs)\n");
});

test("historical policy, provenance, corpus, bundle, and results fail closed as one chain", () => {
  const results = validHistoricalResults();
  assert.deepEqual(validateResults(results, historicalContext), []);

  const policyDrift = {
    ...historicalContext,
    selectionPolicy: Buffer.from(`${historicalPolicy.toString("utf8")}drift\n`),
  };
  assert.match(
    validateResults(results, policyDrift).join("\n"),
    /selection_policy_sha256 must match the raw selection policy bytes/,
  );
  const provenanceDrift = structuredClone(historicalProvenance);
  provenanceDrift.cases[0].answer_rationale = "changed answer key";
  assert.match(
    validateResults(results, { ...historicalContext, provenance: provenanceDrift }).join("\n"),
    /provenance_sha256 must match the canonical historical provenance/,
  );

  const corpusDrift = structuredClone(historicalCorpus);
  corpusDrift.cases[0].title = `${corpusDrift.cases[0].title} drift`;
  const driftedAssets = { ...historicalContext, corpus: corpusDrift };
  assert.deepEqual(validateAssets(driftedAssets), []);
  assert.match(
    validateResults(results, driftedAssets).join("\n"),
    /prompt bundle must match the checked-in schema, rubric, and corpus/,
  );
});

test("historical provenance rejects malformed references, hashes, strings, and sanitization", () => {
  assert.match(
    historicalErrors((assets) => {
      assets.provenance.cases[0].pull_request = 0;
      assets.provenance.cases[0].merge_commit_sha = "f".repeat(39);
      assets.provenance.cases[0].source_diff_sha256 = "F".repeat(64);
      assets.provenance.cases[0].pathspec = ["../outside"];
      assets.provenance.cases[0].answer_rationale = "";
      assets.provenance.cases[0].sanitization.removed_index_lines[0].raw_line = 0;
      assets.provenance.cases[0].sanitization.removed_index_lines[0].text = "index invalid";
      assets.provenance.cases[0].sanitization.content_redaction = "redacted";
    }),
    /pull_request must be a positive integer/,
  );
  const errors = historicalErrors((assets) => {
    assets.provenance.cases[0].pull_request = 0;
    assets.provenance.cases[0].merge_commit_sha = "f".repeat(39);
    assets.provenance.cases[0].source_diff_sha256 = "F".repeat(64);
    assets.provenance.cases[0].pathspec = ["../outside"];
    assets.provenance.cases[0].answer_rationale = "";
    assets.provenance.cases[0].sanitization.removed_index_lines[0].raw_line = 0;
    assets.provenance.cases[0].sanitization.removed_index_lines[0].text = "index invalid";
    assets.provenance.cases[0].sanitization.content_redaction = "redacted";
  });
  assert.match(errors, /merge_commit_sha must be a full lowercase Git SHA/);
  assert.match(errors, /source_diff_sha256 must be a lowercase SHA-256 digest/);
  assert.match(errors, /pathspec must contain repository-relative paths/);
  assert.match(errors, /answer_rationale must be a non-empty string/);
  assert.match(errors, /raw_line must be a positive integer/);
  assert.match(errors, /text must be an exact Git index line/);
  assert.match(errors, /content_redaction must be not_required/);
});

test("historical provenance rejects missing, extra, duplicate, and unknown-key crosslinks", () => {
  assert.match(
    historicalErrors((assets) => {
      assets.provenance.cases.pop();
    }),
    /missing corpus case/,
  );
  assert.match(
    historicalErrors((assets) => {
      const extra = structuredClone(assets.provenance.cases[0]);
      extra.case_id = "RF-000000000000";
      assets.provenance.cases.push(extra);
    }),
    /has no matching corpus case/,
  );
  assert.match(
    historicalErrors((assets) => {
      assets.provenance.cases[1].case_id = assets.provenance.cases[0].case_id;
    }),
    /duplicates/,
  );
  assert.match(
    historicalErrors((assets) => {
      const extra = structuredClone(assets.corpus.cases[0]);
      extra.diff += "@@ -1 +1 @@\n+additional historical line\n";
      extra.id = opaqueCaseId(extra.diff);
      assets.corpus.cases.push(extra);
      assets.corpus.cases.sort((left, right) => left.id.localeCompare(right.id));
    }),
    /historical provenance is missing corpus case/,
  );
  assert.match(
    historicalErrors((assets) => {
      assets.provenance.unexpected = true;
    }),
    /historical provenance has unknown key: unexpected/,
  );
});

test("historical source-to-model binding rejects self-consistent model tampering", () => {
  const assets = clonedHistoricalAssets();
  const corpusCase = assets.corpus.cases[0];
  const provenanceCase = assets.provenance.cases.find(
    (entry) => entry.case_id === corpusCase.id,
  );
  assert.ok(provenanceCase);

  corpusCase.diff = `${corpusCase.diff}+injected model-only line\n`;
  corpusCase.id = opaqueCaseId(corpusCase.diff);
  provenanceCase.case_id = corpusCase.id;
  provenanceCase.model_diff_sha256 = sha256Bytes(corpusCase.diff);
  assets.corpus.cases.sort((left, right) => left.id.localeCompare(right.id));
  assets.corpus.provenance_sha256 = sha256Json(assets.provenance);

  assert.match(
    validateHistoricalAssets(assets).join("\n"),
    /source_diff_sha256 must match the deterministically reconstructed raw diff/,
  );
});

test("historical model diffs reject unsanitized metadata and redaction markers", () => {
  for (const [addition, expected] of [
    ["\nindex 0000000..1111111 100644\n", /remove diff index lines/],
    ["\n+PR #999\n", /pull-request, commit, title, or label metadata/],
    ["\n+commit\n", /pull-request, commit, title, or label metadata/],
    ["\n+title: injected\n", /pull-request, commit, title, or label metadata/],
    ["\n+label: injected\n", /pull-request, commit, title, or label metadata/],
    ["\n[REDACTED]\n", /must not contain redacted content/],
  ]) {
    const assets = clonedHistoricalAssets();
    assets.corpus.cases[0].diff += addition;
    assert.match(validateHistoricalAssets(assets).join("\n"), expected);
  }
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

test("the historical v2 prompt exposes only opaque case IDs and sanitized diffs", () => {
  assert.equal(historicalBundle.bundle_sha256, "6b1e81d597d3085326978477a333d715658d90d4a7a76a74122da9e5af87fd94");
  assert.deepEqual(
    Object.keys(historicalBundle).sort(),
    ["schema_version", "rubric_version", "prompt_version", "rubric_sha256", "corpus_sha256", "instructions", "response_contract", "cases", "bundle_sha256"].sort(),
  );
  assert.deepEqual(
    historicalBundle.cases.map((entry) => Object.keys(entry).sort()),
    historicalCorpus.cases.map(() => ["case_id", "diff"]),
  );
  for (const [index, entry] of historicalBundle.cases.entries()) {
    assert.equal(entry.case_id, historicalCorpus.cases[index].id);
    assert.equal(entry.case_id, opaqueCaseId(entry.diff));
    assert.doesNotMatch(entry.diff, /^index /m);
  }
  const serialized = JSON.stringify(historicalBundle);
  assert.doesNotMatch(serialized, /"provenance_sha256"|"pull_request"|"source_commit_sha"|"answer_rationale"/);
  for (const entry of historicalCorpus.cases) {
    assert.equal(serialized.includes(entry.title), false, `prompt must not expose title: ${entry.id}`);
  }
});

test("the v2 check and prompt CLI require explicit corpus selection", () => {
  const check = spawnSync(process.execPath, [script.pathname, "check", "--corpus-version", "2"], {
    encoding: "utf8",
  });
  const prompt = spawnSync(process.execPath, [script.pathname, "prompt", "--corpus-version", "2"], {
    encoding: "utf8",
  });
  assert.equal(check.status, 0, check.stderr);
  assert.equal(prompt.status, 0, prompt.stderr);
  assert.equal(JSON.parse(prompt.stdout).bundle_sha256, historicalBundle.bundle_sha256);
  assert.notEqual(JSON.parse(prompt.stdout).bundle_sha256, bundle.bundle_sha256);
});

test("lightweight CI validates both captured corpus revisions and their descriptive comparison", () => {
  const workflow = readFileSync(new URL(".github/workflows/ci.yml", root), "utf8");
  assert.match(
    workflow,
    /name: Pure-refactoring evaluator foundation[\s\S]*?node scripts\/pure-refactoring-evaluator\.mjs check[\s\S]*?node scripts\/pure-refactoring-evaluator\.mjs check --corpus-version 2[\s\S]*?node --test scripts\/tests\/pure-refactoring-evaluator\.test\.mjs/,
  );
  assert.match(
    workflow,
    /node scripts\/pure-refactoring-evaluator\.mjs score --results review\/pure-refactoring\/evaluations\/issue-212-v1-titlefree-gpt-5\.6-sol-high\.json[\s\S]*?node scripts\/pure-refactoring-evaluator\.mjs score --corpus-version 2 --results review\/pure-refactoring\/evaluations\/issue-214-v2-historical-gpt-5\.6-sol-high\.json[\s\S]*?node scripts\/pure-refactoring-evaluator\.mjs compare --baseline-results review\/pure-refactoring\/evaluations\/issue-212-v1-titlefree-gpt-5\.6-sol-high\.json --historical-results review\/pure-refactoring\/evaluations\/issue-214-v2-historical-gpt-5\.6-sol-high\.json/,
  );
});

test("the recorded report embeds exactly the score of its checked-in raw result", () => {
  const report = readFileSync(new URL(capturedReportPath, root), "utf8");
  const results = readJson(capturedResultsPath);
  assert.deepEqual(reportJsonBlock(report), scoreResults(results, context));
  assert.throws(() => reportJsonBlock("no json block"), /exactly one json fenced block/);
  assert.throws(
    () => reportJsonBlock("```json\n{}\n```\n\n```json\n{}\n```"),
    /exactly one json fenced block/,
  );
  assert.throws(() => reportJsonBlock("```json\nnot json\n```"), SyntaxError);
});

test("the recorded historical v2 result and report preserve its descriptive comparison", () => {
  const results = readJson(historicalResultsPath);
  const baselineResults = readJson(capturedResultsPath);
  assert.deepEqual(validateResults(results, historicalContext), []);
  const score = scoreResults(results, historicalContext);
  assert.deepEqual(score.metrics.expected_reason_misses, {
    decisions: 4,
    eligible_decisions: 48,
    rate: 4 / 48,
    case_ids: ["RF-51804B57B46A", "RF-B7D2D9E782C5"],
  });
  assert.deepEqual(score.metrics.adversarial_false_safe, {
    decisions: 0,
    eligible_decisions: 9,
    rate: 0,
    case_ids: [],
  });
  assert.deepEqual(score.metrics.classification_agreement, {
    unanimous_cases: 16,
    total_cases: 16,
    unstable_case_ids: [],
    pairwise_agreements: 48,
    pairwise_comparisons: 48,
  });
  assert.deepEqual(score.metrics.reason_code_set_agreement, {
    unanimous_cases: 8,
    total_cases: 16,
    unstable_case_ids: [
      "RF-41A5AEE1AB8E",
      "RF-51804B57B46A",
      "RF-7ECACA2BB8A2",
      "RF-829BC337AB79",
      "RF-B86B5F0605AB",
      "RF-BD2AB69DEC16",
      "RF-C12FCD839D8C",
      "RF-E9352880443D",
    ],
    pairwise_agreements: 31,
    pairwise_comparisons: 48,
  });
  const comparison = compareResults({
    baselineResults,
    historicalResults: results,
    baselineAssets: context,
    historicalAssets: historicalContext,
  });
  const report = readFileSync(new URL(historicalReportPath, root), "utf8");
  assert.deepEqual(reportJsonBlock(report), comparison);
  assert.match(report, /Recommendation: \*\*iterate\*\*/);
  assert.match(report, /Do not open a rollout authority issue/i);
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

test("v2 score CLI accepts only a synthetic fixture with the explicit corpus version", () => {
  const directory = mkdtempSync(join(tmpdir(), "iotkit-pure-refactoring-v2-"));
  const resultsPath = join(directory, "results.json");
  try {
    writeFileSync(resultsPath, `${JSON.stringify(validHistoricalResults(), null, 2)}\n`);
    const result = spawnSync(
      process.execPath,
      [script.pathname, "score", "--corpus-version", "2", "--results", resultsPath],
      { encoding: "utf8" },
    );
    assert.equal(result.status, 0, result.stderr);
    const score = JSON.parse(result.stdout);
    assert.equal(score.authority, "report_only");
    assert.equal(score.bundle_sha256, historicalBundle.bundle_sha256);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("compare emits only descriptive unpaired counts, rates, deltas, and agreements", () => {
  const baseline = validResults();
  const historical = validHistoricalResults();
  const dangerous = historicalCorpus.cases.find((entry) => entry.dangerous);
  assert.ok(dangerous);
  const decision = decisionFor(historical, 0, dangerous.id);
  decision.classification = "proven";
  decision.reason_codes = ["structural_only"];

  const report = compareResults({
    baselineResults: baseline,
    historicalResults: historical,
    baselineAssets: context,
    historicalAssets: historicalContext,
  });
  assert.equal(report.authority, "report_only");
  assert.equal(report.comparison, "unpaired_descriptive");
  assert.equal(report.model_id, "gpt-5.6-sol/high");
  assert.deepEqual(report.deltas.false_safe, {
    decisions: 1,
    eligible_decisions: 0,
    rate: 1 / 36,
  });
  assert.equal(report.deltas.classification_agreement.pairwise_agreements, -2);
  assert.ok(
    Math.abs(report.deltas.classification_agreement.pairwise_rate - (-2 / 48)) < Number.EPSILON,
  );
  assert.equal(report.historical.metrics.classification_agreement.pairwise_comparisons, 48);
  assert.doesNotMatch(JSON.stringify(report), /recommend|threshold/i);
});

test("compare fails closed for model, rubric, swapped, tampered, and malformed inputs", () => {
  assert.throws(
    () => compareResults({
      baselineResults: validResults(),
      historicalResults: validHistoricalResults("gpt-5.6-sol/medium"),
      baselineAssets: context,
      historicalAssets: historicalContext,
    }),
    /same model_id/,
  );

  const changedAssets = clonedHistoricalAssets();
  changedAssets.rubric = structuredClone(rubric);
  changedAssets.rubric.reason_codes[0].description = "Changed controlled wording.";
  const changedContext = {
    ...changedAssets,
    bundle: buildPromptBundle(changedAssets),
  };
  assert.throws(
    () => compareResults({
      baselineResults: validResults(),
      historicalResults: validResultsFor(changedContext),
      baselineAssets: context,
      historicalAssets: changedContext,
    }),
    /same rubric/,
  );

  assert.throws(
    () => compareResults({
      baselineResults: validHistoricalResults(),
      historicalResults: validResults(),
      baselineAssets: context,
      historicalAssets: historicalContext,
    }),
    /bundle_sha256 must match the prompt bundle/,
  );

  const tampered = validHistoricalResults();
  tampered.bundle_sha256 = "0".repeat(64);
  assert.throws(
    () => compareResults({
      baselineResults: validResults(),
      historicalResults: tampered,
      baselineAssets: context,
      historicalAssets: historicalContext,
    }),
    /bundle_sha256 must match the prompt bundle/,
  );

  assert.throws(
    () => compareResults({
      baselineResults: null,
      historicalResults: validHistoricalResults(),
      baselineAssets: context,
      historicalAssets: historicalContext,
    }),
    /results must be an object/,
  );
});

test("compare CLI accepts fixed v1 and v2 result roles without creating a report artifact", () => {
  const directory = mkdtempSync(join(tmpdir(), "iotkit-pure-refactoring-compare-"));
  const baselinePath = join(directory, "baseline.json");
  const historicalPath = join(directory, "historical.json");
  try {
    writeFileSync(baselinePath, `${JSON.stringify(validResults(), null, 2)}\n`);
    writeFileSync(historicalPath, `${JSON.stringify(validHistoricalResults(), null, 2)}\n`);
    const result = spawnSync(
      process.execPath,
      [
        script.pathname,
        "compare",
        "--baseline-results",
        baselinePath,
        "--historical-results",
        historicalPath,
      ],
      { encoding: "utf8" },
    );
    assert.equal(result.status, 0, result.stderr);
    const report = JSON.parse(result.stdout);
    assert.equal(report.authority, "report_only");
    assert.equal(report.comparison, "unpaired_descriptive");
    assert.doesNotMatch(result.stdout, /recommend|threshold/i);
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
