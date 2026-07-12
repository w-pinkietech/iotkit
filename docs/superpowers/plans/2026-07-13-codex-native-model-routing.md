# Codex Native Model Routing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Main and review dispatches use `gpt-5.6-sol/high`, while native implementation/execution roles and process-isolated implementation dispatches use `gpt-5.6-luna/max`.

**Architecture:** Project-scoped Codex agent roles provide native CLI routing without changing machine-local provider or authentication settings. The existing `scripts/codex.sh` process boundary remains the settlement-capable route and uses the same role defaults; a fake-CLI regression harness proves its effective arguments and receipts without spending a model turn. Workflow authority, the active Plan 6 instruction, and the persistent ledger change in the same reviewed artifact so no active path retains the superseded routing.

**Tech Stack:** Codex CLI 0.144.1 project configuration (TOML), Bash, Git, existing manifest/receipt scripts, Markdown workflow authority.

## Global Constraints

- Main and every review or confirmation role use exactly `gpt-5.6-sol/high`.
- Implementation and execution roles use exactly `gpt-5.6-luna/max`.
- Main does not write Rust product code; this task changes only Codex configuration, workflow scripts, tests, and documentation.
- Native role output is advisory and cannot satisfy independent-review settlement by itself.
- Settlement review remains a fresh read-only process with `REVIEW_MANIFEST`, final-hash binding, an atomic receipt, and zero unresolved Critical/Important findings.
- `CODEX_MODEL` and `CODEX_EFFORT` remain explicit wrapper overrides; no route silently falls back or downgrades.
- Codex dispatch captures an atomic JSONL event stream, rejects any model-reroute event, and binds
  the stream to its receipt. Receipts say `requested_model` and `requested_effort`; they do not
  claim an effective model or effort when the CLI does not attest it, and record
  `observed_model=UNAVAILABLE` / `observed_effort=UNAVAILABLE` explicitly.
- Review is explicitly `read-only` plus approval policy `never`; implementation is explicitly
  `workspace-write` plus approval policy `never`. Neither mode inherits machine approval policy.
- Do not copy provider, authentication, notification, telemetry, or other machine-local settings into project configuration.
- Do not issue a model turn merely to validate configuration parsing or default arguments.
- No product code, public API, database, design-corpus contract, Cloud behavior, or external-vendor policy changes.
- This is one Large/Red workflow-authority unit. The user approved the value decision on 2026-07-13; push, PR, release, and other publication remain unauthorized.

## File Structure

- Create `.codex/config.toml`: trusted-repository Main default and named native agent-role registry.
- Create `.codex/agents/luna-max.toml`: shared Luna/max writable role layer for implementer and executor.
- Create `.codex/agents/sol-high.toml`: shared Sol/high read-only role layer for reviewer.
- Create `scripts/check-codex-role-config.sh`: resolve project role paths and independently strict-parse each role layer without a model turn.
- Create `scripts/test-codex-role-config.sh`: deterministic positive/missing/malformed/unknown-key preflight regression fixtures.
- Create `scripts/check-codex-events.sh`: validate JSONL completion and fail on model reroute.
- Create `scripts/test-codex.sh`: no-network fake-CLI regression test for role files, wrapper defaults, overrides, receipts, and fail-closed validation.
- Modify `scripts/codex.sh`: change only documented and executable defaults; preserve sandbox and receipt behavior.
- Modify `scripts/review-receipt.sh`: distinguish requested values and bind approval, sandbox, and
  Codex event-stream evidence.
- Modify `scripts/verify.sh`: run the no-model role preflight and both deterministic routing/config regression tests.
- Modify `docs/development-workflow.md`: replace task-tier model routing with the approved role routing and preserve risk/settlement boundaries.
- Modify `docs/superpowers/plans/2026-07-12-wave1-plan6-http-ingress-authentication.md`: replace active Plan 6 implementation dispatch instructions with Luna/max.
- Modify `docs/superpowers/specs/2026-07-12-wave1-plan6-http-ingress-authentication-design.md`:
  append dated model-routing supersession without rewriting the settled 2026-07-12 record.
