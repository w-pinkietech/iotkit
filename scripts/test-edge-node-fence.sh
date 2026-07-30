#!/usr/bin/env bash
set -euo pipefail
umask 077

repo_root=$(cd "$(dirname "$(realpath "${BASH_SOURCE[0]}")")/.." && pwd -P)
root=$(mktemp -d)
cleanup() {
  rm -rf -- "$root"
}
trap cleanup EXIT

mkdir -m 700 "$root/bin" "$root/edge" "$root/output-parent"
mkdir -m 700 "$root/edge/mosquitto"
printf "node-01:\$old-hash\n" >"$root/edge/mosquitto/passwords"
chmod 640 "$root/edge/mosquitto/passwords"
printf 'IOTKIT_EDGE_ID=edge-test\nIOTKIT_EDGE_USERNAME=edge-archive\n' >"$root/edge/edge.env"
cat >"$root/edge/mosquitto/acl" <<'EOF'
user node-01
topic write iotkit/v1/edge-nodes/node-01/recovery/result
topic write iotkit/v1/edge-nodes/node-01/recovery/completion-ack
topic read iotkit/v1/edge-nodes/node-01/recovery/request
topic read iotkit/v1/edge-nodes/node-01/recovery/completion

user edge-archive
topic read iotkit/v1/edge-nodes/+/recovery/result
topic read iotkit/v1/edge-nodes/+/recovery/completion-ack
topic write iotkit/v1/edge-nodes/+/recovery/request
topic write iotkit/v1/edge-nodes/+/recovery/completion
EOF
printf '{}\n' >"$root/edge/mosquitto/credential-generations.json"

cat >"$root/bin/docker" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == run ]]; then
  work=""
  while (($#)); do
    if [[ "$1" == -v ]]; then
      work=${2%%:*}
      break
    fi
    shift
  done
  id=${work:+$(cut -d: -f1 "$work/new-password")}
  printf '%s:$mock-hash\n' "$id" >"$work/new-password"
  exit 0
fi
if [[ "${IOTKIT_TEST_FENCE_RESTART_FAIL:-}" == once ]]; then
  if [[ ! -e "${IOTKIT_TEST_FENCE_RESTART_STATE:?}" ]]; then
    : >"$IOTKIT_TEST_FENCE_RESTART_STATE"
    exit 1
  fi
elif [[ "${IOTKIT_TEST_FENCE_RESTART_FAIL:-}" == 1 ]]; then
  exit 1
fi
exit 0
MOCK
chmod 700 "$root/bin/docker"

PATH="$root/bin:$PATH" "$repo_root/scripts/fence-edge-node.sh" \
  --edge-dir "$root/edge" \
  --edge-node-id node-01 \
  --output-directory "$root/output-parent/fenced"

jq -e '
  .schema_version == 1
  and .status == "fenced"
  and .edge_node_id == "node-01"
  and .credential_generation == 2
  and (.fence_id | test("^fence-[0-9a-f]{32}$"))
' "$root/output-parent/fenced/broker-fence-receipt.json" >/dev/null
[[ $(jq -r '."node-01"' "$root/edge/mosquitto/credential-generations.json") == 2 ]]
grep -Fx "node-01:\$mock-hash" "$root/edge/mosquitto/passwords" >/dev/null
[[ $(stat -c '%a' "$root/edge/mosquitto/passwords") == 640 ]]
[[ $(stat -c '%a' "$root/output-parent/fenced") == 700 ]]
[[ $(stat -c '%a' "$root/output-parent/fenced/mqtt-password") == 600 ]]

cp "$root/edge/mosquitto/passwords" "$root/passwords.before-failure"
cp "$root/edge/mosquitto/credential-generations.json" "$root/generations.before-failure"
if IOTKIT_TEST_FENCE_RESTART_FAIL=once \
  IOTKIT_TEST_FENCE_RESTART_STATE="$root/restart-once" PATH="$root/bin:$PATH" \
  "$repo_root/scripts/fence-edge-node.sh" \
  --edge-dir "$root/edge" \
  --edge-node-id node-01 \
  --output-directory "$root/output-parent/failed-once" \
  >"$root/failed-once.stdout" 2>"$root/failed-once.stderr"; then
  echo "single restart failure unexpectedly succeeded" >&2
  exit 1
fi
grep -Fq 'credential fence rolled back' "$root/failed-once.stderr"
cmp "$root/passwords.before-failure" "$root/edge/mosquitto/passwords"
cmp "$root/generations.before-failure" "$root/edge/mosquitto/credential-generations.json"
[[ ! -e "$root/output-parent/failed-once" ]]

if IOTKIT_TEST_FENCE_RESTART_FAIL=1 PATH="$root/bin:$PATH" \
  "$repo_root/scripts/fence-edge-node.sh" \
  --edge-dir "$root/edge" \
  --edge-node-id node-01 \
  --output-directory "$root/output-parent/failed" \
  >"$root/failed.stdout" 2>"$root/failed.stderr"; then
  echo "restart failure unexpectedly succeeded" >&2
  exit 1
fi
grep -Fq 'Broker runtime is unavailable or uncertain' "$root/failed.stderr"
cmp "$root/passwords.before-failure" "$root/edge/mosquitto/passwords"
cmp "$root/generations.before-failure" "$root/edge/mosquitto/credential-generations.json"
[[ $(stat -c '%a' "$root/edge/mosquitto/passwords") == 640 ]]
[[ ! -e "$root/output-parent/failed" ]]

rm "$root/edge/mosquitto/credential-generations.json"
if IOTKIT_TEST_FENCE_RESTART_FAIL=1 PATH="$root/bin:$PATH" \
  "$repo_root/scripts/fence-edge-node.sh" \
  --edge-dir "$root/edge" \
  --edge-node-id node-01 \
  --output-directory "$root/output-parent/failed-without-generation-file"; then
  echo "restart failure without generation state unexpectedly succeeded" >&2
  exit 1
fi
[[ ! -e "$root/edge/mosquitto/credential-generations.json" ]]
[[ $(stat -c '%a' "$root/edge/mosquitto/passwords") == 640 ]]
[[ ! -e "$root/output-parent/failed-without-generation-file" ]]

echo "Edge Node Broker credential fence tests passed."
