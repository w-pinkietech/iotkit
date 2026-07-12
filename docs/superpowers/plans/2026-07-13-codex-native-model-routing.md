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
- Do not copy provider, authentication, notification, telemetry, or other machine-local settings into project configuration.
- Do not issue a model turn merely to validate configuration parsing or default arguments.
- No product code, public API, database, design-corpus contract, Cloud behavior, or external-vendor policy changes.
- This is one Large/Red workflow-authority unit. The user approved the value decision on 2026-07-13; push, PR, release, and other publication remain unauthorized.

## File Structure

- Create `.codex/config.toml`: trusted-repository Main default and named native agent-role registry.
- Create `.codex/agents/luna-max.toml`: shared Luna/max writable role layer for implementer and executor.
- Create `.codex/agents/sol-high.toml`: shared Sol/high read-only role layer for reviewer.
- Create `scripts/test-codex.sh`: no-network fake-CLI regression test for role files, wrapper defaults, overrides, receipts, and fail-closed validation.
- Modify `scripts/codex.sh`: change only documented and executable defaults; preserve sandbox and receipt behavior.
- Modify `scripts/verify.sh`: run the new deterministic wrapper/config regression test.
- Modify `docs/development-workflow.md`: replace task-tier model routing with the approved role routing and preserve risk/settlement boundaries.
- Modify `docs/superpowers/plans/2026-07-12-wave1-plan6-http-ingress-authentication.md`: replace active Plan 6 implementation dispatch instructions with Luna/max.
- Modify `docs/superpowers/active-ledger.md`: record the superseding Red decision, artifact/review state, verification, and final settlement evidence.
- Create `.review/codex-model-routing-review.md` and `.review/codex-model-routing.manifest`: ignored operational review inputs; do not commit them.

---

### Task 1: Implement and settle native model routing as one atomic workflow unit

**Files:**
- Create: `.codex/config.toml`
- Create: `.codex/agents/luna-max.toml`
- Create: `.codex/agents/sol-high.toml`
- Create: `scripts/test-codex.sh`
- Modify: `scripts/codex.sh:17-27,49-55`
- Modify: `scripts/verify.sh:14-18`
- Modify: `docs/development-workflow.md:155-171,224-230`
- Modify: `docs/superpowers/plans/2026-07-12-wave1-plan6-http-ingress-authentication.md:10-18,438-443`
- Modify: `docs/superpowers/active-ledger.md` under `Reality`, `User decisions`, and `Review state`
- Create operationally: `.review/codex-model-routing-review.md`
- Create operationally: `.review/codex-model-routing.manifest`

**Interfaces:**
- Consumes: Codex configuration keys `model`, `review_model`, `model_reasoning_effort`, and `agents.<name>.config_file`; wrapper environment variables `CODEX_MODEL`, `CODEX_EFFORT`, `CODEX_BIN`, `CODEX_REPO`, `CODEX_OUT_DIR`, and `REVIEW_MANIFEST`.
- Produces: native roles `implementer`, `executor`, and `reviewer`; wrapper defaults `impl = gpt-5.6-luna/max` and `review = gpt-5.6-sol/high`; deterministic command `scripts/test-codex.sh` returning zero only when arguments and receipts match the approved routing.

- [ ] **Step 1: Record the task start and verify the clean implementation base**

Run:

```bash
date --iso-8601=seconds
git status --short
git rev-parse HEAD
```

Expected: timestamp recorded for the ledger/timing report, clean status, and HEAD equal to the committed design/plan base. If unrelated user changes exist, preserve them and stop if they overlap any listed file.

- [ ] **Step 2: Write the failing wrapper and role regression test**

Create executable `scripts/test-codex.sh` with this behavior and structure:

