# Superpowers-only Development Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the repository-specific review bureaucracy and make standard Superpowers skills the only development-process framework.

**Architecture:** Keep product authority, safety invariants, verification economy, native model-role intent, and optional Codex Cloud operation. Delete the parallel ledger/hash-settlement/review system, then rewrite active entry points so historical files cannot reactivate it.

**Tech Stack:** Markdown, Bash, Git, Superpowers skills

## Global Constraints

- Preserve all existing uncommitted Gateway + Site Server product-design work.
- Do not change Rust product code or product behavior.
- Keep `docs/redesign/`, `docs/architecture.md`, `scripts/check-layers`, and optional Codex Cloud helpers authoritative in their existing scopes.
- Keep the invariants: no secret exposure, no silent data loss, and all mutations through R14 dispatch.
- Keep native role intent: Sol/high for Main and reviewer; Luna/max for implementer and executor.
- Push, PR, merge, release, spending, and destructive external actions remain separately authorized.
- Make one intentional cleanup commit, as required by the approved design.

---

### Task 1: Remove the duplicate workflow framework

**Files:**
- Delete: `docs/development-workflow.md`, `docs/superpowers/active-ledger.md`, `docs/superpowers/PLAN6-DESIGN-READY.md`, `docs/superpowers/HANDOFF-2026-07-11-to-codex-driver.md`, and `codex-review.md`
- Delete: `docs/eval/*.md`
- Delete: `.claude/skills/codex-eval-common/SKILL.md`, `.claude/skills/codex-eval-spec/SKILL.md`, `.claude/skills/codex-eval-plan/SKILL.md`, `.claude/skills/codex-eval-impl/SKILL.md`, `.claude/skills/codex-impl-loop/SKILL.md`, and `.claude/skills/eval-perspectives-curator/SKILL.md`
- Delete: `scripts/review-manifest.sh`, `scripts/review-receipt.sh`, `scripts/check-codex-events.sh`, `scripts/codex.sh`, `scripts/claude-review.sh`, `scripts/grok-review.sh`, `scripts/test-codex.sh`, `scripts/trailer.sh`, `scripts/watchpoints.sh`, `scripts/check-codex-role-config.sh`, and `scripts/test-codex-role-config.sh`

**Interfaces:**
- Consumes: the approved deletion inventory in `docs/superpowers/specs/2026-07-13-superpowers-only-development-workflow-design.md`.
- Produces: a repository with no live custom ledger, hash settlement, duplicate evaluator skills, or mandatory review wrappers.

- [ ] **Step 1: Confirm each deletion target is tracked and belongs to the approved inventory**

Run `git ls-files` over the paths above. Expected: all deletion targets exist; retained files including `scripts/check-layers`, `scripts/codex-cloud.sh`, `scripts/test-codex-cloud.sh`, and `scripts/cloud-setup.sh` remain distinguishable.

- [ ] **Step 2: Delete the approved files with `apply_patch`**

Expected: `git status --short` reports the targets as deleted and no product files are modified.

### Task 2: Rewrite active workflow entry points

**Files:**
- Modify: `AGENTS.md`, `CLAUDE.md`, `docs/superpowers/README.md`, `docs/cloud-development.md`, `scripts/verify.sh`
- Modify in the original working tree only: `docs/superpowers/specs/2026-07-13-minimum-gateway-site-server-design.md`

**Interfaces:**
- Consumes: standard installed Superpowers skills and the retained product/design authorities.
- Produces: concise active instructions, an optional Cloud guide without settlement coupling, a product-only verification script, and a current design status that no longer waits for hash settlement.

- [ ] **Step 1: Replace `AGENTS.md` and `CLAUDE.md` workflow sections**

Retain project context, product authority, the three invariants, verification economy, Main/worker commit boundary, external publication boundary, and native model intent. State the standard flow directly: `brainstorming -> written design/user review -> writing-plans -> TDD implementation -> requesting/receiving code review -> verification-before-completion -> finishing-a-development-branch`.

- [ ] **Step 2: Rewrite `docs/superpowers/README.md`**

State that installed Superpowers skills are process authority; specs/plans are written decisions and historical records; product authority remains `docs/redesign/`; structural authority remains `docs/architecture.md`; historical workflow references are not active instructions.

- [ ] **Step 3: Decouple `docs/cloud-development.md` from deleted workflow state**

Keep Cloud setup, branch isolation, secret handling, receipts for operational diagnosis, explicit external-action authority, and local inspection. Remove ledger, settlement, mandatory review-wrapper, Main product-code prohibition, and hash-bound merge requirements. Return Cloud candidates to the normal Superpowers review and verification flow.

- [ ] **Step 4: Reduce `scripts/verify.sh` to product checks**

The executable sequence must be `cargo fmt --all --check`, `scripts/check-layers`, `cargo test --workspace`, then `cargo clippy --workspace --all-targets -- -D warnings`. The success message must not require independent hash review.

- [ ] **Step 5: Update the current Gateway + Site Server design status**

In the original worktree, set status to `User-approved written design; awaiting user written-spec review`; remove Large/Red classification and `docs/development-workflow.md` from its header. Do not stage that existing product-design work in this cleanup commit.

### Task 3: Focused verification and cleanup commit

**Files:**
- Verify: all files changed by Tasks 1 and 2
- Commit: only workflow cleanup files and this plan

**Interfaces:**
- Consumes: simplified entry points and retained product boundary checker.
- Produces: one revertible cleanup commit and an explicit record of skipped product tests.

- [ ] **Step 1: Run `git diff --check` and `bash -n scripts/verify.sh`**

Expected: both exit 0 with no output.

- [ ] **Step 2: Run `scripts/check-layers`**

Expected: exit 0 and a layer-check success message.

- [ ] **Step 3: Search active entry points for removed machinery**

Run `rg` for `development-workflow`, `active-ledger`, `PLAN6-DESIGN-READY`, `HANDOFF-2026-07-11`, `SETTLED`, `REVIEW_MANIFEST`, review manifest/receipt names, evaluator skills, review wrappers, watchpoints, and `Review-hash` in `AGENTS.md`, `CLAUDE.md`, `README.md`, `docs/cloud-development.md`, `docs/superpowers/README.md`, and `scripts/verify.sh`. Expected: exit 1 with no matches.

- [ ] **Step 4: Inspect staged scope**

Run `git status --short`, `git diff --cached --check`, and `git diff --cached --stat`. Expected: only approved cleanup paths are staged.

- [ ] **Step 5: Commit**

Run `git commit -m "chore: simplify development workflow"`. Expected: one cleanup commit.

- [ ] **Step 6: Record proportional verification**

Report that Rust workspace tests and Clippy were not run because no Rust source, Cargo metadata, or product behavior changed; the retained layer checker was executed directly.
