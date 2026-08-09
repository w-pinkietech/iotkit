#!/usr/bin/env bash
set -euo pipefail
umask 077

repo_root=$(cd "$(dirname "$(realpath "${BASH_SOURCE[0]}")")/.." && pwd -P)
# shellcheck disable=SC1091
source "$repo_root/deploy/mosquitto-image.env"

edge_dir=""
edge_node_id=""
while (($#)); do
  case "$1" in
    --edge-dir) edge_dir=${2:-}; shift 2 ;;
    --edge-node-id) edge_node_id=${2:-}; shift 2 ;;
    *) echo "usage: upgrade-edge-node-recovery-acl.sh --edge-dir DIR --edge-node-id ID" >&2; exit 2 ;;
  esac
done

for command in docker flock realpath stat timeout; do
  command -v "$command" >/dev/null || {
    echo "required command not found: $command" >&2
    exit 1
  }
done
[[ "$edge_node_id" =~ ^[A-Za-z0-9._-]{1,128}$ && -d "$edge_dir" ]] || {
  echo "safe Edge Node identity and existing Edge directory are required" >&2
  exit 2
}
edge_dir=$(realpath "$edge_dir")
acl="$edge_dir/mosquitto/acl"
edge_env="$edge_dir/edge.env"
[[ -f "$acl" && -f "$edge_env" ]] || {
  echo "Edge ACL deployment is incomplete" >&2
  exit 1
}
edge_username=$(awk -F= '
  $1 == "IOTKIT_EDGE_USERNAME" {
    if (++found != 1 || $2 == "") exit 2
    value=substr($0, index($0, "=") + 1)
  }
  END {
    if (found != 1) exit 2
    print value
  }
' "$edge_env") || {
  echo "Edge archive principal is invalid" >&2
  exit 1
}
[[ "$edge_username" =~ ^[A-Za-z0-9._-]{1,255}$ ]] || {
  echo "Edge archive principal is unsafe" >&2
  exit 1
}
[[ $(grep -Fxc "user $edge_node_id" "$acl") -eq 1 \
   && $(grep -Fxc "user $edge_username" "$acl") -eq 1 ]] || {
  echo "Edge Node or Edge archive ACL principal is missing or ambiguous" >&2
  exit 1
}

node_result="topic write iotkit/v1/edge-nodes/$edge_node_id/recovery/result"
node_ack="topic write iotkit/v1/edge-nodes/$edge_node_id/recovery/completion-ack"
node_request="topic read iotkit/v1/edge-nodes/$edge_node_id/recovery/request"
node_completion="topic read iotkit/v1/edge-nodes/$edge_node_id/recovery/completion"
node_status="topic write iotkit/v1/edge-nodes/$edge_node_id/status"
edge_result="topic read iotkit/v1/edge-nodes/+/recovery/result"
edge_ack="topic read iotkit/v1/edge-nodes/+/recovery/completion-ack"
edge_request="topic write iotkit/v1/edge-nodes/+/recovery/request"
edge_completion="topic write iotkit/v1/edge-nodes/+/recovery/completion"
edge_status="topic read iotkit/v1/edge-nodes/+/status"
required=(
  "$node_result" "$node_ack" "$node_request" "$node_completion" "$node_status"
  "$edge_result" "$edge_ack" "$edge_request" "$edge_completion" "$edge_status"
)
recovery_directory="$edge_dir/recovery"
if [[ ! -e "$recovery_directory" ]]; then
  mkdir -m 700 "$recovery_directory"
