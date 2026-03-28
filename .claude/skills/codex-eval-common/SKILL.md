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

```bash
codex exec -m gpt-5.4 -c reasoning_effort=xhigh \
  -o /tmp/codex-eval-{phase}-{review_id}-iter{n}.txt \
  -s read-only \
  "$(cat <<'PROMPT'
{prompt content}
PROMPT
)"
```

**MUST use:** `-s read-only`
**MUST NOT use:** `--full-auto`, `-s workspace-write`, `-s danger-full-access`
**Unique output paths:** Every invocation uses a unique file. Never reuse paths.
**Non-git directories:** Add `--skip-git-repo-check`.
**Fresh sessions:** Every iteration is a new `codex exec` invocation. No session reuse.

## Iteration Loop

1. Run `codex exec` with phase-appropriate prompt
2. Read result
3. If Critical or Important issues found:
   - Non-semantic (wording/structure/omission): fix autonomously
   - Semantic (architecture/requirements): escalate to user
   - Lateral spread check: grep for same pattern workspace-wide, fix ALL instances
4. Re-run `codex exec` (fresh session)
5. Repeat until zero Critical and zero Important
6. Run verification pass (one more `codex exec`)
7. If verification finds new Critical/Important: fix and re-verify
8. Done when Codex returns zero Critical/Important

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
- Summarizing Codex feedback without actually running `codex exec`
- Skipping iteration because "first review was thorough enough"
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
