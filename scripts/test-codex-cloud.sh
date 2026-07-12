#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

REMOTE="$TMP/remote.git"; REPO="$TMP/repo"; OUT="$TMP/out"; FAKE_CODEX="$TMP/codex"
git init --bare -q "$REMOTE"
git init -q "$REPO"
git -C "$REPO" config user.name test
git -C "$REPO" config user.email test@example.invalid
mkdir -p "$REPO/.review"
printf 'unique prompt body\n' >"$REPO/.review/prompt.md"
printf '/.review/\n' >"$REPO/.gitignore"
printf 'tracked\n' >"$REPO/tracked"
git -C "$REPO" add .gitignore tracked
git -C "$REPO" commit -qm init
git -C "$REPO" switch -qc cloud/test
git -C "$REPO" remote add origin "$REMOTE"
git -C "$REPO" push -qu origin cloud/test

cat >"$FAKE_CODEX" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [ "${FAKE_DIAGNOSTIC:-}" = 1 ]; then printf 'private diagnostic\n' >error.log; fi
if [ "${FAKE_SLOW:-}" = 1 ]; then
  [ ! -e "${FAKE_ACTIVE}.active" ] || touch "${FAKE_ACTIVE}.overlap"
  touch "${FAKE_ACTIVE}.active"
  sleep 1
  rm -f "${FAKE_ACTIVE}.active"
fi
case "${1:-} ${2:-}" in
  "cloud exec")
    [ "${3:-}" = --env ] && [ "${4:-}" = env_test ]
    [ "${5:-}" = --branch ] && [ "${6:-}" = cloud/test ]
    [ "${7:-}" = --attempts ] && [ "${8:-}" = "${EXPECTED_ATTEMPTS:-1}" ]
    [ "$#" -eq 9 ]
    [[ "${*: -1}" == *"unique prompt body"* ]] || exit 10
    if [ -n "${FAKE_CAPTURE_QUERY:-}" ]; then printf '%s' "${*: -1}" >"$FAKE_CAPTURE_QUERY"; fi
    [ "${FAKE_EXEC_FAIL:-}" != 1 ] || { printf 'submission may be unknown\n'; exit 8; }
    if [ "${FAKE_MULTI_ID:-}" = 1 ]; then
      printf 'task_e_first task_e_second\n'
    else
      printf 'Submitted: https://chatgpt.com/codex/tasks/task_e_test123\n'
    fi
    ;;
  "cloud status") printf '[READY] fake cloud task\n' ;;
  "cloud diff") printf 'diff --git a/example b/example\n' ;;
  "cloud list") printf '{"tasks":[]}\n' ;;
  *) printf 'unexpected fake codex arguments: %s\n' "$*" >&2; exit 9 ;;
esac
EOF
chmod +x "$FAKE_CODEX"

run_cloud() {
  CODEX_BIN="$FAKE_CODEX" CODEX_REPO="$REPO" CODEX_OUT_DIR="$OUT" \
    CODEX_CLOUD_ENV="env_test" CODEX_CLOUD_ALLOW_ARGV_PROMPT=1 \
    "$ROOT/scripts/codex-cloud.sh" "$@"
}

run_cloud submit impl "$REPO/.review/prompt.md" candidate cloud/test >"$TMP/submit.out"
SUBMIT_RECEIPT="$(find "$OUT" -type f -name 'codex-cloud-candidate-impl-*.txt.receipt' -print -quit)"
[ -s "$SUBMIT_RECEIPT" ]
grep -qx 'status=submitted' "$SUBMIT_RECEIPT"
grep -qx 'settlement_eligible=false' "$SUBMIT_RECEIPT"
grep -qx 'cloud_base_verified=false' "$SUBMIT_RECEIPT"
grep -qx 'task_id=task_e_test123' "$SUBMIT_RECEIPT"
grep -qx 'attempts=1' "$SUBMIT_RECEIPT"
grep -qx "remote_commit=$(git -C "$REPO" rev-parse HEAD)" "$SUBMIT_RECEIPT"
grep -Eq '^integrity_hmac_sha256=[0-9a-f]{64}$' "$SUBMIT_RECEIPT"
[ "$(stat -c '%a' "$OUT/.cloud-receipt-key")" = 600 ]
run_cloud verify-receipt "$SUBMIT_RECEIPT" | grep -qx 'receipt integrity: OK'
PROMPT_SNAPSHOT="$(sed -n 's/^prompt_snapshot_path=//p' "$SUBMIT_RECEIPT")"
QUERY_PATH="$(sed -n 's/^query_path=//p' "$SUBMIT_RECEIPT")"
grep -qx "prompt_sha256=$(sha256sum "$PROMPT_SNAPSHOT" | cut -d' ' -f1)" "$SUBMIT_RECEIPT"
grep -qx "query_sha256=$(sha256sum "$QUERY_PATH" | cut -d' ' -f1)" "$SUBMIT_RECEIPT"
FAKE_CAPTURE_QUERY="$TMP/query.capture" run_cloud submit impl "$REPO/.review/prompt.md" exact-query cloud/test >/dev/null
EXACT_RECEIPT="$(find "$OUT" -type f -name 'codex-cloud-exact-query-impl-*.txt.receipt' -print -quit)"
cmp "$TMP/query.capture" "$(sed -n 's/^query_path=//p' "$EXACT_RECEIPT")"

