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

Workflow authority, risk classification, vendor roles, effort policy, settlement, and the
persistent ledger are defined in `docs/development-workflow.md`. This skill supplies phase
mechanics and must not duplicate or weaken that policy.

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
REVIEW_MANIFEST=<manifest> scripts/codex.sh review <prompt-file> <label>
#   -> read-only sandbox; model/effort defaults live in scripts/codex.sh
#      (normal review defaults to high; high-risk work escalates per the canon)
#   -> output: /tmp/codex-runs/codex-<label>-review-<timestamp>.txt

# Cheaper mechanical pass: dial effort down
REVIEW_MANIFEST=<manifest> CODEX_EFFORT=medium scripts/codex.sh review <prompt-file> <label>
```

**`review` mode is ALWAYS read-only** — evaluation never mutates the tree. The wrapper
enforces this; do not hand-run `codex exec` with a writable sandbox for a review.
(Implementation is a separate path: `scripts/codex.sh impl`, workspace-write — that
is the codex-impl-loop skill, NOT eval.)
**Unique labels:** Each invocation uses a distinct `<label>` so outputs never collide.
Re-review = a fresh `codex.sh` call (no session reuse).

## Three-Vendor Review

Run the same artifact hash and review brief through Codex, Claude, and Grok in parallel.
The brief names the primary roles and mandatory common safety core from
`docs/development-workflow.md`; cross-role findings remain valid. A finding from one vendor
still gets triaged.

## Iteration Loop

1. Run `scripts/watchpoints.sh`; adjudicate anything expired before injecting guides
2. Dispatch the SAME prompt and `REVIEW_MANIFEST=<manifest>` to all three vendors in parallel.
3. Read all results and converge (multi-vendor agreement = high signal; one-raise = triaged)
4. If Critical or Important issues are found by any required vendor:
   - Green/Yellow and authority-settled semantic corrections: fix/reject autonomously
   - Red under `docs/development-workflow.md`: escalate as a bundled user packet
   - Lateral spread check: grep for same pattern workspace-wide, fix ALL instances
5. Re-run per the settlement rules in `docs/development-workflow.md`: addressees = owners of the C/I you
   fixed or rejected (a zero-vendor re-enters when a fix's semantic EFFECT reaches beyond
   the prescription, or when in doubt — when in doubt, send); exact transcription can reduce
   intermediate confirmation only
6. Repeat until zero unresolved Critical and zero Important across all required vendors
7. After any content change, run one final all-required-vendor round on the final hash
8. If verification finds new Critical/Important: fix and re-verify
9. Done when all required vendors are at zero Critical/Important **on the final tree hash**
   (a vendor's zero binds to the hash it reviewed, not to the artifact forever)

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
- Declaring done while any required vendor still has unresolved Critical/Important
- Auto-fixing a Red requirement/architecture issue without escalating
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
