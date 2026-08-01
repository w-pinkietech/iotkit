---
type: Guide
title: "Issue-driven development workflow"
description: "Issue first, worktree, draft PR, change lanes, and lightweight design style."
language: en
translation_key: agents.workflow
status: stable
revision: 1
---

# Issue-driven workflow

Development is **issue-driven**. Product and repository changes go through a
GitHub issue before implementation. Ad-hoc commits on `master`, anonymous
branches without an issue, and PRs that do not close or reference a scoped issue
are out of process.

## Issue first

1. Open or reuse **one** GitHub issue for the intended outcome.
2. Record on the issue (or linked PR body for Standard+):
   - **Outcome** — what done looks like
   - **Non-goals / exclusions**
   - **Lane** — Fast / Standard / Full (see below)
   - **Verification** — how you will disprove the change
3. If scope grows materially, **stop and open a new issue** (or explicitly
   renegotiate the existing one). Do not silently expand the same branch.

Small agent tasks still use an issue. Draft issues are fine; empty “fix later”
issues are not a substitute for outcome and exclusions.

## Branch, worktree, and pull request loop

Every development task maps to one GitHub issue.

1. Update `master`.
2. Create `agent/issue-<number>-<slug>` and
   `.worktrees/issue-<number>-<slug>`.
3. Work and verify **only** in that worktree.
4. Commit intentionally, push the branch, and open a **draft PR that closes the
   issue** (`Closes #N`).
5. Stop for human review. Apply feedback on the same branch and PR.
6. Merge **only after explicit approval**. After confirmed merge, remove the local
   worktree and branch.

Keep the diff inside the issue scope.

Branch push and draft PR creation are pre-authorized completion steps for this
loop. Merge, release, destructive actions, paid actions, and other external
effects still require explicit approval.

## Change lanes

Default is lightweight. Choose the lightest lane that covers realistic risk.
Do not open with a long design/plan pipeline unless the work is Full or the user
asks for it.

For every product behavior change, add or update the closest focused test before
implementation.

| Lane | Use for | Required process |
|---|---|---|
| Fast (default for most work) | local bug, refactor, docs, CI, configuration, or small feature without contract/security/custody/migration/restore impact | **issue** with outcome and exclusions; focused test when behavior changes; focused verification; PR closes issue |
| Standard | multiple packages, a new internal boundary, several credible implementations, or product UX with real design choices | **issue** (or PR body) holds a short decision note: goal, non-goals, chosen approach, verification; one review; proportional tests. No separate plan file under `docs/superpowers/` |
| Full | public wire contract, auth/secrets, custody/data loss, DB migration, backup/restore/rollback, destructive or expensive compatibility decisions | **issue** plus short explicit design in the owning current authority (issue, OKF, or contract docs—not a historical plan tree); tests first; independent review; broad verification. An implementation plan only when the work has many ordered slices or irreversible steps |

Quality that stays in every lane: **scoped issue**, no silent data loss, no
secret leakage, focused tests for behavior changes, human merge approval, and
verification matched to risk.

### Writing style (lightweight)

Borrow the useful shape of historical sprint designs without growing
`docs/superpowers/`:

- **Always:** goal, non-goals, verification (on the issue).
- **Standard+:** short decision note (chosen approach).
- **Full only when needed:** longer design and optional ordered plan.

### Process weight and optional harnesses

- Do not create new files under `docs/superpowers/` for ongoing work. That tree
  is historical lineage. Put lasting product decisions into `docs/okf/`,
  contracts, code, and tests; keep short process decisions on the issue/PR.
- Heavy multi-step harnesses (long specs, checkbox plans, mandatory
  subagent-per-task execution) are optional Full aids when risk or size
  justifies them—not the default development loop.
- Process plugins and skills are optional unless the user names one or
  [`AGENTS.md`](../AGENTS.md) requires one. They do not override the user
  request or repository rules.
- Prefer current code, executable tests, and existing authority over new process
  documents. Do not create a spec only to repeat an existing decision.
- Use repository-local independent review by default. Call an external review
  model or service only when the user explicitly requests it.

Return to [`AGENTS.md`](../AGENTS.md).
