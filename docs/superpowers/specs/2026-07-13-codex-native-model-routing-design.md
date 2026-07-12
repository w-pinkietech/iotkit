# Codex Native Model Routing Design

Status: **Approved for implementation** (2026-07-13)

Date: 2026-07-13

## 1. Mission brief

Configure Codex CLI so the Main session remains `gpt-5.6-sol/high`, review work uses
`gpt-5.6-sol/high`, and implementation or execution work uses `gpt-5.6-luna/max`. Main chooses
the appropriate named agent role when dispatching native Codex subagents. Existing process-isolated
dispatch remains aligned with the same routing and continues to provide manifest-bound settlement
evidence where required.

Acceptance outcome:

- a no-model app-server `config/read` probe resolves named `implementer`, `executor`, and
  `reviewer` role-layer paths without configuration errors; the repository-owned role preflight
  then strict-parses each resolved layer and fails closed for a missing file, malformed TOML, or
  an unknown role-layer key;
- the independently strict-parsed `implementer` and `executor` role layers are Luna/max, while
  the independently strict-parsed `reviewer` layer and Main configuration are Sol/high;
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
- Atomic receipts distinguish requested model/effort from observable runtime evidence. A Codex
  JSONL event stream is bound to each receipt, and any model-reroute event fails closed. Effort is
  recorded as requested unless the installed CLI exposes trustworthy effective-effort evidence;
  unavailable observed model/effort fields are explicit rather than inferred.

### Non-goals

- No product behavior, Rust code, public API, data model, or design-corpus contract changes.
- No change to Claude, Grok, or Codex Cloud availability or settlement eligibility.
- No automatic retry, model fallback, or silent downgrade when a requested model is unavailable.
- No global `~/.codex` role policy for unrelated repositories.
- No attempt to change the model of a running agent thread. A different model requires a new role
  dispatch.
- The currently exposed `spawn_agent` schema has no role selector. Configuration loading is in
  scope; successful native role selection is not claimed until a supported surface demonstrates
  it in a new session.

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

The no-model app-server `config/read` probe resolves the project `agents.<name>.config_file` paths;
it does not load or validate every referenced role file. `scripts/check-codex-role-config.sh` is the
repository-owned authoritative preflight: it resolves those paths through `config/read` and invokes
the installed Codex strict parser independently for each role layer. Configuration loading still
does not prove native role selection.

Native role routing is advisory orchestration, not an independent-review receipt mechanism. The
authoritative settlement route remains `scripts/codex.sh review` with `REVIEW_MANIFEST`.
Every native role sets `approval_policy = "never"`; the reviewer additionally remains read-only,
and Luna roles remain bounded by workspace-write. A role cannot request host escalation silently.

## 5. Wrapper and workflow alignment

`scripts/codex.sh` will use these defaults:

| Mode | Model | Effort | Sandbox |
|---|---|---|---|
| `review` | `gpt-5.6-sol` | `high` | `read-only` |
| `impl` | `gpt-5.6-luna` | `max` | `workspace-write` |

`CODEX_MODEL` and `CODEX_EFFORT` remain explicit per-dispatch overrides. The wrapper continues to
reject unsupported effort strings, require a manifest for review, bind hashes, and record the
requested model and effort in its receipt. Both modes explicitly pass approval policy `never`;
review remains read-only and implementation remains workspace-write. Codex stdout is captured as
an atomic JSONL event stream. The wrapper rejects malformed/incomplete event streams and every
model-reroute event, publishes no successful result or receipt on rejection, and binds the event
stream hash into the receipt. Until Codex exposes trustworthy effective-effort evidence, receipts
must use `requested_effort` plus `observed_effort=UNAVAILABLE` rather than implying runtime
attestation. Likewise, `observed_model=UNAVAILABLE` is explicit when the event stream proves only
that no reroute was reported, not which backend served the request.

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
6. If the JSONL stream reports model rerouting, is malformed, lacks successful turn completion, or
   cannot be bound to the receipt, the wrapper removes partial/final success artifacts and fails.
7. If the installed CLI does not expose configured role selection through its current multi-agent
   interface, the wrapper path remains operational and the native-role limitation is recorded
   rather than represented as working.

No product state or customer data flows through this configuration. The principal risks are false
review provenance, silent model drift, excessive execution cost, and loss of native dispatch. The
design addresses them with receipt binding, one source per role layer, explicit failure, and a
tested wrapper fallback.

## 7. Verification and review

Implementation verification must include:

- resolve the effective project role paths through a no-model app-server `config/read` probe, then
  run `scripts/check-codex-role-config.sh` so the installed Codex strict parser independently
  validates each referenced role layer; deterministic negative fixtures cover missing, malformed,
  and unknown role-layer keys. `config/read` path resolution does not prove native role selection
  or validate every referenced role file;
- confirm the installed model catalog lists Sol and Luna with the requested effort levels;
- exercise wrapper default selection without launching a model by using or adding a bounded test
  seam;
- exercise explicit `CODEX_MODEL` and `CODEX_EFFORT` overrides;
- exercise rejection of an invalid effort and review dispatch without `REVIEW_MANIFEST`;
- compare the complete ordered Codex argv for both modes, including explicit approval policy;
- exercise matching JSONL, model reroute, malformed/incomplete JSONL, empty output, CLI failure,
  and mutated-manifest cases, asserting fail-closed removal of final/partial result and receipt;
- verify requested model/effort, approval policy, event-stream path/hash, result path/hash, prompt
  path/hash, and manifest path/hash in the receipt;
- search the active workflow authority for stale Terra/medium, Sol/medium, and Plan-6 Sol/high
  implementation prescriptions, retaining historical ledger entries only when clearly marked
  superseded;
- run `scripts/verify.sh`;
- obtain a fresh, read-only, manifest-bound Sol/high review of the final artifact hash and resolve
  every Critical/Important finding before commit.

The review must focus on native-role support in the installed CLI, configuration layering, the
explicit limit on role-selection observability, wrapper JSONL/reroute and receipt provenance,
approval non-escalation, fail-closed behavior, and contradictions with active workflow authority.

## 8. Rollback

Rollback removes the project role declarations and role-layer files, restores the prior wrapper
defaults, and restores the prior workflow text in one intentional change. Historical ledger records
remain immutable and a new entry explains the rollback. No product or database rollback is needed.

Reconsider this policy if Luna/max is unavailable, native role selection is not exposed by the
installed CLI, task latency or usage becomes unacceptable, or independent review finds that role
configuration can weaken settlement provenance.