cp "$SUBMIT_RECEIPT" "$TMP/original.receipt"
sed -i 's/^attempts=1$/attempts=2/' "$SUBMIT_RECEIPT"
if run_cloud verify-receipt "$SUBMIT_RECEIPT" >/dev/null 2>&1; then
  echo "tampered receipt unexpectedly verified" >&2; exit 1
fi
cp "$TMP/original.receipt" "$SUBMIT_RECEIPT"

cp "$SUBMIT_RECEIPT" "$TMP/duplicate.receipt"
sed -i '$d' "$TMP/duplicate.receipt"
printf 'attempts=1\n' >>"$TMP/duplicate.receipt"
KEY_HEX="$(od -An -v -tx1 "$OUT/.cloud-receipt-key" | tr -d ' \n')"
DUP_HMAC="$(openssl dgst -sha256 -mac HMAC -macopt "hexkey:$KEY_HEX" "$TMP/duplicate.receipt" | awk '{print $NF}')"
printf 'integrity_hmac_sha256=%s\n' "$DUP_HMAC" >>"$TMP/duplicate.receipt"
if run_cloud verify-receipt "$TMP/duplicate.receipt" >/dev/null 2>&1; then
  echo "valid-HMAC receipt with duplicate field unexpectedly verified" >&2; exit 1
fi

if CODEX_CLOUD_ALLOW_ARGV_PROMPT= CODEX_BIN="$FAKE_CODEX" CODEX_REPO="$REPO" \
  CODEX_OUT_DIR="$OUT" CODEX_CLOUD_ENV=env_test "$ROOT/scripts/codex-cloud.sh" \
  submit impl "$REPO/.review/prompt.md" no-gate cloud/test >/dev/null 2>&1; then
  echo "submit without argv disclosure acknowledgement succeeded" >&2; exit 1
fi
printf 'outside\n' >"$TMP/outside.md"
if run_cloud submit impl "$TMP/outside.md" outside cloud/test >/dev/null 2>&1; then
  echo "outside-repository prompt succeeded" >&2; exit 1
fi
ln -s prompt.md "$REPO/.review/link.md"
if run_cloud submit impl "$REPO/.review/link.md" symlink cloud/test >/dev/null 2>&1; then
  echo "symlink prompt succeeded" >&2; exit 1
fi
CONTROL_PROMPT="$REPO/.review/line"$'\n'"injected.md"
printf 'unique prompt body\n' >"$CONTROL_PROMPT"
if run_cloud submit impl "$CONTROL_PROMPT" control cloud/test >/dev/null 2>&1; then
  echo "control-character prompt path succeeded" >&2; exit 1
fi
if run_cloud submit impl "$REPO/.review/prompt.md" default master >/dev/null 2>&1; then
  echo "default-branch submit succeeded" >&2; exit 1
fi
if CODEX_CLOUD_ATTEMPTS=2 run_cloud submit impl "$REPO/.review/prompt.md" extra cloud/test >/dev/null 2>&1; then
  echo "unauthorized extra attempts succeeded" >&2; exit 1
fi

run_cloud collect task_e_test123 candidate >"$TMP/collect.out"
COLLECT_RECEIPT="$(find "$OUT" -type f -name 'codex-cloud-candidate-collect-*.txt.receipt' -print -quit)"
grep -qx 'settlement_eligible=false' "$COLLECT_RECEIPT"
run_cloud verify-receipt "$COLLECT_RECEIPT" >/dev/null
grep -q '^== status ==$' "$TMP/collect.out"; grep -q '^== diff ==$' "$TMP/collect.out"

FAKE_DIAGNOSTIC=1 run_cloud list >"$TMP/list.out"
[ ! -e "$REPO/error.log" ]
find "$OUT" -maxdepth 1 -type f -name 'codex-cloud-diagnostic-*.log' | grep -q .
printf 'preserve me\n' >"$REPO/error.log"
if run_cloud list >/dev/null 2>&1; then echo "pre-existing diagnostic was not rejected" >&2; exit 1; fi
grep -qx 'preserve me' "$REPO/error.log"; rm "$REPO/error.log"

