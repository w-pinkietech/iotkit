# Codex Native Model Routing Design

Status: **User-approved design direction; implementation pending written-spec review**

Date: 2026-07-13

## 1. Mission brief

Configure Codex CLI so the Main session remains `gpt-5.6-sol/high`, review work uses
`gpt-5.6-sol/high`, and implementation or execution work uses `gpt-5.6-luna/max`. Main chooses
the appropriate named agent role when dispatching native Codex subagents. Existing process-isolated
dispatch remains aligned with the same routing and continues to provide manifest-bound settlement
evidence where required.

Acceptance outcome:

- a new trusted-repository Codex CLI session loads named `implementer`, `executor`, and `reviewer`
  roles without configuration errors;
- `implementer` and `executor` resolve to Luna/max;
- `reviewer` and Main resolve to Sol/high;
- `scripts/codex.sh impl` defaults to Luna/max and `scripts/codex.sh review` defaults to Sol/high;
- explicit wrapper overrides remain available and are recorded in receipts;
- independent-review settlement remains fresh-session, read-only, manifest-bound, and fail-closed;
- workflow documentation and the active ledger no longer prescribe the superseded Terra/medium or
  Sol/medium defaults.

This is a Large/Red workflow-authority and review-harness change under
`docs/development-workflow.md`. The user approved the model-routing value decision on 2026-07-13.
Implementation still requires the ordinary independent Sol/high review and settlement gates.

## 2. Constraints and non-goals

### Inviolable constraints

- Main does not write Rust product code.
- A Main or author session cannot review its own work for settlement.
- A native subagent result alone is not settlement evidence unless the existing manifest, receipt,
  freshness, sandbox, and final-hash requirements are satisfied.
- Model selection must not weaken sandbox, approval, secret handling, data-loss, or external-effect
  controls.
- Model and effort actually used by wrapper dispatches remain visible in atomic receipts.

### Non-goals

- No product behavior, Rust code, public API, data model, or design-corpus contract changes.
- No change to Claude, Grok, or Codex Cloud availability or settlement eligibility.
- No automatic retry, model fallback, or silent downgrade when a requested model is unavailable.
- No global `~/.codex` role policy for unrelated repositories.
- No attempt to change the model of a running agent thread. A different model requires a new role
  dispatch.

## 3. Considered approaches

### A. Repository-native roles plus wrapper alignment — selected

Define named roles in project-scoped Codex configuration and align the existing `codex.sh` mode
defaults. This makes native CLI delegation convenient while retaining the wrapper's independent
process, sandbox, manifest, hash, and receipt controls.

### B. Repository-native roles only — rejected

This would leave wrapper defaults at Terra/medium for implementation and Sol/medium for normal
review. Native and settlement paths would then use different policies and could drift silently.

### C. Global named roles — rejected

Global roles would apply the `iotkit-next` risk and cost policy to unrelated repositories. The
policy belongs in this trusted repository; the user's existing global Main default may remain
Sol/high independently.

## 4. Configuration design

Add a project-scoped `.codex/config.toml` that declares three named roles:

| Role | Purpose | Model / effort |
|---|---|---|
| `implementer` | settled implementation work | `gpt-5.6-luna/max` |
| `executor` | tests, probes, and mechanical execution | `gpt-5.6-luna/max` |
| `reviewer` | design and implementation review | `gpt-5.6-sol/high` |

The role entries use `agents.<name>.config_file` and concise descriptions that let Main choose the
correct agent type. Two role layers avoid duplicated model constants:

- `.codex/agents/luna-max.toml`
- `.codex/agents/sol-high.toml`

The project configuration explicitly sets Main to Sol/high so the repository does not depend on a
particular user's model default. It will not copy provider, authentication, telemetry, or other
machine-local settings into the repository.

Native role routing is advisory orchestration, not an independent-review receipt mechanism. The
authoritative settlement route remains `scripts/codex.sh review` with `REVIEW_MANIFEST`.

## 5. Wrapper and workflow alignment

`scripts/codex.sh` will use these defaults:

| Mode | Model | Effort | Sandbox |
|---|---|---|---|
| `review` | `gpt-5.6-sol` | `high` | `read-only` |
| `impl` | `gpt-5.6-luna` | `max` | `workspace-write` |

`CODEX_MODEL` and `CODEX_EFFORT` remain explicit per-dispatch overrides. The wrapper continues to
reject unsupported effort strings, require a manifest for review, bind hashes, and record the
effective model and effort in its receipt. It must not silently fall back to another model or
effort.

`docs/development-workflow.md` will replace the superseded task-tier matrix with role-based routing.
It will preserve impact-based Green/Yellow/Red classification: choosing Luna/max for implementation
does not authorize Luna to make Red product decisions. Main owns classification and sends any new
Red value decision to the user; the implementation worker executes only the approved contract.

Plan 6 implementation changes from Sol/high to Luna/max. Plan 6 Main, design, independent review,
review reconciliation, confirmation, and final settlement remain Sol/high.

## 6. Dispatch and failure behavior

1. Main classifies the work and selects a named role.
2. A new subagent starts with that role's configuration layer.
3. The subagent reports its result to Main; it does not mutate its own role or model mid-thread.
4. Settlement-required reviews are separately dispatched through the manifest-bound review
   wrapper, even if a native reviewer already provided advisory feedback.
5. If Codex rejects a role configuration, model, or effort, dispatch fails visibly. Main records
   the failure and stops that route; it does not downgrade silently.
6. If the installed CLI does not expose configured role selection through its current multi-agent
   interface, the wrapper path remains operational and the native-role limitation is recorded
   rather than represented as working.

No product state or customer data flows through this configuration. The principal risks are false
review provenance, silent model drift, excessive execution cost, and loss of native dispatch. The
design addresses them with receipt binding, one source per role layer, explicit failure, and a
tested wrapper fallback.

## 7. Verification and review

Implementation verification must include:

- parse and inspect the effective project configuration with the installed Codex CLI without
  issuing a model turn; if the CLI exposes no non-model validation path, record that limitation
  and do not claim native role resolution from parsing alone;
- confirm the installed model catalog lists Sol and Luna with the requested effort levels;
- exercise wrapper default selection without launching a model by using or adding a bounded test
  seam;
- exercise explicit `CODEX_MODEL` and `CODEX_EFFORT` overrides;
- exercise rejection of an invalid effort and review dispatch without `REVIEW_MANIFEST`;
- search the active workflow authority for stale Terra/medium, Sol/medium, and Plan-6 Sol/high
  implementation prescriptions, retaining historical ledger entries only when clearly marked
  superseded;
- run `scripts/verify.sh`;
- obtain a fresh, read-only, manifest-bound Sol/high review of the final artifact hash and resolve
  every Critical/Important finding before commit.

The review must focus on native-role support in the installed CLI, configuration layering, role
selection observability, wrapper/receipt provenance, fail-closed behavior, and contradictions with
the active workflow authority.

## 8. Rollback

Rollback removes the project role declarations and role-layer files, restores the prior wrapper
defaults, and restores the prior workflow text in one intentional change. Historical ledger records
remain immutable and a new entry explains the rollback. No product or database rollback is needed.

Reconsider this policy if Luna/max is unavailable, native role selection is not exposed by the
installed CLI, task latency or usage becomes unacceptable, or independent review finds that role
configuration can weaken settlement provenance.
