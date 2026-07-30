#!/usr/bin/env bash
set -euo pipefail
umask 077

repo_root=$(cd "$(dirname "$(realpath "${BASH_SOURCE[0]}")")/.." && pwd -P)
root=$(mktemp -d)
cleanup() {
  rm -rf -- "$root"
}
trap cleanup EXIT

mkdir -m 700 "$root/bin" "$root/edge"
mkdir -m 700 "$root/edge/mosquitto"
cat >"$root/edge/edge.env" <<'EOF'
IOTKIT_EDGE_USERNAME=edge-archive
EOF
cat >"$root/edge/mosquitto/acl" <<'EOF'
user node-01
topic write iotkit/v1/edge-nodes/node-01/records
topic write iotkit/v1/edge-nodes/node-01/activation/result
topic read iotkit/v1/edge-nodes/node-01/accepted-through
topic read iotkit/v1/edge-nodes/node-01/activation/request

user edge-archive
topic read iotkit/v1/edge-nodes/+/records
topic read iotkit/v1/edge-nodes/+/activation/result
topic write iotkit/v1/edge-nodes/+/accepted-through
topic write iotkit/v1/edge-nodes/+/activation/request
EOF
chmod 640 "$root/edge/mosquitto/acl"

cat >"$root/bin/docker" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == run ]]; then
  sleep 10
fi
if [[ "${IOTKIT_TEST_ACL_RESTART_FAIL:-}" == once ]]; then
  if [[ ! -e "${IOTKIT_TEST_ACL_RESTART_STATE:?}" ]]; then
    : >"$IOTKIT_TEST_ACL_RESTART_STATE"
    exit 1
  fi
fi
exit 0
MOCK
chmod 700 "$root/bin/docker"

PATH="$root/bin:$PATH" "$repo_root/scripts/upgrade-edge-node-recovery-acl.sh" \
  --edge-dir "$root/edge" --edge-node-id node-01

required=(
  "topic write iotkit/v1/edge-nodes/node-01/recovery/result"
  "topic write iotkit/v1/edge-nodes/node-01/recovery/completion-ack"
  "topic read iotkit/v1/edge-nodes/node-01/recovery/request"
  "topic read iotkit/v1/edge-nodes/node-01/recovery/completion"
  "topic read iotkit/v1/edge-nodes/+/recovery/result"
  "topic read iotkit/v1/edge-nodes/+/recovery/completion-ack"
  "topic write iotkit/v1/edge-nodes/+/recovery/request"
  "topic write iotkit/v1/edge-nodes/+/recovery/completion"
)
for rule in "${required[@]}"; do
  [[ $(grep -Fxc "$rule" "$root/edge/mosquitto/acl") -eq 1 ]]
done
[[ $(stat -c '%a' "$root/edge/mosquitto/acl") == 640 ]]
grep -Fxq "IOTKIT_RECOVERY_DIR=$root/edge/recovery" "$root/edge/edge.env"
cp "$root/edge/mosquitto/acl" "$root/after-first"
cp "$root/edge/edge.env" "$root/edge-env.after-first"
PATH="$root/bin:$PATH" "$repo_root/scripts/upgrade-edge-node-recovery-acl.sh" \
  --edge-dir "$root/edge" --edge-node-id node-01
cmp "$root/after-first" "$root/edge/mosquitto/acl"
cmp "$root/edge-env.after-first" "$root/edge/edge.env"

sed -i '/recovery/d' "$root/edge/mosquitto/acl"
cp "$root/edge/mosquitto/acl" "$root/before-failure"
cp "$root/edge/edge.env" "$root/edge-env.before-failure"
if IOTKIT_TEST_ACL_RESTART_FAIL=once \
  IOTKIT_TEST_ACL_RESTART_STATE="$root/restart-once" PATH="$root/bin:$PATH" \
  "$repo_root/scripts/upgrade-edge-node-recovery-acl.sh" \
  --edge-dir "$root/edge" --edge-node-id node-01 \
  >"$root/failure.stdout" 2>"$root/failure.stderr"; then
  echo "ACL restart failure unexpectedly succeeded" >&2
  exit 1
fi
grep -Fq 'upgrade rolled back' "$root/failure.stderr"
cmp "$root/before-failure" "$root/edge/mosquitto/acl"
cmp "$root/edge-env.before-failure" "$root/edge/edge.env"

echo "Edge Node recovery ACL upgrade tests passed."
