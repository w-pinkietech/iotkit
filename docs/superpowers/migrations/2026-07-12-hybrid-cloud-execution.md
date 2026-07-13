# Hybrid Local/Cloud Execution

Date: 2026-07-12

Status: **Historical migration record; completed and superseded as workflow guidance**

This document records the 2026-07-12 migration. References below to the former active ledger,
risk classification, review settlement, or workflow files are historical and are not current
instructions. Current Cloud operation is described by `docs/cloud-development.md`.

Risk: **Large / Red** because this changes workflow authority, review provenance, and external
Cloud-task dispatch mechanics.

## Mission brief

Allow the user's computer to act as the normal Main agent while dispatching bounded implementation
or advisory-review work to Codex Cloud, and allow useful candidate work to continue from the Cloud
UI while the computer is unavailable. Every path must converge back to the same Git and active
ledger without weakening independent review or silently publishing unreviewed work.

Acceptance outcome:

- a repo-owned wrapper submits, lists, inspects, and collects Cloud tasks without relying
  on local plugins inside the Cloud container;
- submission proves the selected remote branch matched local clean HEAD immediately before
  dispatch and records that local intent, the exact query, task ID, hashes, environment, branch,
  attempt count, and timestamps without claiming the Cloud checkout commit was verified;
- Cloud environment setup is reproducible from a repo-owned script;
- local-Main and Cloud-temporary-Main journeys have explicit authority and interruption rules;
- Cloud work stays on a candidate branch until the existing bound independent-review settlement
  completes;
- a task URL, status, or diff alone never masquerades as a bound review result;
- Plan 6 product contracts and Task 5 scope are unchanged.

The user approved adjusting the workflow for this hybrid use on 2026-07-12. This approves local
harness implementation and review, but does not grant standing authority for future Cloud task
submission, candidate-branch push, PR, merge, or release.

## Official and observed constraints

Official Codex documentation says a Cloud task checks out a selected repository branch/commit,
runs environment setup, reads `AGENTS.md`, executes terminal commands, and returns an answer plus
diff. `codex cloud` is currently Experimental and exposes `exec`, `status`, `list`, `diff`, and
`apply`. Local inspection confirms those commands and `--attempts` best-of-N, but `exec` accepts
only a mutable branch name and `list`/`status` expose no checked-out commit. The wrapper therefore
does not expose automatic apply and marks the Cloud base unverified.

Observed on the installed CLI: `codex cloud list --json` returns task metadata, while
`codex cloud status <task>` reports lifecycle/summary but not the final answer body. Therefore an
ordinary Cloud review cannot satisfy the existing result-body hash/receipt contract through the
CLI alone. Cloud review is advisory unless its exact final answer is separately exported and
bound under a reviewed procedure. The default hybrid lane retains fresh local Codex review for
settlement.

## State machine

1. **Ready:** clean local candidate branch; local HEAD equals the live remote branch tip.
2. **Submission pending:** before the external call, an integrity-sealed receipt records the
   prompt/query snapshot, environment, requested branch, locally observed commit, and attempts.
3. **Submitted:** exactly one parsed task ID finalizes the receipt. It records
   `cloud_base_verified=false`; requested branch/commit is intent evidence, not checkout proof.
4. **Running/ready/error:** inspect with Cloud status. The harness permits exactly one attempt
   because attempt-specific collection is not yet provenance-bound.
5. **Collected:** status and diff are saved and hashed locally. This is provenance for candidate
   work, explicitly `settlement_eligible=false`.
6. **Candidate reconciled:** Main inspects the diff and re-enters useful work through the normal
   implementation/review lane. Automatic apply is unavailable until actual-base metadata exists.
7. **Candidate verified:** Main runs normal tests, then a fresh required reviewer receives the
   exact uncommitted candidate artifact under the manifest/receipt workflow.
8. **Settled and committed:** after zero unresolved C/I, Main reconciles/reverifies and commits;
   push/merge occurs only with the authority that applies to that external action.

