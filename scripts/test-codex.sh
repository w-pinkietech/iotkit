#!/usr/bin/env bash
# Deterministic, no-network regression test for native Codex routing and receipts.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() {
  echo "codex routing tests: FAIL: $*" >&2
  exit 1
}

assert_file() {
  [ -f "$1" ] || fail "missing file: $1"
}

assert_contains() {
  local path="$1"
  local text="$2"
  grep -Fqx "$text" "$path" || fail "missing exact line '$text' in $path"
}

assert_not_contains() {
  local path="$1"
  local pattern="$2"
  ! grep -Eq "$pattern" "$path" || fail "unexpected pattern '$pattern' in $path"
}

assert_equals() {
  local expected="$1"
  local actual="$2"
  local label="$3"
  [ "$expected" = "$actual" ] || fail "$label: expected '$expected', got '$actual'"
}

assert_status_nonzero() {
  [ "$1" -ne 0 ] || fail "$2 unexpectedly succeeded"
}

assert_no_publication() {
  local label="$1"
  if find "$OUT" -maxdepth 1 -type f \
      \( -name "codex-${label}-*" -o -name "codex-${label}-*.partial" \) \
      -print -quit | grep -q .; then
    fail "$label published a result, receipt, event stream, or partial artifact"
  fi
}

assert_field() {
  local receipt="$1"
  local key="$2"
  local value="$3"
  grep -Fqx "${key}=${value}" "$receipt" || fail "$receipt lacks ${key}=${value}"
}

write_expected_argv() {
  printf '%s\0' "$@" > "$EXPECTED"
}

command -v jq >/dev/null 2>&1 || fail "jq is required"
command -v git >/dev/null 2>&1 || fail "git is required"

# The project configuration check is deliberately first.  On the required RED
# run, this fails before the fake binary can be invoked.
CONFIG="$REPO_ROOT/.codex/config.toml"
LUNA_ROLE="$REPO_ROOT/.codex/agents/luna-max.toml"
SOL_ROLE="$REPO_ROOT/.codex/agents/sol-high.toml"
assert_file "$CONFIG"
assert_file "$LUNA_ROLE"
assert_file "$SOL_ROLE"
assert_contains "$CONFIG" 'model = "gpt-5.6-sol"'
assert_contains "$CONFIG" 'model_reasoning_effort = "high"'
assert_contains "$CONFIG" 'review_model = "gpt-5.6-sol"'
assert_contains "$CONFIG" '[agents.implementer]'
assert_contains "$CONFIG" '[agents.executor]'
assert_contains "$CONFIG" '[agents.reviewer]'
assert_contains "$CONFIG" 'config_file = "agents/luna-max.toml"'
assert_contains "$CONFIG" 'config_file = "agents/sol-high.toml"'
assert_contains "$LUNA_ROLE" 'model = "gpt-5.6-luna"'
assert_contains "$LUNA_ROLE" 'model_reasoning_effort = "max"'
assert_contains "$LUNA_ROLE" 'sandbox_mode = "workspace-write"'
assert_contains "$LUNA_ROLE" 'approval_policy = "never"'
assert_contains "$SOL_ROLE" 'model = "gpt-5.6-sol"'
assert_contains "$SOL_ROLE" 'model_reasoning_effort = "high"'
assert_contains "$SOL_ROLE" 'sandbox_mode = "read-only"'
assert_contains "$SOL_ROLE" 'approval_policy = "never"'

REPO="$TMP/repo"
OUT="$TMP/out"
FAKE="$TMP/fake-codex"
mkdir -p "$OUT"
git init -q "$REPO"
git -C "$REPO" config user.email test@example.invalid
git -C "$REPO" config user.name codex-routing-test

PROMPT="$REPO/prompt.md"
printf '%s\n' 'bounded fake Codex prompt' > "$PROMPT"
printf '%s\n' 'manifested fixture content' > "$REPO/fixture.txt"

