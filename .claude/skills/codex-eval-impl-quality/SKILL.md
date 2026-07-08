---
name: codex-eval-impl-quality
description: Use as the code-quality lens of the per-task review. Evaluates code quality, error handling, concurrency, Rust idioms, and test adequacy.
---

# Codex Eval Impl Quality

Evaluate code quality of the implementation.
The **code-quality lens** of the single combined per-task review prompt
(`codex-impl-loop`), run through codex + Fable. Paired with the spec-compliance lens
(`codex-eval-impl-spec`) in the SAME prompt — not a separate sequential stage.

**REQUIRED:** Read `codex-eval-common` for shared rules (CLI, iteration, safety).

## When to Use

- As the code-quality lens of the per-task review prompt (paired with spec compliance)

## Context to Inject

| File | Purpose |
|---|---|
| `docs/eval/impl-quality-review.md` | Quality review guide (watchpoints + baseline) |

## Evaluation Focus

1. Error handling — context, recoverability, consistency
2. Concurrency — channel handling, shutdown paths, state management
3. Memory — unnecessary allocations, unbounded growth
4. Rust idioms — types over strings, ownership, naming
5. Observability — structured logging, traceability
6. Test quality — edge cases, contract testing (not just compilation)

## Prompt Template

```
You are a code quality evaluator. Review the implementation for
correctness, robustness, and idiomatic Rust.

## Evaluation Perspectives
1. Error handling — does every error include sufficient context?
   Are recoverable and fatal errors distinguished?
2. Concurrency — are channel send paths consistent? Are shutdown
   paths clean? Any race conditions?
3. Memory — any unnecessary allocations in hot paths? Unbounded growth?
4. Rust idioms — types over sentinel strings? Ownership natural?
   Naming reflects responsibility?
5. Observability — structured tracing fields? Anomaly timeline traceable?
6. Test quality — edge cases covered? Tests verify behavior, not just compile?

## Implementation Quality Review Guide
{content of docs/eval/impl-quality-review.md}

## Code Changes
{git diff or changed file contents}

For each perspective, state findings with severity (Critical/Important/Minor).
```

## After Evaluation

- PASS: Task complete, Main commits (one commit per task)
- FAIL: codex fixes issues (fix prompt), then re-review
- Register novel watchpoints in Active Watchpoints section of `docs/eval/impl-quality-review.md`