- Modify `docs/superpowers/PLAN6-DESIGN-READY.md`: append the same dated workflow supersession to
  historical Design Ready evidence.
- Modify `.claude/skills/codex-eval-common/SKILL.md`: remove duplicated effort examples and defer
  review routing to workflow authority and `scripts/codex.sh`.
- Modify `docs/superpowers/active-ledger.md`: record the superseding Red decision, artifact/review state, verification, and final settlement evidence.
- Create `.review/codex-model-routing-review.md` and `.review/codex-model-routing.manifest`: ignored operational review inputs; do not commit them.

---

### Task 1: Implement and settle native model routing as one atomic workflow unit

**Ownership:** Steps 1–9 belong to a fresh Luna/max implementation worker. The worker must not
modify the active ledger, stage, or commit. Steps 10–14 belong only to Main. Main performs review
reconciliation, settlement bookkeeping, staging, and commit.

**Files:**
- Create: `.codex/config.toml`
- Create: `.codex/agents/luna-max.toml`
- Create: `.codex/agents/sol-high.toml`
- Create: `scripts/check-codex-role-config.sh`
- Create: `scripts/test-codex-role-config.sh`
- Create: `scripts/check-codex-events.sh`
- Create: `scripts/test-codex.sh`
- Modify: `scripts/codex.sh:17-27,49-55`
- Modify: `scripts/verify.sh:14-18`
- Modify: `scripts/review-receipt.sh`
- Modify: `docs/development-workflow.md:155-171,224-230`
- Modify: `docs/superpowers/plans/2026-07-12-wave1-plan6-http-ingress-authentication.md:10-18,438-443`
- Modify: `docs/superpowers/specs/2026-07-12-wave1-plan6-http-ingress-authentication-design.md:21-24`
- Modify: `docs/superpowers/PLAN6-DESIGN-READY.md:21-23`
- Modify: `.claude/skills/codex-eval-common/SKILL.md:24-52`
- Modify: `docs/superpowers/active-ledger.md` under `Reality`, `User decisions`, and `Review state`
- Create operationally: `.review/codex-model-routing-review.md`
- Create operationally: `.review/codex-model-routing.manifest`

**Interfaces:**
- Consumes: Codex configuration keys `model`, `review_model`, `model_reasoning_effort`, and `agents.<name>.config_file`; wrapper environment variables `CODEX_MODEL`, `CODEX_EFFORT`, `CODEX_BIN`, `CODEX_REPO`, `CODEX_OUT_DIR`, and `REVIEW_MANIFEST`.
- Produces: a no-model `config/read` path resolver plus an authoritative strict role-layer preflight; native role declarations `implementer`, `executor`, and `reviewer` (native selection remains unverified); wrapper defaults `impl = gpt-5.6-luna/max` and `review = gpt-5.6-sol/high`; a JSONL event validator; receipt schema fields `requested_model`, `requested_effort`, `observed_model`, `observed_effort`, `approval_policy`, `sandbox_mode`, `event_stream_path`, `event_stream_sha256`, and `model_reroute_observed`; deterministic commands `scripts/test-codex-role-config.sh` and `scripts/test-codex.sh` returning zero only when role parsing, complete ordered arguments, events, cleanup, and receipts match the approved routing.

- [ ] **Step 1 (worker): Record the task start and verify the clean implementation base**

Run:

```bash
date --iso-8601=seconds
git status --short
git rev-parse HEAD
```

Expected: timestamp recorded for the ledger/timing report, clean status, and HEAD equal to the committed design/plan base. If unrelated user changes exist, preserve them and stop if they overlap any listed file.

- [ ] **Step 2 (worker): Write the failing wrapper, event, receipt, and role regression tests**

