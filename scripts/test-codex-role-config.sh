#!/usr/bin/env bash
# Deterministic no-model regression test for the repository-owned role preflight.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CHECK="$REPO_ROOT/scripts/check-codex-role-config.sh"
CODEX_BIN="${CODEX_BIN:-/home/kenta/.local/bin/codex}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() {
  echo "codex role config tests: FAIL: $*" >&2
  exit 1
}

command -v jq >/dev/null 2>&1 || fail "jq is required"
[ -x "$CODEX_BIN" ] || fail "Codex binary is missing or not executable: $CODEX_BIN"

CASE_REPO="$TMP/repo"

reset_repo() {
  rm -rf "$CASE_REPO"
  mkdir -p "$CASE_REPO"
  cp -R "$REPO_ROOT/.codex" "$CASE_REPO/.codex"
}

run_case() {
  local label="$1"
  if CODEX_BIN="$CODEX_BIN" CODEX_REPO="$CASE_REPO" "$CHECK" \
    >"$TMP/$label.stdout" 2>"$TMP/$label.stderr"; then
    CASE_STATUS=0
  else
    CASE_STATUS=$?
  fi
}

assert_success() {
  [ "$CASE_STATUS" -eq 0 ] || {
    sed -n '1,120p' "$TMP/$1.stderr" >&2
    fail "$1 unexpectedly failed with status $CASE_STATUS"
  }
  grep -Fqx 'codex role config preflight: OK (3 role layers)' "$TMP/$1.stdout" || {
    fail "$1 did not report the complete role-layer preflight"
  }
}

assert_failure() {
  [ "$CASE_STATUS" -ne 0 ] || fail "$1 unexpectedly succeeded"
}

assert_failure_message() {
  local label="$1"
  local expected="$2"
  grep -Fq "$expected" "$TMP/$label.stderr" || {
    sed -n '1,120p' "$TMP/$label.stderr" >&2
    fail "$label did not fail at the expected preflight boundary"
  }
}

reset_repo
run_case positive
assert_success positive

reset_repo
rm -f "$CASE_REPO/.codex/agents/sol-high.toml"
run_case missing-role
assert_failure missing-role
assert_failure_message missing-role 'role reviewer layer is missing'

reset_repo
printf '%s\n' 'model = [' > "$CASE_REPO/.codex/agents/sol-high.toml"
run_case malformed-role
assert_failure malformed-role
assert_failure_message malformed-role 'strict parser rejected role reviewer layer'

reset_repo
printf '%s\n' 'unknown_role_layer_key = true' >> "$CASE_REPO/.codex/agents/sol-high.toml"
run_case unknown-role-key
assert_failure unknown-role-key
assert_failure_message unknown-role-key 'strict parser rejected role reviewer layer'

echo "codex role config tests: OK"
