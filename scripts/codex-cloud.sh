#!/usr/bin/env bash
# Submit and collect Codex Cloud candidate work from a local Main agent.
# Cloud status/diff output is provenance only; it is not a bound review result.
set -euo pipefail
umask 077

COMMAND="${1:-}"
shift || true

CODEX_BIN="${CODEX_BIN:-$(command -v codex || true)}"
CODEX_OUT_DIR="${CODEX_OUT_DIR:-/tmp/codex-runs}"
REPO="${CODEX_REPO:-$(git rev-parse --show-toplevel)}"

usage() {
  cat >&2 <<'EOF'
usage:
  scripts/codex-cloud.sh submit <impl|review> <prompt-file> <label> [branch]
  scripts/codex-cloud.sh status <task-id>
  scripts/codex-cloud.sh list [codex-cloud-list-options...]
  scripts/codex-cloud.sh diff <task-id>
  scripts/codex-cloud.sh collect <task-id> <label>
  scripts/codex-cloud.sh verify-receipt <receipt>

submit requires CODEX_CLOUD_ENV and CODEX_CLOUD_ALLOW_ARGV_PROMPT=1.
Only one attempt is currently supported. Automatic apply is intentionally unavailable.
EOF
}

fail() { echo "codex-cloud: $*" >&2; exit 2; }
validate_task_id() { [[ "$1" =~ ^task_[A-Za-z0-9_]+$ ]] || fail "unsafe task id"; }
validate_label() { [[ "$1" =~ ^[A-Za-z0-9._-]{1,80}$ ]] || fail "unsafe label"; }

validate_receipt_value() {
  local name="$1" value="$2"
  if printf '%s' "$value" | od -An -v -tu1 | awk '
    { for (i = 1; i <= NF; i++) if ($i < 32 || $i == 127) found = 1 }
    END { exit !found }
  '; then
    fail "$name contains a control character"
  fi
}

ensure_runtime() {
  [ -n "$CODEX_BIN" ] && [ -x "$CODEX_BIN" ] || fail "codex executable not found"
  git -C "$REPO" rev-parse --is-inside-work-tree >/dev/null 2>&1 ||
    fail "repository not found: $REPO"
  mkdir -p "$CODEX_OUT_DIR"
  chmod 700 "$CODEX_OUT_DIR"
  [ -O "$CODEX_OUT_DIR" ] || fail "output directory is not owned by current user"
  validate_receipt_value "repository path" "$REPO"
  validate_receipt_value "output directory path" "$CODEX_OUT_DIR"
  command -v openssl >/dev/null 2>&1 || fail "openssl is required for receipt HMAC"
  command -v flock >/dev/null 2>&1 || fail "flock is required for Cloud CLI serialization"
}

ensure_clean() {
  [ -z "$(git -C "$REPO" status --porcelain --untracked-files=all)" ] ||
    fail "worktree must be clean; Cloud cannot see local changes"
}

ensure_receipt_key() {
  local key="$CODEX_OUT_DIR/.cloud-receipt-key" candidate
  if [ ! -e "$key" ]; then
    candidate="$(mktemp "$CODEX_OUT_DIR/.cloud-receipt-key.XXXXXX")"
    head -c 32 /dev/urandom >"$candidate"
    chmod 600 "$candidate"
    if ! ln "$candidate" "$key" 2>/dev/null; then
      [ -e "$key" ] || fail "could not initialize receipt integrity key"
    fi
    rm -f "$candidate"
  fi
  [ -f "$key" ] && [ ! -L "$key" ] && [ -O "$key" ] ||
    fail "receipt integrity key must be an owned regular file"
  [ "$(stat -c '%a' "$key")" = 600 ] || fail "receipt integrity key must have mode 600"
  printf '%s\n' "$key"
}

seal_receipt() {
  local receipt="$1" key digest
  key="$(ensure_receipt_key)"
  digest="$(openssl dgst -sha256 -mac HMAC \
    -macopt "hexkey:$(od -An -v -tx1 "$key" | tr -d ' \n')" "$receipt" | awk '{print $NF}')"
  printf 'integrity_hmac_sha256=%s\n' "$digest" >>"$receipt"
}

