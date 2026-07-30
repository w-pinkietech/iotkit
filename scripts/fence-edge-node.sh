#!/usr/bin/env bash
set -euo pipefail
umask 077

repo_root=$(cd "$(dirname "$(realpath "${BASH_SOURCE[0]}")")/.." && pwd -P)
# shellcheck disable=SC1091
source "$repo_root/deploy/mosquitto-image.env"

edge_dir=""
edge_node_id=""
output_directory=""
while (($#)); do
  case "$1" in
    --edge-dir) edge_dir=${2:-}; shift 2 ;;
    --edge-node-id) edge_node_id=${2:-}; shift 2 ;;
    --output-directory) output_directory=${2:-}; shift 2 ;;
    *) echo "usage: fence-edge-node.sh --edge-dir DIR --edge-node-id ID --output-directory ABSENT_DIR" >&2; exit 2 ;;
  esac
done

for command in docker flock jq openssl realpath; do
  command -v "$command" >/dev/null || {
    echo "required command not found: $command" >&2
    exit 1
  }
done
[[ "$edge_node_id" =~ ^[A-Za-z0-9._-]{1,128}$ ]] || {
  echo "unsafe Edge Node identity" >&2
  exit 2
}
[[ -d "$edge_dir" && "$output_directory" = /* && ! -e "$output_directory" ]] || {
  echo "existing Edge directory and absent absolute output directory are required" >&2
  exit 2
}

edge_dir=$(realpath "$edge_dir")
output_parent=$(realpath "$(dirname "$output_directory")")
output_directory="$output_parent/$(basename "$output_directory")"
if [[ $(stat -c '%u' "$output_parent") -ne $(id -u) ]] \
  || (( (8#$(stat -c '%a' "$output_parent") & 8#077) != 0 )); then
  echo "output parent must be owner-only" >&2
  exit 2
fi

passwords="$edge_dir/mosquitto/passwords"
acl="$edge_dir/mosquitto/acl"
edge_env="$edge_dir/edge.env"
generations="$edge_dir/mosquitto/credential-generations.json"
[[ -f "$passwords" && -f "$acl" && -f "$edge_env" ]] || {
  echo "Edge directory is incomplete" >&2
  exit 1
}
grep -q "^${edge_node_id}:" "$passwords" || {
  echo "Edge Node is not enrolled" >&2
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
required_acl=(
  "topic write iotkit/v1/edge-nodes/$edge_node_id/recovery/result"
  "topic write iotkit/v1/edge-nodes/$edge_node_id/recovery/completion-ack"
  "topic read iotkit/v1/edge-nodes/$edge_node_id/recovery/request"
  "topic read iotkit/v1/edge-nodes/$edge_node_id/recovery/completion"
  "topic read iotkit/v1/edge-nodes/+/recovery/result"
  "topic read iotkit/v1/edge-nodes/+/recovery/completion-ack"
  "topic write iotkit/v1/edge-nodes/+/recovery/request"
  "topic write iotkit/v1/edge-nodes/+/recovery/completion"
)
[[ "$edge_username" =~ ^[A-Za-z0-9._-]{1,255}$ \
   && $(grep -Fxc "user $edge_node_id" "$acl") -eq 1 \
   && $(grep -Fxc "user $edge_username" "$acl") -eq 1 ]] || {
  echo "Recovery ACL principals are missing or ambiguous" >&2
  exit 1
}
for rule in "${required_acl[@]}"; do
  [[ $(grep -Fxc "$rule" "$acl") -eq 1 ]] || {
    echo "Recovery ACL is not current; run upgrade-edge-node-recovery-acl.sh before fencing" >&2
    exit 1
  }
done

exec 9>"$edge_dir/.edge-enrollment.lock"
flock -x 9
[[ ! -e "$output_directory" ]] || {
  echo "recovery credential output already exists" >&2
  exit 1
}

stage=$(mktemp -d "$output_parent/.iotkit-node-fence.XXXXXX")
work=$(mktemp -d "$edge_dir/.iotkit-node-fence.XXXXXX")
preserve_stage=false
cleanup() {
  if [[ "$preserve_stage" != true ]]; then
    rm -rf -- "$stage"
  fi
  rm -rf -- "$work"
}
trap cleanup EXIT

generations_existed=false
if [[ -e "$generations" ]]; then
  generations_existed=true
  jq -e 'type == "object" and all(.[]; type == "number" and floor == . and . >= 1)' \
    "$generations" >/dev/null || {
    echo "credential generation state is invalid" >&2
    exit 1
  }
  current_generation=$(jq -r --arg id "$edge_node_id" '.[$id] // 1' "$generations")
else
  current_generation=1
  printf '{}\n' >"$work/generations.previous"
fi
new_generation=$((current_generation + 1))
fence_id="fence-$(openssl rand -hex 16)"
fenced_at=$(date +%s%3N)

mkdir -m 700 "$stage/payload"
openssl rand -hex 24 >"$stage/payload/mqtt-password"
printf '%s:' "$edge_node_id" >"$work/new-password"
tr -d '\r\n' <"$stage/payload/mqtt-password" >>"$work/new-password"
printf '\n' >>"$work/new-password"
docker run --rm --user "$(id -u):$(id -g)" \
  -v "$work:/work" "$IOTKIT_MOSQUITTO_IMAGE" \
  mosquitto_passwd -U /work/new-password >/dev/null

awk -F: -v id="$edge_node_id" '
  NR == FNR { replacement=$0; next }
  $1 == id { print replacement; replaced=1; next }
  { print }
  END { if (!replaced) exit 1 }
' "$work/new-password" "$passwords" >"$work/passwords.next"

if [[ -e "$generations" ]]; then
  cp --preserve=mode,ownership "$generations" "$work/generations.previous"
fi
jq --arg id "$edge_node_id" --argjson generation "$new_generation" \
  '.[$id] = $generation' "$work/generations.previous" >"$work/generations.next"
jq -n \
  --arg fence_id "$fence_id" \
  --arg edge_node_id "$edge_node_id" \
  --argjson credential_generation "$new_generation" \
  --argjson fenced_at "$fenced_at" \
  '{
    schema_version: 1,
    status: "fenced",
    fence_id: $fence_id,
    edge_node_id: $edge_node_id,
    credential_generation: $credential_generation,
    fenced_at: $fenced_at
  }' >"$stage/payload/broker-fence-receipt.json"
chmod 600 "$stage/payload"/*

cp --preserve=mode,ownership "$passwords" "$work/passwords.previous"
chown --reference="$passwords" "$work/passwords.next"
chmod --reference="$passwords" "$work/passwords.next"
if [[ "$generations_existed" == true ]]; then
  chown --reference="$generations" "$work/generations.next"
  chmod --reference="$generations" "$work/generations.next"
else
  chown --reference="$passwords" "$work/generations.next"
  chmod 600 "$work/generations.next"
fi
mv "$work/passwords.next" "$passwords"
mv "$work/generations.next" "$generations"

sync_broker_state() {
  sync "$passwords" "$generations" &&
    sync "$(dirname "$passwords")"
}

rollback_broker_state() {
  mv "$work/passwords.previous" "$passwords"
  if [[ "$generations_existed" == true ]]; then
    mv "$work/generations.previous" "$generations"
  else
    rm -f -- "$generations"
  fi
  if [[ -e "$generations" ]]; then
    sync_broker_state
  else
    sync "$passwords" "$(dirname "$passwords")"
  fi
}

if ! sync_broker_state; then
  if rollback_broker_state; then
    echo "Broker credential state could not be durably synchronized; fence rolled back" >&2
  else
    echo "Broker credential state and rollback durability are uncertain" >&2
  fi
  exit 1
fi

if ! docker compose --env-file "$edge_env" -f "$repo_root/deploy/compose.edge.yaml" \
  restart broker >/dev/null; then
  rollback_broker_state || {
    echo "Broker restart failed and credential rollback durability is uncertain" >&2
    exit 1
  }
  if ! docker compose --env-file "$edge_env" -f "$repo_root/deploy/compose.edge.yaml" \
    restart broker >/dev/null; then
    echo "Broker credential disk state was durably rolled back, but Broker runtime is unavailable or uncertain" >&2
    exit 1
  fi
  echo "Broker restart failed; credential fence rolled back" >&2
  exit 1
fi

sync "$stage/payload/mqtt-password" \
  "$stage/payload/broker-fence-receipt.json" "$stage/payload"
if ! mv "$stage/payload" "$output_directory"; then
  preserve_stage=true
  echo "Broker fence succeeded, but credential publication failed." >&2
  echo "Protected recovery payload remains at: $stage/payload" >&2
  exit 1
fi
chmod 700 "$output_directory"
sync "$output_directory" "$output_parent"
echo "Edge Node credential fenced. Protected output: $output_directory"