FAKE_CAPTURE="$TMP/fake-argv.nul"
FAKE_STDIN_CAPTURE="$TMP/fake-stdin"
FAKE_MARKER="$TMP/fake-invoked"
FAKE_MUTATE_MANIFEST="$TMP/mutate-manifest"
PRE_RECEIPT_HOOK="$TMP/mutate-before-receipt"
PRE_RECEIPT_MARKER="$TMP/pre-receipt-hook-ran"

cat > "$FAKE" <<'FAKE_CODEX'
#!/usr/bin/env bash
set -euo pipefail

fail() {
  echo "fake codex rejected argv: $*" >&2
  exit 91
}

[ -n "${FAKE_ARGV_CAPTURE:-}" ] || fail "missing argv capture path"
[ -n "${FAKE_STDIN_CAPTURE:-}" ] || fail "missing stdin capture path"
[ -n "${FAKE_INVOCATION_MARKER:-}" ] || fail "missing invocation marker path"
printf '%s\0' "$@" > "$FAKE_ARGV_CAPTURE"
: > "$FAKE_INVOCATION_MARKER"

# Parse one exact grammar.  The parent test separately compares the complete
# NUL-delimited vector, while this parser rejects misplaced, duplicate, or
# unexpected options before producing any fixture output.
[ "$#" -ge 2 ] && [ "$1" = "-a" ] && [ "$2" = "never" ] || fail "approval root option"
shift 2
[ "$#" -ge 1 ] && [ "$1" = "exec" ] || fail "exec subcommand"
shift
[ "$#" -ge 2 ] && [ "$1" = "-s" ] || fail "sandbox option"
[ "$2" = "${FAKE_EXPECTED_SANDBOX:?}" ] || fail "sandbox value"
shift 2
[ "$#" -ge 1 ] && [ "$1" = "--skip-git-repo-check" ] || fail "skip-git-repo-check"
shift
[ "$#" -ge 2 ] && [ "$1" = "-C" ] || fail "repo option"
[ "$2" = "${FAKE_EXPECTED_REPO:?}" ] || fail "repo value"
shift 2
[ "$#" -ge 2 ] && [ "$1" = "-m" ] || fail "model option"
[ "$2" = "${FAKE_EXPECTED_MODEL:?}" ] || fail "model value"
shift 2
[ "$#" -ge 2 ] && [ "$1" = "-c" ] || fail "config option"
[ "$2" = "model_reasoning_effort=${FAKE_EXPECTED_EFFORT:?}" ] || fail "effort value"
shift 2
[ "$#" -ge 1 ] && [ "$1" = "--json" ] || fail "json option"
shift
[ "$#" -ge 2 ] && [ "$1" = "-o" ] || fail "output option"
OUTPUT="$2"
[[ "$OUTPUT" == *.partial ]] || fail "output is not private partial"
shift 2
[ "$#" -eq 1 ] && [ "$1" = "-" ] || fail "prompt stdin terminator or trailing argv"

cat > "$FAKE_STDIN_CAPTURE"
cmp -s "${FAKE_EXPECTED_PROMPT:?}" "$FAKE_STDIN_CAPTURE" || fail "prompt stdin differs byte-for-byte"