require_receipt_field_once() {
  local receipt="$1" field="$2"
  [ "$(grep -c "^${field}=" "$receipt")" -eq 1 ] || fail "receipt field $field must occur exactly once"
}

validate_receipt_schema() {
  local receipt="$1" status field value allowed
  while IFS='=' read -r field value; do
    validate_receipt_value "receipt field $field" "$value"
    case "$field" in
      receipt_version|status|vendor|settlement_eligible|cloud_base_verified|mode|label|task_id|task_url|environment_id|branch|remote_commit|attempts|prompt_path|prompt_snapshot_path|prompt_sha256|query_path|query_sha256|submission_output_path|submission_output_sha256|submitted_at|recorded_at|result_path|result_sha256|started_at|completed_at) ;;
      *) fail "receipt contains unknown field: $field" ;;
    esac
  done <"$receipt"
  for field in receipt_version status vendor settlement_eligible; do require_receipt_field_once "$receipt" "$field"; done
  grep -qx 'receipt_version=2' "$receipt" || fail "unsupported receipt version"
  grep -qx 'vendor=codex-cloud' "$receipt" || fail "unexpected receipt vendor"
  grep -qx 'settlement_eligible=false' "$receipt" || fail "Cloud receipt cannot be settlement eligible"
  status="$(sed -n 's/^status=//p' "$receipt")"
  case "$status" in
    submission-pending)
      allowed=" receipt_version status vendor settlement_eligible cloud_base_verified mode label environment_id branch remote_commit attempts prompt_path prompt_snapshot_path prompt_sha256 query_path query_sha256 submitted_at "
      for field in cloud_base_verified mode label environment_id branch remote_commit attempts prompt_path prompt_snapshot_path prompt_sha256 query_path query_sha256 submitted_at; do
        require_receipt_field_once "$receipt" "$field"
      done
      ;;
    submitted)
      allowed=" receipt_version status vendor settlement_eligible cloud_base_verified mode label task_id task_url environment_id branch remote_commit attempts prompt_path prompt_snapshot_path prompt_sha256 query_path query_sha256 submission_output_path submission_output_sha256 submitted_at recorded_at "
      for field in cloud_base_verified mode label task_id task_url environment_id branch remote_commit attempts prompt_path prompt_snapshot_path prompt_sha256 query_path query_sha256 submission_output_path submission_output_sha256 submitted_at recorded_at; do
        require_receipt_field_once "$receipt" "$field"
      done
      ;;
    collected)
      allowed=" receipt_version status vendor settlement_eligible task_id result_path result_sha256 started_at completed_at "
      for field in task_id result_path result_sha256 started_at completed_at; do require_receipt_field_once "$receipt" "$field"; done
      ;;
    *) fail "unexpected receipt status" ;;
  esac
  while IFS='=' read -r field _; do
    [[ "$allowed" == *" $field "* ]] || fail "field $field is not allowed for status $status"
  done <"$receipt"
  grep -qx 'attempts=1' "$receipt" 2>/dev/null || [ "$status" = collected ] || fail "receipt attempt count must be one"
  if [ "$status" != collected ]; then
    grep -qx 'cloud_base_verified=false' "$receipt" || fail "Cloud base must remain unverified"
  fi
}

verify_receipt() {
  local receipt="$1" snapshot key expected actual
  [ -f "$receipt" ] && [ ! -L "$receipt" ] || fail "receipt must be a regular non-symlink file"
  snapshot="$(mktemp "$CODEX_OUT_DIR/.receipt-check.XXXXXX")"
  cp -- "$receipt" "$snapshot"
  [ "$(grep -c '^integrity_hmac_sha256=' "$snapshot")" -eq 1 ] || {
    rm -f "$snapshot"; fail "receipt has no unique integrity seal";
  }
  expected="$(tail -n 1 "$snapshot" | sed -n 's/^integrity_hmac_sha256=//p')"
  [[ "$expected" =~ ^[0-9a-f]{64}$ ]] || { rm -f "$snapshot"; fail "invalid receipt seal"; }
  sed -i '$d' "$snapshot"
  key="$(ensure_receipt_key)"
  actual="$(openssl dgst -sha256 -mac HMAC \
    -macopt "hexkey:$(od -An -v -tx1 "$key" | tr -d ' \n')" "$snapshot" | awk '{print $NF}')"
  [ "$actual" = "$expected" ] || { rm -f "$snapshot"; fail "receipt integrity check failed"; }
  validate_receipt_schema "$snapshot"
  rm -f "$snapshot"
}