Create executable `scripts/test-codex.sh`. It must initialize a temporary Git repository and a
fake Codex binary, then enforce all of these exact contracts:

1. Capture fake Codex argv as NUL-delimited bytes and compare it with `cmp` against a separately
   generated complete ordered vector. Membership-only assertions are forbidden.
2. The expected implementation vector is exactly:

   ```text
   -a never exec -s workspace-write --skip-git-repo-check -C <repo>
   -m gpt-5.6-luna -c model_reasoning_effort=max --json -o <partial-result> -
   ```

3. The expected review vector differs only where required:

   ```text
   -a never exec -s read-only --skip-git-repo-check -C <repo>
   -m gpt-5.6-sol -c model_reasoning_effort=high --json -o <partial-result> -
   ```

4. The fake binary rejects every unexpected option, duplicate, missing option value, or misplaced
   argument. A success fixture writes a non-empty last-message file and emits exactly one
   `thread.started`, one `turn.started`, and one `turn.completed` JSON object to stdout.
5. A reroute fixture emits a `model_reroute` event before `turn.completed`. Malformed JSON,
   `turn.failed`, `error`, missing `turn.completed`, empty last message, and nonzero fake exit each
   have separate fixtures.
6. Successful implementation, review, and explicit-override cases verify complete receipt fields:
   `requested_model`, `requested_effort`, `observed_model=UNAVAILABLE`,
   `observed_effort=UNAVAILABLE`, `sandbox_mode`, `approval_policy`, prompt/manifest/result/
   event-stream absolute paths and SHA-256 hashes, and `model_reroute_observed=false`.
7. Invalid effort and absent manifest fail before fake invocation and also assert absence of final
   result, event stream, receipt, and every `.partial` file. Mutated manifest, reroute,
   malformed/incomplete/error JSONL, empty output, and fake failure assert nonzero status plus the
   same no-publication/no-partial invariant. The mutated-manifest case must prove the fake ran
   before the post-run verification rejected publication.
8. Role-file checks include model, requested effort, sandbox, and `approval_policy = "never"`.

Use helpers with explicit contracts, for example:

```bash
write_expected_argv() { printf '%s\0' "$@" >"$EXPECTED"; }
assert_no_publication() {
  local label="$1"
  ! find "$OUT" -maxdepth 1 -type f \
    \( -name "codex-${label}-*" -o -name "codex-${label}-*.partial" \) | grep -q .
}
```

`scripts/test-codex.sh` itself never invokes the real Codex binary or the network; the separate
`scripts/test-codex-role-config.sh` intentionally invokes the installed strict parser in a
no-model app-server process. Mark both tests executable with `chmod +x scripts/test-codex.sh
scripts/test-codex-role-config.sh`.

- [ ] **Step 3 (worker): Run the new test and verify the intended red state**

Run:

```bash
scripts/test-codex.sh
```

Expected: FAIL before a fake model invocation because `.codex/config.toml` does not exist. A failure caused by missing expected configuration/defaults is the required red signal; an environmental failure must be fixed before continuing.

- [ ] **Step 4 (worker): Add the native project roles**

Create `.codex/config.toml`:

```toml
model = "gpt-5.6-sol"
model_reasoning_effort = "high"
review_model = "gpt-5.6-sol"

[agents.implementer]
description = "Implement settled requirements and write focused tests. Escalate new product or trust decisions to Main."
config_file = "agents/luna-max.toml"

[agents.executor]
description = "Run tests, verification commands, and bounded mechanical probes without changing approved contracts."
config_file = "agents/luna-max.toml"

[agents.reviewer]
description = "Independently review design or implementation for correctness, safety, provenance, and contract compliance."
config_file = "agents/sol-high.toml"
```

Create `.codex/agents/luna-max.toml`:

```toml
model = "gpt-5.6-luna"
model_reasoning_effort = "max"
sandbox_mode = "workspace-write"
approval_policy = "never"
```

