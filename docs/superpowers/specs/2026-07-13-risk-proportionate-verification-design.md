# Risk-Proportionate Verification Design

Status: **Approved in principle; written-spec review pending** (2026-07-13)

## Goal

Make verification proportional to the changed behavior and credible failure modes. Time is finite:
agents must not run checks that are clearly irrelevant merely to maximize the number of checks.

## Policy

- Before verification, identify the changed surfaces, affected boundaries, and plausible
  regressions.
- Run the smallest set of checks that provides evidence for those risks.
- Omit a check when its result cannot materially increase confidence in the changed behavior.
- State every omitted customary check and the concrete reason it is irrelevant in the completion
  report. Silence is not justification.
- Do not weaken checks relevant to Rust product behavior, layer boundaries, authentication,
  secrets, custody/data loss, concurrency, external effects, or review/receipt provenance.
- Documentation-only and narrowly scoped configuration changes normally use focused parsing,
  syntax, link, diff, or configuration probes instead of the complete Rust workspace suite.
- If impact is uncertain, broaden verification or run the full gate. “Time is finite” does not
  justify accepting an unresolved material risk.

## AGENTS.md changes

Add a shared `Verification economy` section before the worker rules. Replace the worker rule that
unconditionally requires `scripts/verify.sh` with a risk-proportionate rule:

- run `scripts/verify.sh` when the task affects Rust product behavior or when relevant impact
  cannot be excluded;
- otherwise run focused checks and report what was omitted and why.

The Main-mode section continues to defer detailed workflow mechanics to
`docs/development-workflow.md`, but the shared rule applies to both Main and workers. Existing
invariants, independent-review requirements, and settlement rules remain unchanged.

## Verification of this documentation change

Inspect the rendered diff and run `git diff --check`. Do not run the Rust workspace tests because
the change affects agent instructions only and cannot alter compiled code or runtime behavior.

## Success criteria

- `AGENTS.md` explicitly says time is finite and irrelevant checks should be omitted.
- Agents must justify omitted customary checks.
- The rule does not provide a pretext for skipping checks tied to a plausible material failure.
- The old unconditional full-verification worker rule no longer contradicts the new policy.
