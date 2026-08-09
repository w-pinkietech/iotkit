# Issue #214 historical v2 evaluator report

Date: 2026-08-09
Status: report-only
Human decision: **iterate** — not rollout and not stop.

## Selection and capture

The v2 population was frozen in
[`historical-selection-policy.v1.md`](../historical-selection-policy.v1.md)
before these runs. Its recorder-only provenance binds that policy to selected
historical source-diff excerpts, exact reversible Git `index`-line
sanitization evidence, the corpus, and the model-visible bundle. The evaluator
and CI contain no model or GitHub client.

Three genuine no-tool runs evaluated that same frozen bundle with the
recorder-attested configuration `gpt-5.6-sol/high`. The checked-in raw
container
[`issue-214-v2-historical-gpt-5.6-sol-high.json`](../evaluations/issue-214-v2-historical-gpt-5.6-sol-high.json)
preserves every supplied run, decision, evidence string, and case order
unchanged.

- Schema / rubric / prompt version: `1 / 1 / 1`
- Historical bundle SHA-256:
  `6b1e81d597d3085326978477a333d715658d90d4a7a76a74122da9e5af87fd94`
- Cases / runs / decisions: `16 / 3 / 48`

## Actual score and descriptive comparison

V2 has no false-safe, false-reject, dangerous false-safe, or adversarial
false-safe decisions: `0/36`, `0/12`, `0/33`, and `0/9`. It has four
expected-reason misses in 48 decisions, all on
`RF-51804B57B46A` and `RF-B7D2D9E782C5`. Classification agreement is
perfect on this set: `16/16` unanimous cases and `48/48` pairwise
comparisons. Exact reason-code-set agreement is only `8/16` unanimous cases
and `31/48` pairwise comparisons.

The following is the evaluator's fixed, unpaired descriptive comparison against
the checked-in Issue #212 v1 capture:

```json
{
  "authority": "report_only",
  "comparison": "unpaired_descriptive",
  "model_id": "gpt-5.6-sol/high",
  "rubric_sha256": "5a17d391f05978a71acd92daa7a353c00554c24d639a1351718a55cc0f97c937",
  "baseline": {
    "corpus_version": 1,
    "bundle_sha256": "fbe6dce65589d706d53c0f9c65bdb68cf9fca5030bffcd8866d8b9be281730af",
    "metrics": {
      "runs": 3,
      "cases": 16,
      "decisions": 48,
      "false_safe": {
        "decisions": 0,
        "eligible_decisions": 36,
        "rate": 0
      },
      "false_reject": {
        "decisions": 0,
        "eligible_decisions": 12,
        "rate": 0
      },
      "dangerous_false_safe": {
        "decisions": 0,
        "eligible_decisions": 33,
        "rate": 0
      },
      "adversarial_false_safe": {
        "decisions": 0,
        "eligible_decisions": 21,
        "rate": 0
      },
      "expected_reason_misses": {
        "decisions": 1,
        "eligible_decisions": 48,
        "rate": 0.020833333333333332
      },
      "classification_agreement": {
        "unanimous_cases": 16,
        "total_cases": 16,
        "unanimous_rate": 1,
        "pairwise_agreements": 48,
        "pairwise_comparisons": 48,
        "pairwise_rate": 1
      },
      "reason_code_set_agreement": {
        "unanimous_cases": 15,
        "total_cases": 16,
        "unanimous_rate": 0.9375,
        "pairwise_agreements": 45,
        "pairwise_comparisons": 48,
        "pairwise_rate": 0.9375
      }
    }
  },
  "historical": {
    "corpus_version": 2,
    "bundle_sha256": "6b1e81d597d3085326978477a333d715658d90d4a7a76a74122da9e5af87fd94",
    "metrics": {
      "runs": 3,
      "cases": 16,
      "decisions": 48,
      "false_safe": {
        "decisions": 0,
        "eligible_decisions": 36,
        "rate": 0
      },
      "false_reject": {
        "decisions": 0,
        "eligible_decisions": 12,
        "rate": 0
      },
      "dangerous_false_safe": {
        "decisions": 0,
        "eligible_decisions": 33,
        "rate": 0
      },
      "adversarial_false_safe": {
        "decisions": 0,
        "eligible_decisions": 9,
        "rate": 0
      },
      "expected_reason_misses": {
        "decisions": 4,
        "eligible_decisions": 48,
        "rate": 0.08333333333333333
      },
      "classification_agreement": {
        "unanimous_cases": 16,
        "total_cases": 16,
        "unanimous_rate": 1,
        "pairwise_agreements": 48,
        "pairwise_comparisons": 48,
        "pairwise_rate": 1
      },
      "reason_code_set_agreement": {
        "unanimous_cases": 8,
        "total_cases": 16,
        "unanimous_rate": 0.5,
        "pairwise_agreements": 31,
        "pairwise_comparisons": 48,
        "pairwise_rate": 0.6458333333333334
      }
    }
  },
  "deltas": {
    "runs": 0,
    "cases": 0,
    "decisions": 0,
    "false_safe": {
      "decisions": 0,
      "eligible_decisions": 0,
      "rate": 0
    },
    "false_reject": {
      "decisions": 0,
      "eligible_decisions": 0,
      "rate": 0
    },
    "dangerous_false_safe": {
      "decisions": 0,
      "eligible_decisions": 0,
      "rate": 0
    },
    "adversarial_false_safe": {
      "decisions": 0,
      "eligible_decisions": -12,
      "rate": 0
    },
    "expected_reason_misses": {
      "decisions": 3,
      "eligible_decisions": 0,
      "rate": 0.0625
    },
    "classification_agreement": {
      "unanimous_cases": 0,
      "total_cases": 0,
      "unanimous_rate": 0,
      "pairwise_agreements": 0,
      "pairwise_comparisons": 0,
      "pairwise_rate": 0
    },
    "reason_code_set_agreement": {
      "unanimous_cases": -7,
      "total_cases": 0,
      "unanimous_rate": -0.4375,
      "pairwise_agreements": -14,
      "pairwise_comparisons": 0,
      "pairwise_rate": -0.29166666666666663
    }
  }
}
```

## Limits and recommendation

This is a small bounded corpus of selected, sanitized historical PR excerpts,
not a representative production population. The comparison is unpaired because
v1 and v2 use different corpora; its deltas are descriptive rather than causal.
Run independence and exact model identity are recorder attestations, and the
model alias/effort does not identify a backend revision.

Classification remained perfect in this small bounded set, but reason stability
materially degraded from v1: unanimous exact reason sets fell from `15/16` to
`8/16`, while pairwise exact-set agreement fell from `45/48` to `31/48`.
Expected-reason misses also rose from `1/48` to `4/48`.

Recommendation: **iterate**. Do not open a rollout authority issue from this
report. The comparison is neither approval nor a threshold for merge,
auto-merge, release, or any product decision; human approval remains required.