Create `.codex/agents/sol-high.toml`:

```toml
model = "gpt-5.6-sol"
model_reasoning_effort = "high"
sandbox_mode = "read-only"
approval_policy = "never"
```

- [ ] **Step 5 (worker): Prove configuration loading without claiming native role selection**

Use Codex app-server `config/read` with `cwd` set to the worktree and `includeLayers=true`. Run it
with host permission if the ordinary sandbox cannot initialize `$CODEX_HOME` SQLite state. Do not
use `codex --strict-config ... features list`: CLI 0.144.1 rejects that combination, while
non-strict `features list` ignores invalid role layers.

The probe must initialize app-server over stdio, issue `config/read`, and use `jq` to assert the
effective Main model/effort and all three `agents.<name>.config_file` paths. `config/read` resolves
those paths but does not load or validate the referenced role files. The authoritative
`scripts/check-codex-role-config.sh` invokes the installed `app-server --strict-config` parser
independently for every resolved layer, and `scripts/test-codex-role-config.sh` runs deterministic
positive and negative fixtures for:

- missing role file;
- malformed TOML role file;
- unknown role-layer field.

Each negative case exits nonzero before any model turn. These checks establish config parsing and
path/layer integrity only; the current exposed `spawn_agent` schema has no role selector, so native
role selection remains unverified rather than claimed.

Separately run catalog checks:

```bash
/home/kenta/.local/bin/codex debug models | jq -e '.models[] | select(.slug == "gpt-5.6-sol") | .supported_reasoning_levels[] | select(.effort == "high")'
/home/kenta/.local/bin/codex debug models | jq -e '.models[] | select(.slug == "gpt-5.6-luna") | .supported_reasoning_levels[] | select(.effort == "max")'
```

Expected: effective-config and catalog assertions pass, the preflight reports three resolved and
strict-parsed layers, and all three negative role-layer fixtures fail closed. These checks establish
config parsing, path integrity, and catalog support only. The current exposed `spawn_agent` schema
has no role selector, so the worker and Main must record native role selection as unverified rather
than claim it works.

- [ ] **Step 6 (worker): Harden wrapper events, approvals, receipts, and defaults**

Modify the environment-variable comments and mode mapping in `scripts/codex.sh` to say and implement:

```bash
#   CODEX_MODEL   (override; review defaults to gpt-5.6-sol, impl to
#                  gpt-5.6-luna.)
#   CODEX_EFFORT  (override; review defaults to high, impl to max.
#                  Accepted scale: low < medium < high < xhigh < max.)
```

```bash
case "$MODE" in
  review) SANDBOX="read-only";       APPROVAL="never"; DEFAULT_MODEL="gpt-5.6-sol";  DEFAULT_EFFORT="high" ;;
  impl)   SANDBOX="workspace-write"; APPROVAL="never"; DEFAULT_MODEL="gpt-5.6-luna"; DEFAULT_EFFORT="max" ;;
  *) echo "mode must be 'review' or 'impl', got: '$MODE'" >&2; exit 2 ;;
esac
```

Create executable `scripts/check-codex-events.sh <events-jsonl>`. It must parse every line with
`jq`, require exactly one `thread.started`, `turn.started`, and `turn.completed`, reject
`turn.failed`, `error`, and any event type matching model reroute, and print only stable
machine-readable evidence needed by the wrapper. Empty or malformed streams fail.

Change `scripts/codex.sh` to invoke `"$CODEX_BIN" -a "$APPROVAL" exec ...` (the approval flag is a
root option and must precede the `exec` subcommand), pass `--json`, redirect stdout to a private
`.events.jsonl.partial`, validate it, and only then atomically publish both final message and event
stream. Any command, event, post-run manifest, or receipt failure removes final and partial success
artifacts. Keep stderr available for progress diagnostics.

