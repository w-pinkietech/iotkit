---
name: eval-perspectives-curator
description: Maintain and evolve Codex eval-perspectives — run after each codex-eval-review cycle completes, or when the user asks to review perspective quality. Prevents staleness, bias, and overfit.
---

# Eval Perspectives Curator

## Overview

The Codex evaluator's effectiveness depends on the quality of its learned perspectives. Without curation, perspectives become stale, narrow, or redundant — and the evaluator regresses to generic advice.

This skill maintains the phase-specific eval-perspectives files as living documents:
- `docs/eval/spec-review.md` — Active Watchpoints section for design/architecture
- `docs/eval/plan-review.md` — Active Watchpoints section for task decomposition
- `docs/eval/impl-spec-review.md` — Active Watchpoints for spec compliance
- `docs/eval/impl-quality-review.md` — Active Watchpoints for code quality/runtime

**Core principle:** Perspectives should be the distilled lessons that make *this project's* reviews sharper than a generic code review. If a perspective wouldn't change what Codex looks for, it's noise.

## When to Use

### Automatic (after codex-eval-review)

Run after every codex-eval-review cycle completes. Check:
1. Did this review produce findings that deserve a new perspective?
2. Did any existing perspective prove its value (Codex found what it predicted)?
3. Did any perspective fail to help (Codex missed something the perspective should have caught)?

### Periodic (review-by dates)

Each perspective has a review-by date (3 months from creation). When a perspective reaches its review-by date:
- **Still relevant + recently validated:** Reset review-by date (+3 months)
- **Relevant but never triggered:** Keep, but lower confidence
- **Outdated (code/architecture changed):** Update or remove
- **Superseded by a broader insight:** Merge into the broader one

### On Request

When the user asks to review, audit, or improve perspectives.

## Curation Process

### Step 1: Read Current State

```bash
cat docs/eval/spec-review.md
cat docs/eval/plan-review.md
cat docs/eval/impl-spec-review.md
cat docs/eval/impl-quality-review.md
```

### Step 2: Evaluate Each Perspective

For each active perspective, assess:

| Dimension | Question | Action if failing |
|-----------|----------|-------------------|
| **Relevance** | Does this still apply to the current codebase? | Update or remove |
| **Specificity** | Is this specific to iotkit-next, or generic advice? | Sharpen or remove |
| **Actionability** | Does this change what the evaluator looks for? | Rewrite or remove |
| **Coverage balance** | Are perspectives clustered on one topic? | Diversify |
| **Freshness** | When was this last validated by a real finding? | Mark stale if never |

### Step 3: Score and Categorize

Assign each perspective a status:

- **Active** — recently validated, clearly relevant
- **Aging** — still relevant but not validated in last 2 review cycles
- **Stale** — past review-by date or codebase has moved on
- **Redundant** — covered by another perspective or now obvious

### Step 4: Apply Changes

- **Active:** Keep, optionally sharpen wording
- **Aging:** Keep but add "(unvalidated since {date})" note
- **Stale:** Remove or update
- **Redundant:** Merge into the stronger perspective, remove the weaker

### Step 5: Check for Gaps

After curation, check coverage across these concern areas:

| Area | Example perspectives |
|------|---------------------|
| **Concurrency** | select! fairness, channel backpressure, shutdown ordering |
| **I/O boundaries** | byte order, transport abstraction, error context |
| **Config/deployment** | config surface consistency, env var design, enable gates |
| **Testing** | testable boundaries, mock seams, integration test coverage |
| **Naming/boundaries** | adapter boundaries, crate naming, key format stability |

If an area has zero perspectives but the project has had findings there, consider adding one.

If an area has 3+ perspectives, consider merging the most similar ones.

## Anti-Patterns to Prevent

### Overfit (too narrow)

Bad: "OPT3001 needs LE byte swap for INIT_CONFIG write"
Good: "Every I2C error message must include bus path and address for field debugging"

The bad example is a fix for one sensor. The good example is a principle that applies to any future sensor.

**Test:** Would this perspective help review a *different* adapter or sensor? If not, it's too narrow.

### Underfit (too generic)

Bad: "Write good error messages"
Good: "Every I2C error message must include bus path and address for field debugging on multi-bus systems"

The bad example is advice anyone would give. The good example is specific to iotkit-next's hardware context.

**Test:** Would a reviewer *without* iotkit-next context already check for this? If yes, it's too generic.

### Staleness (outdated)

Bad: Keeping "bravepi-adapter naming is ambiguous" after the rename is done
Good: Removing it, or evolving to "adapter names must distinguish transport boundary from device family"

**Test:** Does the current codebase still have the condition this perspective guards against? If not, update or remove.

### Clustering (all perspectives on one topic)

Bad: 4 of 5 perspectives are about channel send consistency
Good: Perspectives spread across concurrency, I/O, config, testing, naming

**Test:** If you removed all perspectives in one area, would the evaluator still be useful? If not, the other areas are under-represented.

## Format Rules

Perspectives files (one per review phase):
- `docs/eval/spec-review.md`
- `docs/eval/plan-review.md`
- `docs/eval/impl-spec-review.md`
- `docs/eval/impl-quality-review.md`

```markdown
# Codex Eval Perspectives

## Active Perspectives (max 10)

- **[YYYY-MM-DD]** [Perspective] — learned from: [source] — review by: [YYYY-MM-DD]
```

- Maximum 10 active perspectives
- Each has a creation date and review-by date (3 months out)
- When adding an 11th, remove the least valuable one
- No duplicates: check before adding

## Generalization Ladder

When a perspective comes from a specific fix, try to climb the generalization ladder:

```
Level 0 (fix):     "OPT3001 write needs LE bytes"
Level 1 (pattern): "I2C register writes must match sensor datasheet byte order"
Level 2 (principle): "Every I/O boundary must document its byte order contract"
```

**Target Level 1-2.** Level 0 is too narrow to help future reviews. Level 3+ ("write correct code") is too broad to be actionable.

## Integration

- **Called by:** `codex-eval-spec`, `codex-eval-plan`, `codex-eval-impl-spec`, `codex-eval-impl-quality` (after each review cycle)
- **Reads/writes:** Active Watchpoints section in `docs/eval/{spec,plan,impl-spec,impl-quality}-review.md`
- **Does NOT modify:** any source code or specs

## Quick Reference

| Trigger | Action |
|---------|--------|
| codex-eval-review completed | Run curation: new perspectives? existing ones validated? |
| Perspective review-by date reached | Evaluate: keep, update, or remove |
| User asks to audit perspectives | Full curation pass |
| Adding perspective #11 | Remove least valuable existing one |
| Finding not caught by any perspective | Gap analysis — add new perspective? |
| Same perspective triggered 3+ times | Consider it well-validated, extend review-by |
