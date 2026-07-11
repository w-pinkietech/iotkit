---
name: codex-eval-spec
description: Use after writing a design spec, before proceeding to plan. Replaces user review gate in brainstorming. Evaluates architecture, scope, and feasibility.
---

# Codex Eval Spec

Evaluate a design spec with Codex before proceeding to implementation planning.

**REQUIRED:** Read `codex-eval-common` for shared rules (CLI, iteration, safety).

## When to Use

- After `superpowers:brainstorming` writes the spec (replaces user review gate)
- When spec self-review is complete
- Before invoking `superpowers:writing-plans`

Precondition: the Design Ready evidence pack in `docs/development-workflow.md` is complete.
For Large/Red work, inject its constraint ledger, state machine, trust provenance, user
journey, adversarial-six answers, and invariant traceability into the review prompt.

## Context to Inject

| File | Purpose |
|---|---|
| `docs/eval/spec-review.md` | Spec review guide (watchpoints + baseline checklist) |
| `docs/architecture.md` — Site anatomy, crate map, layer rules, Who this serves | The structure canon; the guide's placement/anatomy items reference it |

## Evaluation Focus (5 perspectives)

1. Logical contradictions, implicit assumptions, overlooked dependencies
2. Scope creep — YAGNI
3. Feasibility and technical risk
4. Edge cases and failure scenarios
5. Architectural alternatives

## Prompt Template

```
You are an independent design evaluator. Thoroughly review the following
design spec from a third-party perspective.

## Vendor roles and common safety core
{required vendor roles from docs/development-workflow.md; every vendor checks Red
classification, auth/secrets, data loss/custody, external effects, hash provenance,
settlement, then its specialty and a residual out-of-role C/I pass}

## Evaluation Perspectives (all 5)
1. Logical contradictions, implicit assumptions, overlooked dependencies
2. Scope creep — is everything truly needed? Simpler alternatives?
3. Feasibility and technical risk — is this implementable? Hidden complexity?
4. Edge cases and failure scenarios — error paths, boundary conditions
5. Architectural alternatives — better approaches? Missed options?

## Spec Review Guide
{content of docs/eval/spec-review.md}

## Architecture Canon (structure/placement/persona reference)
{Site anatomy, crate map, layer rules, Who this serves from docs/architecture.md}

## Spec Content
{the spec document}

{standard output format from codex-eval-common}
```

## After Evaluation

- Fix Critical/Important issues following `codex-eval-common` iteration loop
- Register novel watchpoints in the Active Watchpoints section of `docs/eval/spec-review.md`
- When clean: proceed to `superpowers:writing-plans`