printf 'fake result (%s)\n' "${FAKE_FIXTURE:?}" > "$OUTPUT"
case "$FAKE_FIXTURE" in
  success|mutated-manifest|empty|fake-failure)
    if [ "$FAKE_FIXTURE" = empty ]; then
      : > "$OUTPUT"
    fi
    printf '%s\n' \
      '{"type":"thread.started","thread_id":"fake-thread"}' \
      '{"type":"turn.started","turn_id":"fake-turn"}' \
      '{"type":"turn.completed","turn_id":"fake-turn"}'
    ;;
  reroute)
    printf '%s\n' \
      '{"type":"thread.started","thread_id":"fake-thread"}' \
      '{"type":"turn.started","turn_id":"fake-turn"}' \
      '{"type":"model_reroute","from":"gpt-5.6-luna","to":"fallback"}' \
      '{"type":"turn.completed","turn_id":"fake-turn"}'
    ;;
  malformed)
    printf '%s\n' \
      '{"type":"thread.started"}' \
      'not-json' \
      '{"type":"turn.completed"}'
    ;;
  turn-failed)
    printf '%s\n' \
      '{"type":"thread.started"}' \
      '{"type":"turn.started"}' \
      '{"type":"turn.failed"}'
    ;;
  error)
    printf '%s\n' \
      '{"type":"thread.started"}' \
      '{"type":"turn.started"}' \
      '{"type":"error","message":"fixture failure"}'
    ;;
  incomplete)
    printf '%s\n' \
      '{"type":"thread.started"}' \
      '{"type":"turn.started"}'
    ;;
  reordered)
    printf '%s\n' \
      '{"type":"turn.completed"}' \
      '{"type":"thread.started"}' \
      '{"type":"turn.started"}'
    ;;
  duplicate)
    printf '%s\n' \
      '{"type":"thread.started"}' \
      '{"type":"thread.started"}' \
      '{"type":"turn.started"}' \
      '{"type":"turn.completed"}'
    ;;
  after-completion)
    printf '%s\n' \
      '{"type":"thread.started"}' \
      '{"type":"turn.started"}' \
      '{"type":"turn.completed"}' \
      '{"type":"thread.started"}'
    ;;
  *)
    fail "unknown fixture ${FAKE_FIXTURE}"
    ;;
esac

if [ "$FAKE_FIXTURE" = mutated-manifest ]; then
  printf '%s\n' 'mutated-after-dispatch' >> "${FAKE_MUTATE_MANIFEST:?}"
fi
if [ "$FAKE_FIXTURE" = fake-failure ]; then
  exit 73
fi
FAKE_CODEX
chmod +x "$FAKE"
printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' ': > "${FAKE_PRE_RECEIPT_MARKER:?}"' 'printf "%s\\n" mutated-before-receipt >> "${FAKE_MUTATE_MANIFEST:?}"' > "$PRE_RECEIPT_HOOK"
chmod +x "$PRE_RECEIPT_HOOK"

write_manifest() {
  local manifest="$1"
  rm -f "$manifest" "$manifest.partial"
  (cd "$REPO" && "$REPO_ROOT/scripts/review-manifest.sh" "$manifest" prompt.md fixture.txt >/dev/null)
}

run_case() {
  local label="$1"
  local mode="$2"
  local fixture="$3"
  local requested_model="$4"
  local requested_effort="$5"
  local expected_model="$6"
  local expected_effort="$7"
  local sandbox="$8"
  local with_manifest="$9"

  find "$OUT" -mindepth 1 -maxdepth 1 -type f -delete
  rm -f "$FAKE_CAPTURE" "$FAKE_STDIN_CAPTURE" "$FAKE_MARKER" "$PRE_RECEIPT_MARKER"
  local manifest="$TMP/${label}.manifest"
  if [ "$with_manifest" = yes ]; then
    write_manifest "$manifest"
  else
    rm -f "$manifest" "$manifest.partial"
  fi

  CASE_LABEL="$label"
  CASE_MODE="$mode"
  CASE_MANIFEST="$manifest"
  CASE_EXPECTED_MODEL="$expected_model"
  CASE_EXPECTED_EFFORT="$expected_effort"
  CASE_SANDBOX="$sandbox"
  if (
    export CODEX_BIN="$FAKE"
    export CODEX_REPO="$REPO"
    export CODEX_OUT_DIR="$OUT"
    export CODEX_MODEL="$requested_model"
    export CODEX_EFFORT="$requested_effort"
    export FAKE_ARGV_CAPTURE="$FAKE_CAPTURE"
    export FAKE_STDIN_CAPTURE="$FAKE_STDIN_CAPTURE"
    export FAKE_INVOCATION_MARKER="$FAKE_MARKER"
    export FAKE_EXPECTED_PROMPT="$PROMPT"
    export FAKE_EXPECTED_REPO="$REPO"
    export FAKE_EXPECTED_MODEL="$expected_model"
    export FAKE_EXPECTED_EFFORT="$expected_effort"
    export FAKE_EXPECTED_SANDBOX="$sandbox"
    export FAKE_FIXTURE="$fixture"
    export FAKE_MUTATE_MANIFEST="$manifest"
    if [ "$label" = mutated-before-receipt ]; then
      export CODEX_TEST_PRE_RECEIPT_HOOK="$PRE_RECEIPT_HOOK"
      export FAKE_PRE_RECEIPT_MARKER="$PRE_RECEIPT_MARKER"
    else
      unset CODEX_TEST_PRE_RECEIPT_HOOK FAKE_PRE_RECEIPT_MARKER
    fi
    if [ "$with_manifest" = yes ]; then
      export REVIEW_MANIFEST="$manifest"
    else
      unset REVIEW_MANIFEST
    fi
    "$REPO_ROOT/scripts/codex.sh" "$mode" "$PROMPT" "$label"
  ) > "$TMP/${label}.stdout" 2> "$TMP/${label}.stderr"; then
    CASE_STATUS=0
  else
    CASE_STATUS=$?
  fi
}