Change `scripts/review-receipt.sh` to schema version 2. Rename ambiguous `model`/`effort` fields to
`requested_model`/`requested_effort`; add `observed_model`, `observed_effort`, `sandbox_mode`,
`approval_policy`, `event_stream_path`, `event_stream_sha256`, and
`model_reroute_observed=false`. Use `UNAVAILABLE` for observed model/effort unless a documented
event attests them; absence of reroute is not effective-model attestation. Require a bound stream
for vendor `codex`; other vendors record `UNAVAILABLE` rather than fabricating evidence. Preserve
prompt, manifest, result, timestamp, atomic-write, newline-rejection, and hash binding behavior.
Update Claude/Grok callers only as needed for the explicit schema inputs; do not change their model
policy or availability.

- [ ] **Step 7 (worker): Run the focused regression test and verify green**

Run:

```bash
scripts/test-codex.sh
```

Expected final line: `codex routing tests: OK`; exit status zero. No real model process or network request occurs because `CODEX_BIN` points at the fake CLI.

- [ ] **Step 8 (worker): Add the routing regression to full verification**

Insert these blocks in `scripts/verify.sh` after `scripts/test-codex-cloud.sh`:

```bash
echo "== scripts/check-codex-role-config.sh (strict no-model role preflight) =="
scripts/check-codex-role-config.sh

echo "== scripts/test-codex-role-config.sh (role preflight negative fixtures) =="
scripts/test-codex-role-config.sh

echo "== scripts/test-codex.sh (model routing and receipt defaults) =="
scripts/test-codex.sh
```

Run:

```bash
scripts/test-codex.sh
git diff --check
```

Expected: routing tests report OK and `git diff --check` emits nothing.

- [ ] **Step 9 (worker): Align every active workflow, Plan 6, and evaluation instruction**

Replace the model-effort table and surrounding routing prose in `docs/development-workflow.md`
with:

```markdown
Model and effort follow the dispatched role:

| Role | Default | Boundary |
|---|---|---|
| Main | `gpt-5.6-sol/high` | Classifies work, owns design and reconciliation, and does not write Rust product code |
| Independent review and confirmation | `gpt-5.6-sol/high` | Fresh read-only session; manifest/receipt/final-hash settlement remains mandatory |
| Implementation and execution | `gpt-5.6-luna/max` | Executes the settled contract; new Red decisions return to Main/user |

Project-scoped native agent roles and `scripts/codex.sh` use the same mapping. A running thread
does not change models; Main starts a new role dispatch. Explicit wrapper overrides remain visible
as requested values in receipts. Codex JSONL evidence is bound to the receipt and every observed
model reroute fails closed. Effective effort is not claimed when the CLI does not attest it.

Plan 6 and every Large/Red workflow keep Main, design, independent review, reconciliation,
confirmation, and final settlement on `gpt-5.6-sol/high`. Their implementation and execution
workers use `gpt-5.6-luna/max`; model choice does not change the risk classification or authorize
workers to make Red product decisions.
```

Replace the implementation paragraph in section 7 with:

```markdown
Product implementation remains worker-driven through `scripts/codex.sh impl`, whose default is
`gpt-5.6-luna/max`. Main remains `gpt-5.6-sol/high`, verifies with `scripts/verify.sh`, runs the
required independent `gpt-5.6-sol/high` review, reconciles findings, and commits one intentional
unit at a time. Final integration review remains mandatory.
```

In `docs/superpowers/plans/2026-07-12-wave1-plan6-http-ingress-authentication.md`, replace the
active implementation command with:

```bash
CODEX_MODEL=gpt-5.6-luna CODEX_EFFORT=max scripts/codex.sh impl <prompt> <label>
```

At the later worker redispatch instruction, state explicitly that product-code fixes use the same
Luna/max implementation role. Preserve settled historical review receipts and statements that
truthfully record earlier Sol/high runs.