```bash
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

REPO="$TMP/repo"
OUT="$TMP/out"
FAKE_CODEX="$TMP/codex"
CAPTURE="$TMP/args"
mkdir -p "$REPO/.review"
git init -q "$REPO"
git -C "$REPO" config user.name test
git -C "$REPO" config user.email test@example.invalid
printf 'artifact\n' >"$REPO/artifact"
printf 'prompt\n' >"$REPO/.review/prompt.md"
git -C "$REPO" add artifact
git -C "$REPO" commit -qm init

cat >"$FAKE_CODEX" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
: >"$FAKE_CAPTURE"
OUT_PATH=""
while [ "$#" -gt 0 ]; do
  printf '%s\n' "$1" >>"$FAKE_CAPTURE"
  if [ "$1" = -o ]; then
    shift
    OUT_PATH="$1"
    printf '%s\n' "$1" >>"$FAKE_CAPTURE"
  fi
  shift
done
[ -n "$OUT_PATH" ]
printf 'fake result\n' >"$OUT_PATH"
EOF
chmod +x "$FAKE_CODEX"

assert_arg() {
  grep -Fx -- "$1" "$CAPTURE" >/dev/null || {
    printf 'missing fake Codex argument: %s\n' "$1" >&2
    exit 1
  }
}

assert_role_file() {
  local file="$1" model="$2" effort="$3" sandbox="$4"
  grep -Fx "model = \"$model\"" "$file" >/dev/null
  grep -Fx "model_reasoning_effort = \"$effort\"" "$file" >/dev/null
  grep -Fx "sandbox_mode = \"$sandbox\"" "$file" >/dev/null
}

grep -Fx 'model = "gpt-5.6-sol"' "$ROOT/.codex/config.toml" >/dev/null
grep -Fx 'model_reasoning_effort = "high"' "$ROOT/.codex/config.toml" >/dev/null
grep -Fx 'review_model = "gpt-5.6-sol"' "$ROOT/.codex/config.toml" >/dev/null
grep -Fx '[agents.implementer]' "$ROOT/.codex/config.toml" >/dev/null
grep -Fx '[agents.executor]' "$ROOT/.codex/config.toml" >/dev/null
grep -Fx '[agents.reviewer]' "$ROOT/.codex/config.toml" >/dev/null
[ "$(grep -Fc 'config_file = "agents/luna-max.toml"' "$ROOT/.codex/config.toml")" -eq 2 ]
[ "$(grep -Fc 'config_file = "agents/sol-high.toml"' "$ROOT/.codex/config.toml")" -eq 1 ]
assert_role_file "$ROOT/.codex/agents/luna-max.toml" gpt-5.6-luna max workspace-write
assert_role_file "$ROOT/.codex/agents/sol-high.toml" gpt-5.6-sol high read-only

FAKE_CAPTURE="$CAPTURE" CODEX_BIN="$FAKE_CODEX" CODEX_REPO="$REPO" \
  CODEX_OUT_DIR="$OUT" "$ROOT/scripts/codex.sh" impl "$REPO/.review/prompt.md" impl-default
assert_arg workspace-write
assert_arg gpt-5.6-luna
assert_arg model_reasoning_effort=max
IMPL_RECEIPT="$(find "$OUT" -name 'codex-impl-default-impl-*.txt.receipt' -print -quit)"
grep -Fx 'model=gpt-5.6-luna' "$IMPL_RECEIPT" >/dev/null
grep -Fx 'effort=max' "$IMPL_RECEIPT" >/dev/null

(cd "$REPO" && "$ROOT/scripts/review-manifest.sh" .review/manifest artifact >/dev/null)
FAKE_CAPTURE="$CAPTURE" CODEX_BIN="$FAKE_CODEX" CODEX_REPO="$REPO" \
  CODEX_OUT_DIR="$OUT" REVIEW_MANIFEST="$REPO/.review/manifest" \
  "$ROOT/scripts/codex.sh" review "$REPO/.review/prompt.md" review-default
assert_arg read-only
assert_arg gpt-5.6-sol
assert_arg model_reasoning_effort=high
REVIEW_RECEIPT="$(find "$OUT" -name 'codex-review-default-review-*.txt.receipt' -print -quit)"
grep -Fx 'model=gpt-5.6-sol' "$REVIEW_RECEIPT" >/dev/null
grep -Fx 'effort=high' "$REVIEW_RECEIPT" >/dev/null
grep -E '^artifact_manifest_sha256=[0-9a-f]{64}$' "$REVIEW_RECEIPT" >/dev/null

FAKE_CAPTURE="$CAPTURE" CODEX_BIN="$FAKE_CODEX" CODEX_REPO="$REPO" \
  CODEX_OUT_DIR="$OUT" CODEX_MODEL=gpt-5.6-sol CODEX_EFFORT=xhigh \
  "$ROOT/scripts/codex.sh" impl "$REPO/.review/prompt.md" explicit-override
assert_arg gpt-5.6-sol
assert_arg model_reasoning_effort=xhigh
OVERRIDE_RECEIPT="$(find "$OUT" -name 'codex-explicit-override-impl-*.txt.receipt' -print -quit)"
grep -Fx 'model=gpt-5.6-sol' "$OVERRIDE_RECEIPT" >/dev/null
grep -Fx 'effort=xhigh' "$OVERRIDE_RECEIPT" >/dev/null

if CODEX_BIN="$FAKE_CODEX" CODEX_REPO="$REPO" CODEX_OUT_DIR="$OUT" \
  CODEX_EFFORT=invalid "$ROOT/scripts/codex.sh" impl \
  "$REPO/.review/prompt.md" invalid-effort >/dev/null 2>&1; then
  echo 'invalid effort unexpectedly succeeded' >&2
  exit 1
fi
if CODEX_BIN="$FAKE_CODEX" CODEX_REPO="$REPO" CODEX_OUT_DIR="$OUT" \
  "$ROOT/scripts/codex.sh" review "$REPO/.review/prompt.md" no-manifest \
  >/dev/null 2>&1; then
  echo 'review without manifest unexpectedly succeeded' >&2
  exit 1
fi

echo 'codex routing tests: OK'
```