assert_exact_argv() {
  local label="$1"
  local mode="$2"
  local expected_model="$3"
  local expected_effort="$4"
  local sandbox="$5"
  [ -s "$FAKE_CAPTURE" ] || fail "$label did not capture fake argv"
  [ -s "$FAKE_STDIN_CAPTURE" ] || fail "$label did not capture fake stdin"
  cmp -s "$PROMPT" "$FAKE_STDIN_CAPTURE" || fail "$label stdin was not the manifested prompt"
  local partial
  partial="$(tr '\0' '\n' < "$FAKE_CAPTURE" | awk '$0 == "-o" { getline; print; exit }')"
  [ -n "$partial" ] || fail "$label did not pass an output path"
  EXPECTED="$TMP/${label}.expected-argv.nul"
  write_expected_argv \
    -a never exec \
    -s "$sandbox" \
    --skip-git-repo-check \
    -C "$REPO" \
    -m "$expected_model" \
    -c "model_reasoning_effort=${expected_effort}" \
    --json \
    -o "$partial" \
    -
  cmp -s "$EXPECTED" "$FAKE_CAPTURE" || fail "$label argv was not the exact ordered vector"
}

assert_success_case() {
  local label="$1"
  local mode="$2"
  local expected_model="$3"
  local expected_effort="$4"
  local sandbox="$5"
  [ "$CASE_STATUS" -eq 0 ] || {
    sed -n '1,120p' "$TMP/${label}.stderr" >&2
    fail "$label failed unexpectedly with status $CASE_STATUS"
  }
  assert_exact_argv "$label" "$mode" "$expected_model" "$expected_effort" "$sandbox"
  local result
  result="$(find "$OUT" -maxdepth 1 -type f -name "codex-${label}-${mode}-*.txt" -print)"
  [ -n "$result" ] || fail "$label result was not published"
  [ "$(printf '%s\n' "$result" | wc -l)" -eq 1 ] || fail "$label published multiple results"
  local event_stream="${result}.events.jsonl"
  local receipt="${result}.receipt"
  assert_file "$result"
  assert_file "$event_stream"
  assert_file "$receipt"
  [ -s "$result" ] || fail "$label result is empty"
  [ -s "$event_stream" ] || fail "$label event stream is empty"
  [ -s "$receipt" ] || fail "$label receipt is empty"
  local prompt_path manifest_path result_path event_path
  prompt_path="$(readlink -f "$PROMPT")"
  manifest_path="$(readlink -f "$CASE_MANIFEST")"
  result_path="$(readlink -f "$result")"
  event_path="$(readlink -f "$event_stream")"
  assert_field "$receipt" receipt_version 2
  assert_field "$receipt" status success
  assert_field "$receipt" vendor codex
  assert_field "$receipt" mode "$mode"
  assert_field "$receipt" requested_model "$expected_model"
  assert_field "$receipt" requested_effort "$expected_effort"
  assert_field "$receipt" observed_model UNAVAILABLE
  assert_field "$receipt" observed_effort UNAVAILABLE
  assert_field "$receipt" sandbox_mode "$sandbox"
  assert_field "$receipt" approval_policy never
  assert_field "$receipt" model_reroute_observed false
  assert_field "$receipt" prompt_path "$prompt_path"
  assert_field "$receipt" prompt_sha256 "$(sha256sum -- "$PROMPT" | cut -d' ' -f1)"
  assert_field "$receipt" artifact_manifest_path "$manifest_path"
  assert_field "$receipt" artifact_manifest_sha256 "$(sha256sum -- "$CASE_MANIFEST" | cut -d' ' -f1)"
  assert_field "$receipt" result_path "$result_path"
  assert_field "$receipt" result_sha256 "$(sha256sum -- "$result" | cut -d' ' -f1)"
  assert_field "$receipt" event_stream_path "$event_path"
  assert_field "$receipt" event_stream_sha256 "$(sha256sum -- "$event_stream" | cut -d' ' -f1)"
  assert_not_contains "$receipt" '^model='
  assert_not_contains "$receipt" '^effort='
  assert_equals 'model_reroute_observed=false' "$("$REPO_ROOT/scripts/check-codex-events.sh" "$event_stream")" \
    "$label event evidence"
}

