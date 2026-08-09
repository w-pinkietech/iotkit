# Pure-refactoring evaluator

[日本語](README.ja.md) | English

Status: **Experimental / report-only** ([#212](https://github.com/w-pinkietech/iotkit/issues/212),
[#214](https://github.com/w-pinkietech/iotkit/issues/214)). The
[title-free v1 recorded report](reports/issue-212-v1-titlefree.md) recommends
**iterate**, not rollout. The [historical v2 report](reports/issue-214-v2-historical.md)
also records the human decision **iterate**, not rollout or stop.

## Intent

Measure whether independently captured evaluator responses distinguish small,
structural refactors from changes that cannot be established as pure refactors.
The versioned corpus is deliberately conservative around IoTKit's security,
custody, recovery, deployment, public wire/API, current product documentation
authority, and operator-visible boundaries. V1 is synthetic; v2 is a
sanitized, provenance-bound historical corpus.

Each case ID is `RF-` plus the first 12 uppercase hexadecimal characters of
SHA-256 over the model-visible diff. Cases are sorted by that opaque ID, not by
expected outcome; both corpus halves include both labels as a regression guard.
Corpus titles remain human answer-key context and are not included in the
model-visible bundle: every prompt case contains only `case_id` and `diff`.

## When

Use this perspective only to evaluate the evaluator itself: after changing its
rubric, corpus, prompt packaging, or scoring rules, and before considering any
future automation proposal. It is not a normal PR gate.

## How

1. Validate the checked-in v1 inputs:

   ```bash
   node scripts/pure-refactoring-evaluator.mjs check
   ```

2. Validate and emit the frozen historical v2 bundle:

   ```bash
   node scripts/pure-refactoring-evaluator.mjs check --corpus-version 2
   node scripts/pure-refactoring-evaluator.mjs prompt --corpus-version 2 > /tmp/iotkit-pure-refactoring-v2.json
   ```

   V2 binds raw selection-policy bytes to recorder-only provenance, then to
   the corpus and prompt bundle. It reversibly removes Git `index` lines:
   provenance records their exact text and one-based raw positions so
   validation reconstructs and hashes the source diff before checking the
   model-visible diff. It excludes PR, commit, title, and label metadata from
   model-visible input. Its recorded capture and report remain report-only.

3. Emit one deterministic blinded v1 bundle:

   ```bash
   node scripts/pure-refactoring-evaluator.mjs prompt > /tmp/iotkit-pure-refactoring-v1.json
   ```

4. Give the same bundle to at least three independently recorded evaluators
   using one exact pinned configuration. Each evaluator returns **exactly one**
   run object matching `single_run_keys` and `case_keys`; the recorder supplies
   a unique `run_id` and the same non-empty `model_id` for every run (for
   example, `gpt-5.6-sol/high`). The recorder, not an evaluator, combines those
   run objects into the result container. Do not invent result examples;
   record only actual captured runs under `evaluations/`.
5. Score the checked-in title-free v1 capture:

   ```bash
   node scripts/pure-refactoring-evaluator.mjs score --results review/pure-refactoring/evaluations/issue-212-v1-titlefree-gpt-5.6-sol-high.json
   ```

The recorded v1/v2 comparison is descriptive and unpaired:

```bash
node scripts/pure-refactoring-evaluator.mjs compare \
  --baseline-results V1_RESULTS.json \
  --historical-results V2_RESULTS.json
```

It compares counts, rates, deltas, and agreement summaries only. It has no
threshold or rollout authority; the final report's human decision is
**iterate**.

The evaluator rejects unknown keys, unknown reason codes, incomplete or
ambiguous runs, version/hash drift, policy/provenance/corpus drift, and case
coverage mismatches. It emits false-safe, false-reject, dangerous/adversarial
false-safe, and repeat-run metrics as a report only. Repeat-run metrics are
deliberately separate:

- **classification agreement** compares only `proven` / `not_proven` labels;
- **reason-code-set agreement** compares exact controlled-code sets, ignoring
  order; and
- **expected-reason misses** count decisions whose code set includes none of
  the case's expected codes.

Each error metric separately reports its observed `decisions`, relevant
`eligible_decisions`, and `rate`; the denominator is the matching expected-case
population across every recorded run.

## Not

- Not a proof of behavioral equivalence, security, custody, compatibility, or
  release readiness.
- Not an approval, required status, auto-merge trigger, or replacement for
  human review. The per-head `human approval` boundary remains required.
- Not a model client, network call, secret store, or live pull-request reader.
- Not permission to record customer data, credentials, or unredacted field
  evidence.
- Not a reason to treat the recorded v2 result or report as rollout authority.

The title-free v1 report recommends iteration, not rollout. Any proposal to
make these metrics authoritative is a separate Full-lane decision after broader
evidence supports it.
