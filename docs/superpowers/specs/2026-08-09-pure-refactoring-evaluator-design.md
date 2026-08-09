# Pure-refactoring evaluator foundation

Issue: [#212](https://github.com/w-pinkietech/iotkit/issues/212)

Status: approved

Lane: Standard

## Goal

Create a versioned, offline experiment that measures whether repeated evaluator
runs recognize demonstrably local structural refactors and reject changes at
IoTKit's high-risk boundaries.

## Non-goals

- Calling a model API or adding a model SDK, dependency, secret, or network
  path.
- Reading live pull requests or changing product behavior.
- Posting a status, approval, auto-merge request, branch-protection setting, or
  any other GitHub write.
- Treating a score as proof or authority to merge.

## Chosen boundary

Checked-in synthetic v1 rubric and corpus inputs produce one deterministic,
blinded SHA-256-bound prompt bundle. Each independent evaluator returns one run
object; the recorder combines at least three complete runs from one exact
pinned model configuration. The evaluator rejects
unknown fields, unknown reason codes, missing/extra/duplicate case decisions,
version/hash drift, empty evidence, and incomplete or ambiguous runs before it
can score them. It reports false-safe, false-reject, dangerous/adversarial
false-safe, expected-reason misses, classification agreement, and exact
reason-code-set agreement metrics only.

Each model-visible case uses an opaque ID derived only from its diff:
`RF-` plus the first 12 uppercase hexadecimal characters of SHA-256. The corpus
is sorted lexicographically by that ID and guards against labels being
partitioned into either half. Corpus-authored titles remain human answer-key
context but are excluded from the prompt: each model-visible case has only
`case_id` and `diff`. This changed the prompt bundle and required fresh
independent blinded captures before recording a report or recommendation.

## Recorded title-free v1 result

On 2026-08-09, three independent blinded contexts used the title-free bundle
and the recorder-attested pinned `gpt-5.6-sol/high` configuration. The
checked-in [report](../../../review/pure-refactoring/reports/issue-212-v1-titlefree.md)
records zero false-safe, false-reject, dangerous/adversarial false-safe
decisions and unanimous classification agreement across all 16 cases. It also
records one expected-reason miss and reason-code-set variation, so the decision
is **iterate**. The report is evidence about a small synthetic corpus, never
merge or rollout authority.

## Verification

The focused Node test covers the corpus risk categories, blinded deterministic
bundle, opaque ID/order invariants, title-free prompt cases, valid score rates
and denominators, expected-reason coverage, and every fail-closed response
boundary. Lightweight CI runs `check`, the focused test, and the checked-in
score command. The score validates capture shape and reports metrics only; it
does not apply a rollout threshold. Any rollout remains a separate Full-lane
decision.
