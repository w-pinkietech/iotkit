# Single-Repository Context Migration

Date: 2026-07-12

Status: **approved; implementation under review**

Risk: **Large / Red** because it changes design and workflow authority location

## Mission brief

Make `iotkit-next` the complete unit of development so a single local or Codex Cloud clone can
resume work with the same product design, implementation state, workflow, and next-task context.

Acceptance outcome:

- `docs/redesign/` exists inside `iotkit-next` at the exact content represented by
  `iotkit-redesign` commit `f10c2a5`;
- the imported corpus retains its file history through split commit
  `28fe683359c9f39843e6a828956e4f1e6a53f299` as a merge parent;
- active instructions and links require no sibling checkout;
- one documented restart path leads from `AGENTS.md` to the active ledger, workflow, and scoped
  design decisions;
- the former design repository clearly points to the new authority and accepts no new design
  work after the migration;
- Rust product behavior and Plan 6 Task 5 scope are unchanged.

The user approved this consolidation before Task 5 on 2026-07-12. Approval includes pushing the
old repository's final design commit, importing its history, publishing the consolidated state,
and freezing the old design repository after the pointer is published.

## Constraints and non-goals

- Preserve the original design history; do not flatten it into an unexplained file copy.
- Do not import vendor memory, review scratch files, unrelated sibling repositories, or secrets.
- Import the tracked root `rewrite-prep.md` as historical evidence because the design terminology
  links to it; it is not a competing authority.
- Do not create two writable copies of the design authority or an automatic bidirectional sync.
- Do not use a Git submodule: Cloud work must be atomic in one repository and one PR/change
  series.
- Do not change product contracts while moving their documents.
- Keep historical reports truthful about their original environment, but label superseded paths
  and point current readers to the restart authority.

## Authority transition

| State | Writable design authority | Implementation authority | Restart authority |
|---|---|---|---|
| Before | `iotkit-redesign/docs/redesign/` | `iotkit-next` | split across both repositories and model memory |
| Migration | old authority fixed at `f10c2a5`; unpublished import is not yet authoritative | `iotkit-next` | local migration record |
| After publication | `iotkit-next/docs/redesign/` only | `iotkit-next` | `AGENTS.md` → cloud guide → active ledger |

The cutover becomes effective when the consolidated `iotkit-next` commit is pushed and the old
repository's pointer is pushed. If publication fails between those actions, the old repository
remains frozen at `f10c2a5` and work stops until both pointers are observable; no design edits are
made in either location during that interval.

Rollback before publication is deliberately scoped so unrelated or user-owned files survive:

1. Record `git status --short` and verify the three mission-created untracked paths against the
   frozen review manifest: `docs/cloud-development.md`, this migration record, and
   `rewrite-prep.md`.
2. Run `git merge --abort` to restore tracked files and the pre-migration index.
3. Remove only those three paths, only if each was absent at the recorded `f8186ea` base and its
   current blob still matches the frozen migration artifact. Never use a repository-wide
   `git clean` for this rollback.
4. Verify `git status`, `git rev-parse HEAD`, absence of `MERGE_HEAD`, and the restored active
   authority references before resuming work.

After publication, rollback means reverting the authority change in a new reviewed commit; it
does not resume two-way edits or rewrite published history.

## Context placement

- Product rationale and settled decisions: `docs/redesign/`.
- Workflow rules: `docs/development-workflow.md`.
- Current phase, receipts, user decisions, exact next work, and timing: active ledger.
- Structural/code placement rules: `docs/architecture.md`.
- Historical handoffs and review reports: evidence only, never restart authority.

The old `.claude/memory/` contains useful history but is vendor-specific and partly stale. Any
durable rule it once carried is already represented by the workflow, `AGENTS.md`, or active
ledger. It is intentionally not imported.

## Failure and adversarial review

1. **Concurrent old-repository edit:** freeze old design changes at `f10c2a5`; reject later work
   there and reapply it through a reviewed `iotkit-next` change.
2. **Power/session loss during import:** no authority changes until publication; Git merge state
   and the migration record make the incomplete state observable.
3. **One push succeeds and the other fails:** stop new work, keep the old corpus frozen, and retry
   the missing pointer publication. Do not create divergent edits.
4. **Stale relative link silently escapes review:** search for old repository names, absolute
   local paths, and parent-directory design references; resolve every active occurrence and check
   repository-local Markdown targets.
5. **Cloud opens only `iotkit-next`:** `AGENTS.md` and the cloud guide must reach all authorities
   without local memory, `/tmp`, or sibling repositories.
6. **Someone follows the old repository later:** its root README states the cutover date, frozen
   commit, canonical destination, and prohibition on new changes.
7. **Imported history is lost:** the final consolidation commit has the split design history as a
   second parent; verify ancestry and `git log --follow` on an imported decision.
8. **Secret or local-only state is copied:** history import is restricted to tracked
   `docs/redesign/`, plus the tracked and referenced `rewrite-prep.md` evidence file;
   `.claude/memory/`, `.review/`, `/tmp`, credentials, and other root files are excluded.

## Verification

- compare old `f10c2a5:docs/redesign` with the imported `docs/redesign` tree;
- scan active files for sibling design paths, old authority URLs, and machine-absolute paths;
- validate repository-local Markdown links in changed files;
- run `git diff --check`, `scripts/verify.sh`, and an independent fresh Codex Sol/high review;
- before the final confirmation, stage every migration file and verify the staged set equals the
  final manifest (the final manifest includes the active ledger); a plain merge commit must not be
  able to omit the Cloud guide, migration record, reference updates, or `rewrite-prep.md`;
- verify the final commit has both the prior implementation HEAD and split design history as
  parents;
- after push, verify both repositories' status and remote tips, then freeze the old repository.
