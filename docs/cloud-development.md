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

## Configure the Cloud environment

Create/select the repository environment in [Codex environment settings](https://chatgpt.com/codex/settings/environments).
Use the following repo-owned command for both initial setup and optional cached-container
maintenance:

```bash
bash scripts/cloud-setup.sh
```

The setup script uses `rust-toolchain.toml` and `Cargo.lock`; it does not install local plugins or
copy local credentials. Put required setup-only secrets in the Cloud environment secret store,
never in prompts or the repository. Agent internet access is a separate environment decision and
remains off unless the task genuinely requires it.

## Local Main dispatching Cloud work

Cloud work runs from a remote branch, not from uncommitted local files. Create a candidate branch
such as `cloud/plan6-task5`, obtain the required authority to push it, and make sure local HEAD and
the live remote branch are identical. Then set the environment ID and submit:

```bash
export CODEX_CLOUD_ENV=<environment-id>
export CODEX_CLOUD_ALLOW_ARGV_PROMPT=1
scripts/codex-cloud.sh submit impl .review/plan6-task5-cloud.md plan6-task5 cloud/plan6-task5
```

The wrapper fails on a dirty, unpushed, stale, or mismatched branch. It snapshots the prompt and
exact query, then records a locally integrity-sealed receipt with the task ID, hashes,
environment, requested branch, locally observed remote commit, attempts, and timestamps. The
installed CLI cannot report the commit actually checked out by Cloud, so the receipt explicitly
records `cloud_base_verified=false`. Record the task ID/URL and receipt hash in the active ledger
before leaving the task unattended.

```bash
scripts/codex-cloud.sh status <task-id>
scripts/codex-cloud.sh collect <task-id> plan6-task5
scripts/codex-cloud.sh diff <task-id>
scripts/codex-cloud.sh verify-receipt <receipt-path>
```

Automatic `codex cloud apply` is deliberately not exposed by the wrapper: `exec --branch` accepts
a mutable branch name, while `list` and `status` do not return the checked-out commit. Inspect the
collected diff and reconcile useful work through the normal candidate implementation/review lane;
do not treat its requested base as proven provenance. A future automatic apply path requires
authoritative task metadata that binds the actual source commit.

Prompts must be non-secret regular files under `.review/` and are limited to 100,000 bytes. The
experimental CLI transports the query as a positional process argument; setting
`CODEX_CLOUD_ALLOW_ARGV_PROMPT=1` acknowledges that local same-host processes may observe it. Do
not put secrets or sensitive personal data in Cloud prompts. Best-of-N is disabled because
attempt-specific collection is not yet bound to a verified submission receipt;
`CODEX_CLOUD_ATTEMPTS` must remain `1`.

The wrapper writes an integrity-sealed `status=submission-pending` receipt before dispatch. If
dispatch is interrupted or its output does not contain exactly one task ID, retain that receipt
and output, use `list --env <environment-id> --json` and `status` to find the task, and record the
uncertain outcome in the active ledger. Do not resubmit automatically: that may spend twice.

Cloud `status` and `diff` do not expose a hash-bound final review body through the installed CLI.
Their receipts therefore state `settlement_eligible=false`. A Cloud review task is advisory unless
its exact answer is exported and bound through a separately reviewed procedure. The normal lane
uses a fresh local `scripts/codex.sh review` on the verified, uncommitted candidate.

## Temporary Main while away

A Cloud agent may temporarily act as Main for investigation, implementation, tests, and persistent
handoff updates. It must:

- work on `cloud/<slug>` or another explicit candidate branch, not integrate directly into
  `master`;
- read this guide and the active ledger, then verify their claims against Git;
- keep product Rust implementation agent-driven and run the relevant verification available in
  the Cloud environment;
- record branch/base/commit, Cloud task ID/URL, tests, remaining review debt, timing, and exact next
  action in the active ledger;
- leave the result unmerged and not `SETTLED` until a required hash-bound independent review is
  complete;
- never push, open a PR, merge, release, or spend additional Cloud attempts without the authority
  that applies to that external action.

Candidate reconciliation follows the normal gate without exception: verify the reconciled
artifact, obtain its manifest-bound independent review, resolve findings and reverify, commit,
then push or merge only with the separately applicable authority. A Cloud diff never moves commit
ahead of review.

When local Main returns, it verifies remote tips, candidate commits, the ledger, tests, and task
receipts. It also treats the Cloud source commit as unverified unless newer authoritative metadata
proves it. The Cloud conversation itself is evidence input, not restart authority.

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
