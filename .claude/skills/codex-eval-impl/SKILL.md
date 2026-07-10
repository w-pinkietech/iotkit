---
name: codex-eval-impl
description: Use when reviewing a codex-implemented task after scripts/verify.sh passes. Defines the two lenses (spec compliance + code quality) of the single combined per-task review prompt.
---

# Codex Eval Impl (spec + quality lenses)

The per-task review is ONE combined prompt carrying BOTH lenses below, run through
codex (read-only) AND Fable (review-max) in parallel. This skill defines what goes
IN the prompt; the loop around it (dispatch, converge, fix, watchpoint registration)
lives in `codex-impl-loop` step 4 — not duplicated here.

**REQUIRED:** Read `codex-eval-common` for shared rules (CLI, iteration, safety).

## Context to Inject

| File | Purpose |
|---|---|
| `docs/eval/impl-spec-review.md` | Spec-compliance lens guide (watchpoints + baseline) |
| `docs/eval/impl-quality-review.md` | Code-quality lens guide (watchpoints + baseline) |
| `docs/architecture.md` — the crate map, layer rules, placement table, and "Who this serves" sections | The structure canon: reviewers can't judge placement without the map |
| The plan task being evaluated | For spec-implementation comparison |

Run `scripts/watchpoints.sh` first — adjudicate any expired watchpoints
(eval-perspectives-curator) so the guides you inject are current.

## Prompt Template

```
You are an independent implementation reviewer. Evaluate the changes through
BOTH lenses; report findings per lens.

## Lens A — Spec Compliance
1. Coverage — is every step in the task spec implemented?
2. Scope — is there anything beyond the task spec?
3. Naming consistency — do types, fields, functions match the spec exactly?
4. Test compliance — are specified tests implemented and correct?
5. Lateral impact — are pattern matches and references updated workspace-wide?

## Lens B — Code Quality
1. Error handling — sufficient context? recoverable vs fatal distinguished?
2. Concurrency — consistent send paths? clean shutdown? races?
3. Memory — allocations in hot paths? unbounded growth?
4. Rust idioms — types over sentinels? natural ownership? honest naming?
5. Observability — structured tracing fields? anomaly timeline traceable?
6. Test quality — edge cases covered? behavior verified, not just compilation?
7. Structure & placement — new code in the right crate/module per the
   architecture canon? file responsibilities intact? consistent with
   neighboring code's patterns?
8. Contributor/user lens — pub items documented? wire/ops-visible changes
   synced to docs? usable by the target persona without reading Rust internals?

## Architecture Canon (structure/placement/persona reference)
{crate map, layer rules, placement table, "Who this serves" from docs/architecture.md}

## Spec Compliance Review Guide
{content of docs/eval/impl-spec-review.md}

## Implementation Quality Review Guide
{content of docs/eval/impl-quality-review.md}

## Task Spec (from plan)
{the specific task text}

## Code Changes
{git diff or changed file contents}

## Reality Check
{state claims — expected HEAD, commit range, key code facts, test counts —
for the vendor to independently confirm/refute against git/disk/test}

For each lens and perspective, state findings with severity
(Critical/Important/Minor); "No issues identified" where clean.
```

## After Evaluation

- Findings feed the converge step (`codex-impl-loop` step 4); the fix loop is step 5.
- Register novel blind spots in the matching guide: spec-compliance →
  `docs/eval/impl-spec-review.md`, quality → `docs/eval/impl-quality-review.md`.