move_diagnostic() {
  local diagnostic="$REPO/error.log" suffix="${1:-}"
  if [ -f "$diagnostic" ]; then
    local saved="$CODEX_OUT_DIR/codex-cloud-diagnostic${suffix}-$(date +%Y%m%d-%H%M%S)-$$.log"
    mv "$diagnostic" "$saved"
    chmod 600 "$saved"
    echo "Cloud CLI diagnostic moved to private output: $saved" >&2
  fi
}

run_cloud_cli() {
  local lock="$CODEX_OUT_DIR/.cloud-cli.lock" owner="$CODEX_OUT_DIR/.cloud-cli.owner"
  local diagnostic="$REPO/error.log" lock_fd guardian_pid status
  exec {lock_fd}>"$lock"
  flock -n "$lock_fd" || { exec {lock_fd}>&-; fail "another Cloud CLI command is active"; }
  if [ -f "$owner" ]; then
    move_diagnostic "-recovered"
    rm -f "$owner"
  fi
  [ ! -e "$diagnostic" ] || fail "refusing to overwrite pre-existing $diagnostic"
  printf '%s\n' "$$" >"$owner"
  # A guardian inherits the lock and outlives a hard-interrupted wrapper until the CLI exits.
  # The CLI itself closes the descriptor, so only the guardian controls its lifetime.
  (
    trap '' HUP INT TERM
    (
      exec {lock_fd}>&-
      trap - HUP INT TERM
      cd "$REPO"
      exec "$CODEX_BIN" cloud "$@"
    ) &
    wait "$!"
  ) &
  guardian_pid=$!
  if wait "$guardian_pid"; then status=0; else status=$?; fi
  move_diagnostic
  rm -f "$owner"
  flock -u "$lock_fd"
  exec {lock_fd}>&-
  return "$status"
}

