# Issue #212 title-free v1 evaluator report

Date: 2026-08-09
Status: report-only
Recommendation: **iterate**

## Capture

Three independently recorded blinded contexts evaluated the same title-free v1
bundle using the recorder-attested pinned model configuration
`gpt-5.6-sol/high`. The raw run container is
[`issue-212-v1-titlefree-gpt-5.6-sol-high.json`](../evaluations/issue-212-v1-titlefree-gpt-5.6-sol-high.json).

- Schema / rubric / prompt version: `1 / 1 / 1`
- Bundle SHA-256:
  `fbe6dce65589d706d53c0f9c65bdb68cf9fca5030bffcd8866d8b9be281730af`
- Cases / runs / decisions: `16 / 3 / 48`

This capture uses the corrected model-visible input: every prompt case contains
only `case_id` and `diff`; corpus-authored titles remain answer-key context.
Case IDs are opaque SHA-256 prefixes derived only from their diffs and ordered
by that ID, rather than by expected outcome. The three raw runs were newly
captured against this title-free bundle; each checked-in run deep-equals its
recorded source.

## Score

```json
{
  "authority": "report_only",
  "schema_version": 1,
  "rubric_version": 1,
  "prompt_version": 1,
  "bundle_sha256": "fbe6dce65589d706d53c0f9c65bdb68cf9fca5030bffcd8866d8b9be281730af",
  "metrics": {
    "runs": 3,
    "cases": 16,
    "decisions": 48,
    "false_safe": {
      "decisions": 0,
      "eligible_decisions": 36,
      "rate": 0,
      "case_ids": []
    },
    "false_reject": {
      "decisions": 0,
      "eligible_decisions": 12,
      "rate": 0,
      "case_ids": []
    },
    "dangerous_false_safe": {
      "decisions": 0,
      "eligible_decisions": 33,
      "rate": 0,
      "case_ids": []
    },
    "adversarial_false_safe": {
      "decisions": 0,
      "eligible_decisions": 21,
      "rate": 0,
      "case_ids": []
    },
    "expected_reason_misses": {
      "decisions": 1,
      "eligible_decisions": 48,
      "rate": 0.020833333333333332,
      "case_ids": [
        "RF-59429CEA4563"
      ]
    },
    "classification_agreement": {
      "unanimous_cases": 16,
      "total_cases": 16,
      "unstable_case_ids": [],
      "pairwise_agreements": 48,
      "pairwise_comparisons": 48
    },
    "reason_code_set_agreement": {
      "unanimous_cases": 15,
      "total_cases": 16,
      "unstable_case_ids": [
        "RF-59429CEA4563"
      ],
      "pairwise_agreements": 45,
      "pairwise_comparisons": 48
    }
  }
}
```

All 16 classifications agree across all three runs (48/48 pairwise comparisons).
There are no false-safe, false-reject, dangerous false-safe, or adversarial
false-safe decisions: `0/36`, `0/12`, `0/33`, and `0/21` respectively. One
expected-reason miss is `RF-59429CEA4563` (`1/48`): the third run used only
`operator_visible_behavior`, not the corpus's expected
`public_wire_api_contract` code. Exact reason-code sets vary only on that case
and agree on 45/48 pairwise comparisons.

## Limits and recommendation

This is not enough evidence for rollout. The corpus has only 16 small, obvious
synthetic diffs; it uses one configured model alias/effort and does not expose a
backend revision. Run independence and model identity are recorder attestations,
not independently verified properties. There are no sanitized historical PR or
near-miss refactor cases, and the expected-reason miss plus reason-code
variability show remaining explanatory inconsistency.

Recommendation: **iterate** by expanding the corpus with sanitized historical
near-miss/refactor cases and repeating blinded runs. Do not open a rollout issue
from this report. The score is neither an approval nor a threshold for merge,
auto-merge, or release; human approval remains required.