fi
[[ -d "$recovery_directory" \
   && $(stat -c '%u' "$recovery_directory") -eq $(id -u) \
   && $((8#$(stat -c '%a' "$recovery_directory") & 8#077)) -eq 0 ]] || {
  echo "recovery directory must be owner-only" >&2
  exit 1
}

complete=true
for rule in "${required[@]}"; do
  if [[ $(grep -Fxc "$rule" "$acl") -ne 1 ]]; then
    complete=false
  fi
done
recovery_env="IOTKIT_RECOVERY_DIR=$recovery_directory"
recovery_env_count=$(grep -c '^IOTKIT_RECOVERY_DIR=' "$edge_env" || true)
if [[ $recovery_env_count -gt 1 ]] \
  || { [[ $recovery_env_count -eq 1 ]] && ! grep -Fxq "$recovery_env" "$edge_env"; }; then
  echo "IOTKIT_RECOVERY_DIR is ambiguous or does not match this deployment" >&2
  exit 1
fi
env_current=false
[[ $recovery_env_count -eq 1 ]] && env_current=true
if [[ "$complete" == true && "$env_current" == true ]]; then
  echo "Recovery ACL is already current."
  exit 0
fi

exec 9>"$edge_dir/.edge-enrollment.lock"
flock -x 9
work=$(mktemp -d "$edge_dir/.iotkit-recovery-acl.XXXXXX")
cleanup() {
  rm -rf -- "$work"
}
trap cleanup EXIT

cp --preserve=mode,ownership "$acl" "$work/acl.previous"
cp --preserve=mode,ownership "$edge_env" "$work/edge.env.previous"
awk \
  -v node_user="user $edge_node_id" \
  -v edge_user="user $edge_username" \
  -v node_result="$node_result" -v node_ack="$node_ack" \
  -v node_request="$node_request" -v node_completion="$node_completion" -v node_status="$node_status" \
  -v edge_result="$edge_result" -v edge_ack="$edge_ack" \
  -v edge_request="$edge_request" -v edge_completion="$edge_completion" -v edge_status="$edge_status" '
  NR == FNR { present[$0]=1; next }
  {
    print
    if ($0 == node_user) {
      if (!present[node_result]) print node_result
      if (!present[node_ack]) print node_ack
      if (!present[node_request]) print node_request
      if (!present[node_completion]) print node_completion
      if (!present[node_status]) print node_status
    } else if ($0 == edge_user) {
      if (!present[edge_result]) print edge_result
      if (!present[edge_ack]) print edge_ack
      if (!present[edge_request]) print edge_request
      if (!present[edge_completion]) print edge_completion
      if (!present[edge_status]) print edge_status
    }
  }
' "$acl" "$acl" >"$work/acl.next"

for rule in "${required[@]}"; do
  [[ $(grep -Fxc "$rule" "$work/acl.next") -eq 1 ]] || {
    echo "generated recovery ACL is incomplete or ambiguous" >&2
    exit 1
  }
done
cp --preserve=mode,ownership "$edge_env" "$work/edge.env.next"
if [[ "$env_current" == false ]]; then
  printf '%s\n' "$recovery_env" >>"$work/edge.env.next"
fi
cat >"$work/validate.conf" <<'EOF'
listener 1883 127.0.0.1
allow_anonymous true
acl_file /work/acl.next
persistence false
EOF
set +e
timeout 3 docker run --rm --user "$(id -u):$(id -g)" \
  -v "$work:/work:ro" "$IOTKIT_MOSQUITTO_IMAGE" \
  mosquitto -c /work/validate.conf >/dev/null 2>&1
validation_status=$?
set -e
[[ $validation_status -eq 124 ]] || {
  echo "generated recovery ACL failed Mosquitto validation" >&2
  exit 1
}

chown --reference="$acl" "$work/acl.next"
chmod --reference="$acl" "$work/acl.next"
chown --reference="$edge_env" "$work/edge.env.next"
chmod --reference="$edge_env" "$work/edge.env.next"
mv "$work/acl.next" "$acl"
mv "$work/edge.env.next" "$edge_env"
sync "$acl" "$edge_env" "$(dirname "$acl")" "$(dirname "$edge_env")"
if ! IOTKIT_RECOVERY_DIR="$recovery_directory" docker compose --env-file "$edge_env" \
  -f "$repo_root/deploy/compose.edge.yaml" restart broker >/dev/null; then
  mv "$work/acl.previous" "$acl"
  mv "$work/edge.env.previous" "$edge_env"
  sync "$acl" "$edge_env" "$(dirname "$acl")" "$(dirname "$edge_env")"
  if ! IOTKIT_RECOVERY_DIR="$recovery_directory" docker compose --env-file "$edge_env" \
    -f "$repo_root/deploy/compose.edge.yaml" restart broker >/dev/null; then
    echo "Recovery ACL was rolled back on disk, but Broker runtime is unavailable or uncertain" >&2
    exit 1
  fi
  echo "Broker restart failed; recovery ACL upgrade rolled back" >&2
  exit 1
fi
echo "Recovery ACL upgrade completed."