Mark it executable:

```bash
chmod +x scripts/test-codex.sh
```

- [ ] **Step 3: Run the new test and verify the intended red state**

Run:

```bash
scripts/test-codex.sh
```

Expected: FAIL before a fake model invocation because `.codex/config.toml` does not exist. A failure caused by missing expected configuration/defaults is the required red signal; an environmental failure must be fixed before continuing.

- [ ] **Step 4: Add the native project roles**

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
```

Create `.codex/agents/sol-high.toml`:

```toml
model = "gpt-5.6-sol"
model_reasoning_effort = "high"
sandbox_mode = "read-only"
```

- [ ] **Step 5: Prove the installed CLI accepts the project configuration without a model turn**

Run:

```bash
/home/kenta/.local/bin/codex --strict-config -C "$PWD" features list
/home/kenta/.local/bin/codex debug models | jq -e '.models[] | select(.slug == "gpt-5.6-sol") | .supported_reasoning_levels[] | select(.effort == "high")'
/home/kenta/.local/bin/codex debug models | jq -e '.models[] | select(.slug == "gpt-5.6-luna") | .supported_reasoning_levels[] | select(.effort == "max")'
```

Expected: all commands exit zero; strict config reports no unknown-field/config-layer error, and each `jq` command returns one supported effort entry. These checks establish parsing and catalog support, not proof that the current exposed `spawn_agent` schema selected a role.

- [ ] **Step 6: Change wrapper defaults and keep overrides explicit**

Modify the environment-variable comments and mode mapping in `scripts/codex.sh` to say and implement:

```bash
#   CODEX_MODEL   (override; review defaults to gpt-5.6-sol, impl to
#                  gpt-5.6-luna.)
#   CODEX_EFFORT  (override; review defaults to high, impl to max.
#                  Effort scale: low < medium < high < xhigh < max; "ultra"
#                  also exists but fans out subagents — a different execution/cost
#                  mode, never a silent default; opt in explicitly via CODEX_EFFORT)
```

```bash
case "$MODE" in
  review) SANDBOX="read-only";       DEFAULT_MODEL="gpt-5.6-sol";  DEFAULT_EFFORT="high" ;;
  impl)   SANDBOX="workspace-write"; DEFAULT_MODEL="gpt-5.6-luna"; DEFAULT_EFFORT="max" ;;
  *) echo "mode must be 'review' or 'impl', got: '$MODE'" >&2; exit 2 ;;