In `docs/superpowers/specs/2026-07-12-wave1-plan6-http-ingress-authentication-design.md`, retain the
original 2026-07-12 sentence and immediately append:

```markdown
> **Workflow supersession (2026-07-13):** The product contract above is unchanged. Current model
> routing follows `docs/development-workflow.md`: Plan 6 Main/design/review/confirmation/settlement
> use Sol/high; implementation/execution workers use Luna/max.
```

Append the equivalent dated note to `docs/superpowers/PLAN6-DESIGN-READY.md`; do not rewrite its
historical 2026-07-12 evidence.

In `.claude/skills/codex-eval-common/SKILL.md`, keep the single default command
`REVIEW_MANIFEST=<manifest> scripts/codex.sh review ...`, remove normal/mechanical/high-risk model
examples, and state that all current review routing and approval/sandbox defaults come exclusively
from `docs/development-workflow.md` and `scripts/codex.sh`. Preserve its fresh-session, unique-label,
read-only, manifest, and settlement mechanics.

- [ ] **Step 10 (Main): Record the superseding Red decision before implementation review dispatch**

Update `docs/superpowers/active-ledger.md`:

- set Reality to the actual current HEAD and this model-routing workflow task;
- add a 2026-07-13 User decision that supersedes only the 2026-07-12 routing decision: Main and all
  review/confirmation use Sol/high; implementation/execution use Luna/max; native roles and wrapper
  defaults are aligned; no silent fallback; settlement rules are unchanged;
- retain the older entry, marking it superseded rather than rewriting history;
- add an in-flight Review state with the artifact/base HEAD, prompt/manifest paths and hashes,
  required vendor `Codex`, model/effort Sol/high, and unresolved status.

Do not claim `SETTLED` before a final receipt with zero unresolved Critical/Important findings.

- [ ] **Step 11 (Main): Run full verification before freezing the review artifact**

Run:

```bash
scripts/verify.sh
git diff --check
git status --short
```

Expected: `scripts/verify.sh` ends with its PASS line, diff check is silent, and status lists only
the intended configuration, scripts, tests, workflow, Plan 6 instruction, ledger, and this plan if
it was not previously committed. Diagnose any failure before review; do not weaken a check.

- [ ] **Step 12 (Main): Build the exact review manifest and prompt**

Create `.review/codex-model-routing-review.md` containing:

```markdown
# Codex native model routing review

Review the exact manifest as a fresh read-only independent reviewer. This is a Large/Red workflow-
authority and review-harness change. Verify every manifest blob and run bounded tests/probes.

Required contract: Main and review/confirmation are gpt-5.6-sol/high; implementation/execution are
gpt-5.6-luna/max. Native roles are advisory. Settlement remains process-isolated, read-only,
manifest-bound, receipt-bound, final-hash-bound, and fail-closed. Explicit wrapper overrides remain
observable; no silent fallback is allowed. No product behavior or external-vendor policy changes.

Focus on installed-CLI role/config support, config-layer paths, sandbox non-escalation, fake-CLI test
fidelity, default/override receipt provenance, stale active instructions, historical-record
preservation, Red classification, secrets, data loss, external effects, and settlement integrity.

Run at least scripts/test-codex.sh, config parsing/catalog probes, git diff --check, and relevant
negative wrapper probes. Report Critical, Important, and Minor findings separately. Zero findings
must be explicit. Do not modify files.
```

Build the manifest over every substantive changed file except the operational ledger, prompt, and
manifest themselves:

```bash
scripts/review-manifest.sh .review/codex-model-routing.manifest \
  AGENTS.md \
  .codex/config.toml \
  .codex/agents/luna-max.toml \
  .codex/agents/sol-high.toml \
  scripts/codex.sh \
  scripts/check-codex-role-config.sh \
  scripts/test-codex-role-config.sh \
  scripts/check-codex-events.sh \
  scripts/test-codex.sh \
  scripts/verify.sh \
  scripts/review-manifest.sh \
  scripts/review-receipt.sh \
  docs/development-workflow.md \
  docs/superpowers/plans/2026-07-12-wave1-plan6-http-ingress-authentication.md \
  docs/superpowers/specs/2026-07-12-wave1-plan6-http-ingress-authentication-design.md \
  docs/superpowers/PLAN6-DESIGN-READY.md \
  .claude/skills/codex-eval-common/SKILL.md \
  docs/superpowers/specs/2026-07-13-codex-native-model-routing-design.md \
  docs/superpowers/plans/2026-07-13-codex-native-model-routing.md
sha256sum .review/codex-model-routing.manifest .review/codex-model-routing-review.md
```

Expected: manifest creation exits zero and both SHA-256 values are recorded in the ledger. Freeze
every manifest file until the result is collected.

- [ ] **Step 13 (Main): Dispatch the required independent Sol/high review and reconcile findings**

Run:

```bash
REVIEW_MANIFEST=.review/codex-model-routing.manifest \
CODEX_MODEL=gpt-5.6-sol CODEX_EFFORT=high \
scripts/codex.sh review .review/codex-model-routing-review.md codex-model-routing
```

Expected: a non-empty result, event stream, and receipt under `/tmp/codex-runs`; receipt fields
include `mode=review`, `requested_model=gpt-5.6-sol`, `requested_effort=high`,
`observed_model=UNAVAILABLE`, `observed_effort=UNAVAILABLE`, `sandbox_mode=read-only`,
`approval_policy=never`, `model_reroute_observed=false`, and exact
event/prompt/manifest/result hashes.

For every Critical/Important finding:

1. classify it against the approved design and workflow authority;
2. make the smallest in-scope correction;
3. rerun focused tests and `scripts/verify.sh`;
4. rebuild a final manifest over the corrected files;
5. send a fresh Sol/high confirmation to the finding owner and require zero unresolved C/I on the
   final hash.

If the current CLI does not actually expose configured native roles, retain wrapper routing, record
the limitation, and do not claim native dispatch works. If resolving it changes the approved
architecture materially, stop and return to the user.

- [ ] **Step 14 (Main): Record final settlement, verify, and commit the atomic unit**

Update the active ledger with:

- final manifest, prompt, result, and receipt paths plus SHA-256 hashes;
- requested and observed model/effort evidence, plus timestamps;
- finding progression and confirmation count;
- exact verification commands/results;
- task timing categories required by `docs/development-workflow.md`;
- `SETTLED` only after zero unresolved Critical/Important findings on the final hash.

Then run fresh completion verification:

```bash
scripts/verify.sh
git diff --check
git status --short
git diff --stat
```

Expected: full verification PASS, no whitespace errors, and only intended files changed. Read every
applied finding fix against its prescription before staging.

Commit:

```bash
git add .codex/config.toml .codex/agents/luna-max.toml .codex/agents/sol-high.toml \
  scripts/codex.sh scripts/check-codex-role-config.sh scripts/test-codex-role-config.sh \
  scripts/check-codex-events.sh scripts/test-codex.sh scripts/verify.sh \
  scripts/review-receipt.sh \
  docs/development-workflow.md \
  docs/superpowers/plans/2026-07-12-wave1-plan6-http-ingress-authentication.md \
  docs/superpowers/specs/2026-07-12-wave1-plan6-http-ingress-authentication-design.md \
  docs/superpowers/PLAN6-DESIGN-READY.md \
  .claude/skills/codex-eval-common/SKILL.md \
  docs/superpowers/specs/2026-07-13-codex-native-model-routing-design.md \
  docs/superpowers/plans/2026-07-13-codex-native-model-routing.md \
  docs/superpowers/active-ledger.md
git commit -m "chore: route Codex work by native agent role"
```

Do not add `.review/` operational files. Do not push or open a PR.
