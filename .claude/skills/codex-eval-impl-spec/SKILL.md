---
name: codex-eval-impl-spec
description: Use as spec compliance reviewer after codex implements a task. Evaluates whether implementation matches the plan task specification exactly.
---

# Codex Eval Impl Spec

Evaluate whether implementation matches the plan task specification.
The **spec-compliance lens** of the single combined per-task review prompt
(`codex-impl-loop`), run through codex + Fable. Paired with the code-quality lens
(`codex-eval-impl-quality`) in the SAME prompt — not a separate sequential stage.

**REQUIRED:** Read `codex-eval-common` for shared rules (CLI, iteration, safety).

## When to Use

- After codex reports task completion (host-verified with `scripts/verify.sh`)
- As the spec-compliance lens of the per-task review prompt (paired with code quality)

## Context to Inject

| File | Purpose |
|---|---|
| `docs/eval/impl-spec-review.md` | Spec compliance review guide (watchpoints + baseline) |
| The plan task being evaluated | For spec-implementation comparison |

## Evaluation Focus

1. Does the code implement every step in the plan task?
2. Is there anything implemented that the plan task did not specify?
3. Do type names, field names, function signatures match the plan exactly?
4. Are the specified tests implemented and passing?
5. Is the scope contained — no "improvements" beyond the task?

## Prompt Template

```
You are a spec compliance evaluator. Compare the implementation changes
against the task specification and identify any deviations.

## Evaluation Perspectives
1. Coverage — is every step in the task spec implemented?
2. Scope — is there anything beyond the task spec?
3. Naming consistency — do types, fields, functions match the spec exactly?
4. Test compliance — are specified tests implemented and correct?
5. Lateral impact — are pattern matches and references updated workspace-wide?

## Spec Compliance Review Guide
{content of docs/eval/impl-spec-review.md}

## Task Spec (from plan)
{the specific task text}

## Code Changes
{git diff or changed file contents}

For each perspective, state findings with severity (Critical/Important/Minor).
```

## After Evaluation

- Findings feed the per-task converge step (codex-impl-loop step 4); the same prompt also carries the code-quality lens (codex-eval-impl-quality)
- FAIL: codex fixes issues (fix prompt), then re-review
- Register novel watchpoints in Active Watchpoints section of `docs/eval/impl-spec-review.md`
