# Risk-Proportionate Verification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `AGENTS.md` require risk-proportionate verification and omission of clearly irrelevant checks because time is finite.

**Architecture:** Add one shared verification-economy section that applies to Main and workers, then replace the worker-only unconditional full-suite rule with a conditional rule. Preserve all product invariants, independent-review requirements, and settlement mechanics.

**Tech Stack:** Markdown, Git whitespace validation.

## Global Constraints

- Modify only `AGENTS.md`.
- Do not weaken checks tied to a plausible material failure.
- Do not run Rust tests for this agent-instruction-only change.
- Do not push without separate user authority.

---

### Task 1: Add risk-proportionate verification guidance

**Files:**
- Modify: `AGENTS.md`, between `Invariants` and `Worker mode rules`, and the existing worker verification bullet.
- Test: `AGENTS.md` via focused text inspection and `git diff --check`.

**Interfaces:**
- Consumes: existing shared invariants and Worker/Main role separation.
- Produces: a shared verification-economy rule plus a non-contradictory worker completion rule.

- [ ] **Step 1: Add the shared policy**

Insert before `## Worker mode rules(codex タスク)`:

```markdown
## Verification economy（時間は有限）

- 検証は変更範囲、リスク、現実的な失敗経路に比例させる。検査数を増やすこと自体を目的にしない。
- 結果が変更の信頼性を実質的に高めないと明らかに判断できる検査は省略する。
- 通常なら実行する検査を省略した場合、完了報告に省略した検査と、変更へ無関係と判断した具体的理由を書く。
- Rust製品動作、層境界、認証、秘密情報、data loss/custody、並行処理、外部作用、review/receipt provenanceに関係する検査は、その失敗可能性を除外できない限り省略しない。
- 影響範囲が不明な場合は検証を広げる。「時間は有限」は未解決の重大リスクを受け入れる理由にしない。
```

- [ ] **Step 2: Remove the contradictory unconditional worker rule**

Replace the existing unconditional `scripts/verify.sh` bullet with:

```markdown
- Rust製品動作へ影響する、または関連影響を除外できないタスクは、完了報告前に `scripts/verify.sh`（fmt + 層規則 check-layers + `cargo test --workspace` + clippy `-D warnings`）を通す。文書のみ・限定的な設定変更など、製品動作へ影響しないことを説明できるタスクはfocused checksに限定し、省略理由を報告する。
```

- [ ] **Step 3: Verify only the changed instruction surface**

Run:

```bash
git diff --check
git diff -- AGENTS.md
```

Expected: whitespace check is silent; the diff contains only the shared section and worker-rule replacement. Explicitly omit `scripts/verify.sh` because Markdown agent instructions cannot affect compiled Rust or runtime behavior.

- [ ] **Step 4: Commit locally**

```bash
git add AGENTS.md docs/superpowers/plans/2026-07-13-risk-proportionate-verification.md
git commit -m "docs: make verification risk proportionate"
```

Do not push.