FAKE_ACTIVE="$TMP/slow" FAKE_SLOW=1 run_cloud list >/dev/null &
SLOW_PID=$!
for _ in $(seq 1 100); do [ -e "$TMP/slow.active" ] && break; sleep 0.01; done
[ -e "$TMP/slow.active" ] || { echo "slow fake CLI did not start" >&2; exit 1; }
if run_cloud list >/dev/null 2>&1; then echo "concurrent invocation unexpectedly succeeded" >&2; exit 1; fi
wait "$SLOW_PID"
[ ! -e "$TMP/slow.overlap" ] && [ ! -e "$OUT/.cloud-cli.owner" ]

FAKE_ACTIVE="$TMP/orphan" FAKE_SLOW=1 FAKE_DIAGNOSTIC=1 run_cloud list >/dev/null 2>&1 &
WRAPPER_JOB=$!
for _ in $(seq 1 100); do
  [ -e "$TMP/orphan.active" ] && [ -s "$OUT/.cloud-cli.owner" ] && break
  sleep 0.01
done
[ -e "$TMP/orphan.active" ] || { echo "orphan probe fake CLI did not start" >&2; exit 1; }
OWNER_PID="$(cat "$OUT/.cloud-cli.owner")"
kill -KILL "$OWNER_PID"
wait "$WRAPPER_JOB" 2>/dev/null || true
[ -e "$TMP/orphan.active" ] || { echo "fake CLI did not outlive killed wrapper" >&2; exit 1; }
if run_cloud list >/dev/null 2>&1; then
  echo "second invocation entered while orphan CLI was active" >&2; exit 1
fi
for _ in $(seq 1 200); do [ ! -e "$TMP/orphan.active" ] && break; sleep 0.01; done
[ ! -e "$TMP/orphan.active" ] || { echo "orphan fake CLI did not exit" >&2; exit 1; }
run_cloud list >/dev/null
[ ! -e "$OUT/.cloud-cli.owner" ] && [ ! -e "$REPO/error.log" ]
find "$OUT" -maxdepth 1 -type f -name 'codex-cloud-diagnostic-recovered-*.log' | grep -q .

touch "$REPO/untracked"
if run_cloud submit impl "$REPO/.review/prompt.md" dirty cloud/test >/dev/null 2>&1; then
  echo "dirty submit succeeded" >&2; exit 1
fi
rm "$REPO/untracked"
printf 'local-only\n' >>"$REPO/tracked"; git -C "$REPO" add tracked; git -C "$REPO" commit -qm local-only
if run_cloud submit impl "$REPO/.review/prompt.md" unpushed cloud/test >/dev/null 2>&1; then
  echo "unpushed submit succeeded" >&2; exit 1
fi
git -C "$REPO" reset -q --hard 'HEAD^'

OTHER="$TMP/other"
git clone -q --branch cloud/test "$REMOTE" "$OTHER"
git -C "$OTHER" config user.name test
git -C "$OTHER" config user.email test@example.invalid
printf 'remote-only\n' >>"$OTHER/tracked"
git -C "$OTHER" add tracked; git -C "$OTHER" commit -qm remote-only
git -C "$OTHER" push -qu origin cloud/test
if run_cloud submit impl "$REPO/.review/prompt.md" remote-moved cloud/test >/dev/null 2>&1; then
  echo "submit accepted a moved live remote with stale tracking ref" >&2; exit 1
fi
git -C "$REPO" push -q --force origin HEAD:cloud/test

FAKE_EXEC_FAIL=1 run_cloud submit impl "$REPO/.review/prompt.md" failed cloud/test >/dev/null 2>&1 || true
FAILED_RECEIPT="$(find "$OUT" -type f -name 'codex-cloud-failed-impl-*.txt.receipt' -print -quit)"
grep -qx 'status=submission-pending' "$FAILED_RECEIPT"
run_cloud verify-receipt "$FAILED_RECEIPT" >/dev/null
find "$OUT" -type f -name 'codex-cloud-failed-impl-*.txt.failed' | grep -q .

if FAKE_MULTI_ID=1 run_cloud submit impl "$REPO/.review/prompt.md" ambiguous cloud/test >/dev/null 2>&1; then
  echo "ambiguous task IDs unexpectedly succeeded" >&2; exit 1
fi
AMBIGUOUS_RECEIPT="$(find "$OUT" -type f -name 'codex-cloud-ambiguous-impl-*.txt.receipt' -print -quit)"
grep -qx 'status=submission-pending' "$AMBIGUOUS_RECEIPT"

if run_cloud status '../unsafe' >/dev/null 2>&1; then echo "unsafe task id succeeded" >&2; exit 1; fi
if run_cloud apply task_e_test123 >/dev/null 2>&1; then echo "automatic apply unexpectedly exists" >&2; exit 1; fi

echo "codex-cloud harness tests: OK"
