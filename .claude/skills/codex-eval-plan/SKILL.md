---
name: codex-eval-plan
description: Use after writing an implementation plan, before starting the codex-impl-loop. Evaluates task granularity, dependency ordering, and spec consistency.
---

# Codex Eval Plan

Evaluate an implementation plan with Codex before starting the `codex-impl-loop`.

**REQUIRED:** Read `codex-eval-common` for shared rules (CLI, iteration, safety).

## When to Use

- After `superpowers:writing-plans` completes
- After plan self-review is complete
- Before starting the `codex-impl-loop` for implementation

Plans are contract-centered per `docs/development-workflow.md`: constrain invariants,
forbidden scope, failing tests, verification, review focus, commit boundary, and rollback.
Do not require helper names or code snippets unless semantically load-bearing.

## Context to Inject

| File | Purpose |
|---|---|
| `docs/eval/plan-review.md` | Plan review guide (watchpoints + baseline checklist) |
| `docs/architecture.md` — crate map, layer rules, placement table | The structure canon; the guide's File-Structure check needs the map |
| The corresponding spec document | For spec-plan consistency check |

## Evaluation Focus (5 perspectives)

1. Task decomposition — each task independently compilable/testable?
2. Dependency ordering — inner-to-outer, no circular, parallelizable tasks identified?
3. Spec coverage — every spec requirement mapped to a task? No scope creep?
4. Implementation accuracy — type names, field names, file paths consistent with spec?
5. Test strategy — each task has concrete tests, not just "update tests"?

## Prompt Template

```
You are an independent plan evaluator. Thoroughly review the following
implementation plan from a third-party perspective.

## Vendor roles and common safety core
{required vendor roles from docs/development-workflow.md; every vendor checks Red
classification, auth/secrets, data loss/custody, external effects, hash provenance,
settlement, then its specialty and a residual out-of-role C/I pass}

## Evaluation Perspectives (all 5)
1. Task decomposition — is each task independently compilable and testable?
   Are tasks too large or too small?
2. Dependency ordering — do tasks follow inner-to-outer order? Are parallelizable
   tasks identified? Will cargo test pass at each intermediate step?
3. Spec coverage — does every spec requirement map to a specific task and step?
   Are there tasks that go beyond the spec?
4. Implementation accuracy — do type names, field names, function signatures,
   and file paths match the spec exactly? Any inconsistencies between tasks?
5. Test strategy — does each task have concrete test code (not just "add tests")?
   Are edge cases and error paths covered?

## Plan Review Guide
{content of docs/eval/plan-review.md}

## Architecture Canon (structure/placement reference)
{crate map, layer rules, placement table from docs/architecture.md}

## Spec Document (for consistency check)
{content of the spec this plan implements}

## Plan Content
{the plan document}

{standard output format from codex-eval-common}
```

## After Evaluation

- Fix Critical/Important issues following `codex-eval-common` iteration loop
- Register novel watchpoints in the Active Watchpoints section of `docs/eval/plan-review.md`
- When clean: start the `codex-impl-loop` for implementation
