# Issue-driven workflow

Development is issue-driven. Product and repository changes go through a
GitHub issue before implementation.

## Issue first

1. Open or reuse one GitHub issue for the intended outcome.
2. Record the outcome, non-goals or exclusions, lane, and acceptance evidence.
3. If scope grows materially, stop and open a new issue or explicitly
   renegotiate the existing one. Do not silently expand the same branch.

## Branch, worktree, and pull request loop

Every development task maps to one GitHub issue.

1. Update `master`.
2. Create `agent/issue-<number>-<slug>` and
   `.worktrees/issue-<number>-<slug>`.
3. Work and verify only in that worktree.
4. Judge product-doc impact. If lasting product facts change, update matching
   `docs/product/` files in the same worktree. Otherwise record a concrete
   no-update reason on the PR.
5. Commit intentionally, push the branch, and open a draft PR that closes the
   issue. List updated product-doc paths or the no-update reason in the body.
6. Stop for human review. Apply feedback on the same branch and PR.
7. Merge only after explicit approval. After confirmed merge, remove the local
   worktree and branch.

Keep the diff inside the issue scope. Create a separate issue when the scope
changes materially.

Branch push and draft PR creation are pre-authorized completion steps for this
loop. Merge, release, destructive actions, paid actions, and other external
effects still require explicit approval.

## Change lanes

Default is lightweight. Choose the lightest lane that covers realistic risk.
Do not open with a long design or plan pipeline unless the work is Full or the
user asks for it.

For every product behavior change, add or update the closest focused test
before implementation.

| Lane | Use for | Required process |
|---|---|---|
| Fast | local bug, refactor, docs, CI, configuration, or small feature without contract/security/custody/migration/restore impact | issue outcome and exclusions; focused test when behavior changes; focused verification; PR. Update `docs/product/` ja+en when operator-, integrator-, or contract-visible facts change; otherwise record a no-update reason |
| Standard | multiple packages, a new internal boundary, several credible implementations, or product UX with real design choices | short decision note with goal, non-goals, chosen approach, and verification; one review; proportional tests; lasting decisions in `docs/product/` or paired contract artifacts |
| Full | public wire contract, auth/secrets, custody/data loss, DB migration, backup/restore/rollback, destructive or expensive compatibility decisions | short explicit design in the owning current authority; tests first; independent review; broad verification; an implementation plan only for many ordered slices or irreversible steps; ship corpus, schema/types, fixtures, and tests as one contract unit |

Every lane keeps the scoped issue, product invariants, focused behavior tests,
product-doc impact, human merge approval, and risk-matched verification.

### Process weight and optional harnesses

- Do not create new files under `docs/superpowers/` for ongoing work. That tree
  is historical. Put lasting decisions in `docs/product/`, contracts, code, or
  tests; keep transient decisions on the issue or PR.
- Heavy multi-step harnesses are optional Full aids, not the default loop.
- Process plugins and skills are optional unless the user names one or
  [`AGENTS.md`](../AGENTS.md) requires one. They do not override the user
  request or repository rules.
- Prefer current code, executable tests, and existing authority over new
  process documents. Do not create a spec only to repeat an existing decision.
- Use repository-local independent review by default. Call an external review
  service only when the user explicitly requests it.

Return to [`AGENTS.md`](../AGENTS.md).