Cloud-temporary-Main may edit, test, and record progress on `cloud/<slug>` or another explicitly
named candidate branch. Without a bound required review it must leave the branch/PR as candidate,
mark review debt in the active ledger, and must not claim `SETTLED`, merge to `master`, publish, or
release. Local Main resumes by verifying remote tips, commits, active-ledger facts, tests, and task
IDs rather than trusting the Cloud conversation.

## User journeys

### At the computer

1. Create and push a candidate branch with explicit push authority.
2. Set the configured Cloud environment ID in `CODEX_CLOUD_ENV`.
3. Submit an implementation prompt through the wrapper and record its task receipt in the ledger.
4. Monitor/collect, inspect the diff, and reconcile useful work through the normal candidate lane.
5. Verify, run bound independent review, reconcile/reverify, commit, then perform only an
   authorized push/merge.

### Away from the computer

1. Open the same repository/environment in Codex Cloud and read `AGENTS.md`, the Cloud guide, and
   active ledger.
2. Work only on a candidate branch; run `scripts/verify.sh` where the environment permits.
3. Record exact next work, tests, task URL/ID, branch, commit/base, and review debt in the ledger.
4. Leave unbound work unmerged. Local Main later performs the normal reality check and settlement.

## Failure and adversarial review

1. **Dirty or unpushed local state:** submission fails before external dispatch; Cloud never sees
   a falsely described base.
2. **Stale remote-tracking ref:** submission compares against `git ls-remote`, not only the local
   tracking ref, and fails on mismatch.
3. **Branch moves or dispatch race:** requested commit is retained only as local intent and the
   receipt says the Cloud base is unverified; automatic apply is disabled.
4. **Task errors, interruption, or ambiguous output:** the pre-submission receipt remains pending;
   reconcile with filtered list/status and the ledger before any retry to avoid duplicate spend.
5. **More than one attempt requested:** submission fails until attempt-specific collection and
   receipt binding are implemented and reviewed.
6. **Automatic apply requested:** the wrapper rejects it until authoritative task base metadata is
   available; collected output remains candidate input only.
7. **Cloud review has no exportable answer:** status/diff receipt is marked settlement-ineligible;
   required review remains owed.
8. **Cloud agent tries to push/merge:** repo guidance forbids external effects without authority;
   candidate branch and active-ledger debt make the incomplete state visible.
9. **Cached environment is stale:** maintenance reruns repo setup/fetch; reset the environment
   cache if toolchain or dependency state remains incompatible.
10. **Secrets and argv exposure:** prompts are non-secret files under `.review/`; the user must
    acknowledge that the installed CLI places the query in process arguments. Cloud secrets
    belong in the environment secret store and are never passed in prompts.
11. **Experimental CLI diagnostic spill:** commands are serialized by a guardian-held kernel
    `flock`; the guardian outlives a hard-interrupted wrapper until its CLI exits. Generated
    `error.log` is moved into the private output directory; an owner marker lets the next lock
    holder recover it. A pre-existing unrelated file is never
    overwritten. Diagnostics are not committed or used as settlement evidence.
12. **Receipt substitution:** receipts use HMAC-SHA-256 plus status-specific exactly-once schema
    validation. This protects against accidental or untrusted-file edits, not a malicious process
    running as the same OS user.

## Verification

- shell syntax and a fake-CLI/local-bare-remote test cover local remote-tip checks, prompt/query
  snapshots, sealed pending/final receipts, collect, dirty/unpushed rejection, interruption/stale
  locks including an orphaned CLI after wrapper death, live-remote movement, ambiguous output,
  argv acknowledgement, and
  automatic-apply rejection;
- setup script syntax and non-secret behavior are reviewed;
- existing `scripts/verify.sh` stays green and includes the wrapper test;
- a fresh Sol/high review checks external-effect authority, provenance, result-body limitations,
  candidate-branch recovery, user journey, and residual C/I.