case "$COMMAND" in
  submit)
    MODE="${1:-}"; PROMPT="${2:-}"; LABEL="${3:-}"; BRANCH="${4:-}"
    [ "$MODE" = impl ] || [ "$MODE" = review ] || fail "mode must be impl or review"
    [ -n "$PROMPT" ] && [ -f "$PROMPT" ] && [ ! -L "$PROMPT" ] ||
      fail "prompt must be a non-symlink regular file"
    validate_label "$LABEL"
    ensure_runtime
    ensure_clean

    [ -n "$BRANCH" ] || BRANCH="$(git -C "$REPO" branch --show-current)"
    git check-ref-format --branch "$BRANCH" >/dev/null 2>&1 || fail "invalid branch"
    [ "$BRANCH" != master ] && [ "$BRANCH" != main ] ||
      fail "Cloud work must use a candidate branch, not $BRANCH"
    [ -n "${CODEX_CLOUD_ENV:-}" ] || fail "CODEX_CLOUD_ENV is required"
    [[ "$CODEX_CLOUD_ENV" =~ ^[A-Za-z0-9_-]+$ ]] || fail "unsafe environment id"
    [ "${CODEX_CLOUD_ALLOW_ARGV_PROMPT:-}" = 1 ] ||
      fail "installed CLI exposes prompts in process arguments; acknowledge with CODEX_CLOUD_ALLOW_ARGV_PROMPT=1"

    ATTEMPTS="${CODEX_CLOUD_ATTEMPTS:-1}"
    [ "$ATTEMPTS" = 1 ] || fail "best-of-N is disabled until attempt-specific collection is provenance-bound"

    LOCAL_HEAD="$(git -C "$REPO" rev-parse HEAD)"
    [ "$LOCAL_HEAD" = "$(git -C "$REPO" rev-parse "$BRANCH")" ] || fail "HEAD is not the selected branch tip"
    TRACKING_HEAD="$(git -C "$REPO" rev-parse "refs/remotes/origin/$BRANCH" 2>/dev/null || true)"
    [ "$LOCAL_HEAD" = "$TRACKING_HEAD" ] || fail "local branch is not synchronized with origin/$BRANCH"
    REMOTE_HEAD="$(git -C "$REPO" ls-remote --exit-code origin "refs/heads/$BRANCH" | awk 'NR == 1 {print $1}')"
    [ "$LOCAL_HEAD" = "$REMOTE_HEAD" ] || fail "live remote branch does not match local HEAD"

    PROMPT="$(readlink -f "$PROMPT")"
    case "$PROMPT" in "$REPO/.review/"*) ;; *) fail "prompt must be under $REPO/.review" ;; esac
    validate_receipt_value "prompt path" "$PROMPT"
    STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    OUT="$CODEX_OUT_DIR/codex-cloud-${LABEL}-${MODE}-$(date +%Y%m%d-%H%M%S)-$$.txt"
    PARTIAL="$OUT.partial"; RECEIPT="$OUT.receipt"; RECEIPT_PARTIAL="$RECEIPT.partial"
    PROMPT_SNAPSHOT="$OUT.prompt"; QUERY_PATH="$OUT.query"
    cp -- "$PROMPT" "$PROMPT_SNAPSHOT"
    chmod 600 "$PROMPT_SNAPSHOT"
    [ -s "$PROMPT_SNAPSHOT" ] && [ "$(wc -c <"$PROMPT_SNAPSHOT")" -le 100000 ] ||
      fail "prompt snapshot must contain 1..100000 bytes"
    PROMPT_SHA256="$(sha256sum -- "$PROMPT_SNAPSHOT" | cut -d' ' -f1)"

    MODE_RULE="Implement candidate work and run relevant verification."
    if [ "$MODE" = review ]; then
      MODE_RULE="Review independently without editing. Return explicit Critical/Important/Minor counts. This answer is advisory until separately exported and hash-bound."
    fi
    QUERY="$({ printf '%s\n\n' \
        "IoTKit Cloud candidate task. Mode: $MODE. Requested branch: $BRANCH. Locally observed remote commit: $REMOTE_HEAD." \
        "Read AGENTS.md, docs/cloud-development.md, docs/development-workflow.md, and docs/superpowers/active-ledger.md before acting. $MODE_RULE Do not push, open a PR, merge, release, or claim SETTLED without explicit authority.";
      cat "$PROMPT_SNAPSHOT"; })"
    printf '%s' "$QUERY" >"$QUERY_PATH"
    chmod 600 "$QUERY_PATH"
    QUERY_SHA256="$(sha256sum -- "$QUERY_PATH" | cut -d' ' -f1)"

    {
      printf 'receipt_version=2\nstatus=submission-pending\nvendor=codex-cloud\n'
      printf 'settlement_eligible=false\ncloud_base_verified=false\nmode=%s\nlabel=%s\n' "$MODE" "$LABEL"
      printf 'environment_id=%s\nbranch=%s\nremote_commit=%s\nattempts=%s\n' "$CODEX_CLOUD_ENV" "$BRANCH" "$REMOTE_HEAD" "$ATTEMPTS"
      printf 'prompt_path=%s\nprompt_snapshot_path=%s\nprompt_sha256=%s\n' "$PROMPT" "$PROMPT_SNAPSHOT" "$PROMPT_SHA256"
      printf 'query_path=%s\nquery_sha256=%s\nsubmitted_at=%s\n' "$QUERY_PATH" "$QUERY_SHA256" "$STARTED_AT"
    } >"$RECEIPT_PARTIAL"
    seal_receipt "$RECEIPT_PARTIAL"
    mv "$RECEIPT_PARTIAL" "$RECEIPT"

    if ! run_cloud_cli exec --env "$CODEX_CLOUD_ENV" --branch "$BRANCH" --attempts "$ATTEMPTS" \
      "$QUERY" >"$PARTIAL" 2>&1; then
      [ ! -e "$PARTIAL" ] || mv "$PARTIAL" "$OUT.failed"
      echo "Cloud submission failed or was interrupted; pending receipt/output require reconciliation" >&2
      exit 1
    fi
    mv "$PARTIAL" "$OUT"
    cat "$OUT"
    mapfile -t TASK_IDS < <(grep -Eo 'task_[A-Za-z0-9_]+' "$OUT" | sort -u)
    [ "${#TASK_IDS[@]}" -eq 1 ] || fail "submission outcome unknown: expected exactly one task id in $OUT"
    TASK_ID="${TASK_IDS[0]}"; validate_task_id "$TASK_ID"

    {
      printf 'receipt_version=2\nstatus=submitted\nvendor=codex-cloud\n'
      printf 'settlement_eligible=false\ncloud_base_verified=false\nmode=%s\nlabel=%s\n' "$MODE" "$LABEL"
      printf 'task_id=%s\ntask_url=https://chatgpt.com/codex/tasks/%s\n' "$TASK_ID" "$TASK_ID"
      printf 'environment_id=%s\nbranch=%s\nremote_commit=%s\nattempts=%s\n' "$CODEX_CLOUD_ENV" "$BRANCH" "$REMOTE_HEAD" "$ATTEMPTS"
      printf 'prompt_path=%s\nprompt_snapshot_path=%s\nprompt_sha256=%s\n' "$PROMPT" "$PROMPT_SNAPSHOT" "$PROMPT_SHA256"
      printf 'query_path=%s\nquery_sha256=%s\n' "$QUERY_PATH" "$QUERY_SHA256"
      printf 'submission_output_path=%s\nsubmission_output_sha256=%s\n' "$(readlink -f "$OUT")" "$(sha256sum -- "$OUT" | cut -d' ' -f1)"
      printf 'submitted_at=%s\nrecorded_at=%s\n' "$STARTED_AT" "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    } >"$RECEIPT_PARTIAL"
    seal_receipt "$RECEIPT_PARTIAL"
    mv "$RECEIPT_PARTIAL" "$RECEIPT"
    echo "Cloud task: $TASK_ID" >&2
    echo "Receipt: $RECEIPT" >&2
    ;;

  status|diff)
    TASK_ID="${1:-}"; validate_task_id "$TASK_ID"; ensure_runtime
    run_cloud_cli "$COMMAND" "$TASK_ID"
    ;;
  list)
    ensure_runtime; run_cloud_cli list "$@"
    ;;
  collect)
    TASK_ID="${1:-}"; LABEL="${2:-}"
    validate_task_id "$TASK_ID"; validate_label "$LABEL"; ensure_runtime
    STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    OUT="$CODEX_OUT_DIR/codex-cloud-${LABEL}-collect-$(date +%Y%m%d-%H%M%S)-$$.txt"
    PARTIAL="$OUT.partial"; RECEIPT="$OUT.receipt"
    trap 'rm -f "$PARTIAL" "$RECEIPT.partial"' EXIT
    { printf 'task_id=%s\n== status ==\n' "$TASK_ID"; run_cloud_cli status "$TASK_ID";
      printf '\n== diff ==\n'; run_cloud_cli diff "$TASK_ID"; } >"$PARTIAL"
    mv "$PARTIAL" "$OUT"
    {
      printf 'receipt_version=2\nstatus=collected\nvendor=codex-cloud\n'
      printf 'settlement_eligible=false\ntask_id=%s\n' "$TASK_ID"
      printf 'result_path=%s\nresult_sha256=%s\n' "$(readlink -f "$OUT")" "$(sha256sum -- "$OUT" | cut -d' ' -f1)"
      printf 'started_at=%s\ncompleted_at=%s\n' "$STARTED_AT" "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    } >"$RECEIPT.partial"
    seal_receipt "$RECEIPT.partial"; mv "$RECEIPT.partial" "$RECEIPT"; trap - EXIT
    cat "$OUT"; echo "Collected (not settlement evidence): $RECEIPT" >&2
    ;;
  verify-receipt)
    ensure_runtime; verify_receipt "${1:-}"; echo "receipt integrity: OK"
    ;;
  -h|--help|help|'') usage ;;
  *) usage; fail "unknown command: $COMMAND" ;;
esac
