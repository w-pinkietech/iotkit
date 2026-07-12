# Codex Cloud Development

Status: **Authoritative restart guide** (2026-07-12)

`iotkit-next` is the complete development unit. A single clone contains product code, the
design authority, workflow authority, implementation plans, review state, and the current
restart pointer. Do not require a sibling checkout or vendor-specific memory to continue work.

## Start or resume

1. Read [`../AGENTS.md`](../AGENTS.md) for repository rules and role boundaries.
2. Read [`superpowers/active-ledger.md`](superpowers/active-ledger.md) for the verified phase,
   artifact/base HEAD, review debt, user decisions, and next executable work.
3. Read [`development-workflow.md`](development-workflow.md) for risk classification,
   independent review, settlement, timing, commit, and external-effect rules.
4. Read only the design decisions named by the active task from [`redesign/`](redesign/), plus
   [`redesign/terminology.md`](redesign/terminology.md) and the affected rows in
   [`redesign/responsibility-ledger.md`](redesign/responsibility-ledger.md).
5. Verify the ledger against `git status`, `git log -1`, files, and tests before acting.

Historical handoffs and review reports explain prior decisions but are not restart authority.
If they disagree with Git, the design corpus, the workflow, or the active ledger, use that
authority order and record the correction.

## One-repository invariant

- Product decisions and their implementation land in this repository. Update
  `docs/redesign/` in the same branch or change series as the code/spec they govern.
- Persistent execution context belongs in `docs/superpowers/active-ledger.md`; durable workflow
  rules belong in `docs/development-workflow.md`; product rationale belongs in
  `docs/redesign/`. Do not put required context only in chat, `/tmp`, or model memory.
- The former `iotkit-redesign` repository is historical and read-only after the migration.
  Do not copy changes back or treat it as a competing authority.
- References to sibling repositories such as YokaKit or monojoh-authority are evidence inputs,
  not hidden prerequisites. Record any required conclusion inside this repository before it
  becomes implementation authority.

## Before ending a session

Update the active ledger with the real artifact/base HEAD, completed verification and review
receipts, unresolved Red decisions, exact next executable work, and measured task timing. Commit
that state with the work it describes or in the immediately following ledger commit. Push/PR/
merge/release still require the authority stated in the workflow or an explicit user decision.
