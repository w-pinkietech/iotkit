---
name: codex-eval-common
description: Shared infrastructure for Codex evaluation skills (codex-eval-spec, codex-eval-plan, codex-eval-impl). Do not invoke directly — use the phase-specific skill instead.
---

# Codex Eval Common

Shared rules for all `codex-eval-*` skills. **Do not invoke this skill directly** — it is referenced by the phase-specific skills.

## Core Principle

```
THE IRON LAW: SELF-REVIEW IS NOT INDEPENDENT REVIEW.
If you analyzed the content, you cannot also be its evaluator.
You MUST invoke Codex. Your own analysis does NOT substitute.
```

**Violating the letter of this rule is violating the spirit of this rule.**

## CLI Usage

Invoke through the wrapper — `scripts/codex.sh` is the single source of truth for
model / flags / sandbox, so a model bump happens in one place (no stale constant to
rot in this doc):

```bash
# BEFORE authoring any eval prompt: check the guides you are about to inject
scripts/watchpoints.sh
#   -> lists expired/malformed watchpoints; adjudicate them first
#      (eval-perspectives-curator) — never inject a stale guide

# Write the review prompt to a file, then:
scripts/codex.sh review <prompt-file> <label>
#   -> read-only sandbox; model/effort defaults live in scripts/codex.sh
#      (review defaults to the deepest reasoning tier — reviews earn max reasoning)
#   -> output: /tmp/codex-runs/codex-<label>-review-<timestamp>.txt

# Cheaper mechanical pass: dial effort down
CODEX_EFFORT=high scripts/codex.sh review <prompt-file> <label>
```

**`review` mode is ALWAYS read-only** — evaluation never mutates the tree. The wrapper
enforces this; do not hand-run `codex exec` with a writable sandbox for a review.
(Implementation is a separate path: `scripts/codex.sh impl`, danger-full-access — that
is the codex-impl-loop skill, NOT eval.)
**Unique labels:** Each invocation uses a distinct `<label>` so outputs never collide.
Re-review = a fresh `codex.sh` call (no session reuse).

## Cross-Vendor Review (Fable)

Codex is one vendor. Run the **same** review prompt through a Fable review-max agent
in parallel (Agent tool, `subagent_type: review-max`) — identical lens, only the vendor
differs. Converge the two result sets: a finding both vendors raise is high-signal; a
finding only one raises still gets triaged. Same-vendor self-consistency bias is exactly
what the second vendor catches (memory: cross-vendor-review-same-lens). This is standard
from plan 5 onward, not optional.

## Iteration Loop

1. Run `scripts/watchpoints.sh`; adjudicate anything expired before injecting guides
2. Dispatch the SAME prompt to BOTH vendors in parallel:
   `scripts/codex.sh review` + Fable review-max (Agent tool)
3. Read both results and converge (both-raise = high signal; one-raise = still triaged)
4. If Critical or Important issues found — **by either vendor**:
   - Non-semantic (wording/structure/omission): fix autonomously
   - Semantic (architecture/requirements): escalate to user
   - Lateral spread check: grep for same pattern workspace-wide, fix ALL instances
5. Re-run BOTH vendors (fresh invocations, new label)
6. Repeat until zero unresolved Critical and zero Important **across both vendors**
7. Run verification pass (one more cross-vendor round)
8. If verification finds new Critical/Important: fix and re-verify
9. Done when BOTH vendors return zero Critical/Important

**Safety valve:** If same issue reappears after being fixed twice, escalate to user.

**Minor issues:** Noted but do not block completion.

## Evaluator Growth

After each review, evaluate Codex feedback for novel perspectives:

1. Which findings were genuinely novel blind spots?
2. Check against existing phase-appropriate perspectives file
3. Register if: project-specific, concrete, reproducible, applicable to future reviews
4. Do NOT register: generic advice, obvious points, incorrect observations

**Perspectives files:**
- `docs/eval/spec-review.md` (Active Watchpoints section)
- `docs/eval/plan-review.md` (Active Watchpoints section)
- `docs/eval/impl-spec-review.md` (Active Watchpoints section)
- `docs/eval/impl-quality-review.md` (Active Watchpoints section)

Max 10 per file. Review-by date: 3 months from creation.

## Red Flags — STOP

- About to review content yourself instead of invoking Codex
- "I can see the issues myself, no need for Codex"
- Summarizing Codex feedback without actually running `scripts/codex.sh review`
- Skipping iteration because "first review was thorough enough"
- Declaring done while the OTHER vendor still has unresolved Critical/Important
- Auto-fixing requirement/architecture issue without escalating
- Reusing a Codex session instead of starting fresh

## Output Format (standard for all phases)

```
For each perspective, state findings:
- **Issue:** What is the problem
- **Severity:** Critical / Important / Minor
- **Why it matters:** Impact if unaddressed
- **Suggestion:** Concrete improvement

If a perspective has no issues, state "No issues identified" for that perspective.
```