assert_failure_case() {
  local label="$1"
  local fake_must_run="$2"
  assert_status_nonzero "$CASE_STATUS" "$label"
  assert_no_publication "$label"
  if [ "$fake_must_run" = yes ]; then
    assert_file "$FAKE_MARKER"
  else
    [ ! -e "$FAKE_MARKER" ] || fail "$label invoked fake Codex before preflight rejection"
  fi
}

# Defaults: implementation is Luna/max and review is Sol/high.
run_case impl-default impl success '' '' gpt-5.6-luna max workspace-write yes
assert_success_case impl-default impl gpt-5.6-luna max workspace-write

run_case review-default review success '' '' gpt-5.6-sol high read-only yes
assert_success_case review-default review gpt-5.6-sol high read-only

# Explicit values remain visible as requested values while sandbox and approval
# policy stay tied to the selected mode.
run_case explicit-override review success gpt-5.6-luna max gpt-5.6-luna max read-only yes
assert_success_case explicit-override review gpt-5.6-luna max read-only

# Preflight failures must not invoke Codex or leave any publication/partial.
run_case invalid-effort impl success gpt-5.6-luna ultra gpt-5.6-luna ultra workspace-write yes
assert_failure_case invalid-effort no

run_case absent-manifest review success '' '' gpt-5.6-sol high read-only no
assert_failure_case absent-manifest no

# Post-run binding failure proves the fake ran, but publication remains fail-closed.
run_case mutated-manifest review mutated-manifest '' '' gpt-5.6-sol high read-only yes
assert_failure_case mutated-manifest yes

for fixture in reroute malformed turn-failed error incomplete empty fake-failure; do
  run_case "$fixture" review "$fixture" '' '' gpt-5.6-sol high read-only yes
  assert_failure_case "$fixture" yes
done

for fixture in reordered duplicate after-completion; do
  run_case "$fixture" review "$fixture" '' '' gpt-5.6-sol high read-only yes
  assert_failure_case "$fixture" yes
done

run_case mutated-before-receipt review success '' '' gpt-5.6-sol high read-only yes
assert_failure_case mutated-before-receipt yes
assert_file "$PRE_RECEIPT_MARKER"

echo "codex routing tests: OK"
