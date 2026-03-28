---
name: codex-eval-impl-quality
description: Use as code quality reviewer after spec compliance passes. Evaluates code quality, error handling, concurrency, Rust idioms, and test adequacy.
---

# Codex Eval Impl Quality

Evaluate code quality of the implementation.
Used by the code quality reviewer subagent in the 2-stage review process.

**REQUIRED:** Read `codex-eval-common` for shared rules (CLI, iteration, safety).

## When to Use

- After spec compliance review (codex-eval-impl-spec) passes
- Second stage of the 2-stage review

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

- PASS: Task complete, lead merges
- FAIL: Dev subagent fixes issues, then re-review (quality only, not spec again)
- Register novel watchpoints in Active Watchpoints section of `docs/eval/impl-quality-review.md`