esac
```

Do not change the effort allowlist, command construction, prompt transport, manifest verification,
atomic output handling, or receipt call.

- [ ] **Step 7: Run the focused regression test and verify green**

Run:

```bash
scripts/test-codex.sh
```

Expected final line: `codex routing tests: OK`; exit status zero. No real model process or network request occurs because `CODEX_BIN` points at the fake CLI.

- [ ] **Step 8: Add the routing regression to full verification**

Insert this block in `scripts/verify.sh` after `scripts/test-codex-cloud.sh`:

```bash
echo "== scripts/test-codex.sh (model routing and receipt defaults) =="
scripts/test-codex.sh
```

Run:

```bash
scripts/test-codex.sh
git diff --check
```

Expected: routing tests report OK and `git diff --check` emits nothing.

- [ ] **Step 9: Align the authoritative workflow and active Plan 6 instructions**

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
in receipts, but no route silently downgrades or falls back. `ultra` remains a distinct automatic-
delegation mode and is never a default.

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

- [ ] **Step 10: Record the superseding Red decision before review dispatch**

Update `docs/superpowers/active-ledger.md`:

- set Reality to the actual current HEAD and this model-routing workflow task;
- add a 2026-07-13 User decision that supersedes only the 2026-07-12 routing decision: Main and all
  review/confirmation use Sol/high; implementation/execution use Luna/max; native roles and wrapper
  defaults are aligned; no silent fallback; settlement rules are unchanged;
- retain the older entry, marking it superseded rather than rewriting history;
- add an in-flight Review state with the artifact/base HEAD, prompt/manifest paths and hashes,
  required vendor `Codex`, model/effort Sol/high, and unresolved status.

Do not claim `SETTLED` before a final receipt with zero unresolved Critical/Important findings.

- [ ] **Step 11: Run full verification before freezing the review artifact**

Run:

```bash
scripts/verify.sh
git diff --check
git status --short
```

Expected: `scripts/verify.sh` ends with its PASS line, diff check is silent, and status lists only
the intended configuration, scripts, tests, workflow, Plan 6 instruction, ledger, and this plan if
it was not previously committed. Diagnose any failure before review; do not weaken a check.

- [ ] **Step 12: Build the exact review manifest and prompt**

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
  .codex/config.toml \
  .codex/agents/luna-max.toml \
  .codex/agents/sol-high.toml \
  scripts/codex.sh \
  scripts/test-codex.sh \
  scripts/verify.sh \
  docs/development-workflow.md \
  docs/superpowers/plans/2026-07-12-wave1-plan6-http-ingress-authentication.md \
  docs/superpowers/specs/2026-07-13-codex-native-model-routing-design.md \
  docs/superpowers/plans/2026-07-13-codex-native-model-routing.md
sha256sum .review/codex-model-routing.manifest .review/codex-model-routing-review.md
```

Expected: manifest creation exits zero and both SHA-256 values are recorded in the ledger. Freeze
every manifest file until the result is collected.

- [ ] **Step 13: Dispatch the required independent Sol/high review and reconcile findings**

Run:

```bash
REVIEW_MANIFEST=.review/codex-model-routing.manifest \
CODEX_MODEL=gpt-5.6-sol CODEX_EFFORT=high \
scripts/codex.sh review .review/codex-model-routing-review.md codex-model-routing
```

Expected: a non-empty result and receipt under `/tmp/codex-runs`; receipt fields include
`mode=review`, `model=gpt-5.6-sol`, `effort=high`, and the exact prompt/manifest hashes.

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

- [ ] **Step 14: Record final settlement, verify, and commit the atomic unit**

Update the active ledger with:

- final manifest, prompt, result, and receipt paths plus SHA-256 hashes;
- actual model/effort and timestamps;
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
  scripts/codex.sh scripts/test-codex.sh scripts/verify.sh \
  docs/development-workflow.md \
  docs/superpowers/plans/2026-07-12-wave1-plan6-http-ingress-authentication.md \
  docs/superpowers/plans/2026-07-13-codex-native-model-routing.md \
  docs/superpowers/active-ledger.md
git commit -m "chore: route Codex work by native agent role"
```

Do not add `.review/` operational files. Do not push or open a PR.
