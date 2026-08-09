#!/usr/bin/env bash
set -euo pipefail
umask 077

repo_root=$(cd "$(dirname "$(realpath "${BASH_SOURCE[0]}")")/.." && pwd -P)
# shellcheck disable=SC1091
source "$repo_root/deploy/mosquitto-image.env"

binding=""
edge_dir=""
while (($#)); do
  case "$1" in
    --binding) binding=${2:-}; shift 2 ;;
    --edge-dir) edge_dir=${2:-}; shift 2 ;;
    *) echo "usage: add-edge-node.sh --binding FILE --edge-dir DIR" >&2; exit 2 ;;
  esac
done
[[ -f "$binding" && -d "$edge_dir" ]] || {
  echo "binding file and existing Edge directory are required" >&2
  exit 2
}
for command in jq docker flock openssl; do
  command -v "$command" >/dev/null || { echo "required command not found: $command" >&2; exit 1; }
done

jq -e '
  (keys | sort) == [
    "accepted_through_topic", "activation_request_topic", "activation_result_topic",
    "client_id", "descriptor_retain", "descriptor_topic", "edge_node_id", "qos",
    "records_topic", "recovery_completion_ack_topic", "recovery_completion_topic", "recovery_request_topic",
    "recovery_result_topic", "retain", "status_retain", "status_topic", "username"
  ]
  and (.edge_node_id | type == "string" and test("^[A-Za-z0-9._-]{1,128}$"))
  and .username == .edge_node_id
  and .client_id == ("iotkit-edge-node-" + .edge_node_id)
  and .records_topic == ("iotkit/v1/edge-nodes/" + .edge_node_id + "/records")
  and .status_topic == ("iotkit/v1/edge-nodes/" + .edge_node_id + "/status")
  and .accepted_through_topic == ("iotkit/v1/edge-nodes/" + .edge_node_id + "/accepted-through")
  and .descriptor_topic == ("iotkit/v1/edge-nodes/" + .edge_node_id + "/descriptors")
  and .activation_request_topic == ("iotkit/v1/edge-nodes/" + .edge_node_id + "/activation/request")
  and .activation_result_topic == ("iotkit/v1/edge-nodes/" + .edge_node_id + "/activation/result")
  and .recovery_request_topic == ("iotkit/v1/edge-nodes/" + .edge_node_id + "/recovery/request")
  and .recovery_result_topic == ("iotkit/v1/edge-nodes/" + .edge_node_id + "/recovery/result")
  and .recovery_completion_topic == ("iotkit/v1/edge-nodes/" + .edge_node_id + "/recovery/completion")
  and .recovery_completion_ack_topic == ("iotkit/v1/edge-nodes/" + .edge_node_id + "/recovery/completion-ack")
  and .qos == 1 and .retain == false and .descriptor_retain == true and .status_retain == true
' "$binding" >/dev/null || { echo "binding is not an exact IoTKit Edge Node MQTT binding" >&2; exit 1; }

edge_dir=$(realpath "$edge_dir")
acl="$edge_dir/mosquitto/acl"
passwords="$edge_dir/mosquitto/passwords"
edge_env="$edge_dir/edge.env"
handoff_root="$edge_dir/edge-handoff"
[[ -f "$acl" && -f "$passwords" && -f "$edge_env" ]] || {
  echo "Edge directory is incomplete" >&2
  exit 1
}
edge_node_id=$(jq -er '.edge_node_id' "$binding")
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
edge_status_rule='topic read iotkit/v1/edge-nodes/+/status'
if ! awk -v edge_user="user $edge_username" -v rule="$edge_status_rule" '
  $0 == edge_user { active=1; next }
  $1 == "user" { active=0 }
  active && $0 == rule { count++ }
  END { exit count == 1 ? 0 : 1 }
' "$acl"; then
  echo "Edge status ACL is not current; run scripts/upgrade-edge-node-recovery-acl.sh before enrolling a Node" >&2
  exit 1
fi

exec 9>"$edge_dir/.edge-enrollment.lock"
flock -x 9
if grep -Fxq "user $edge_node_id" "$acl"; then
  echo "Edge is already enrolled: $edge_node_id" >&2
  exit 1
fi
destination="$handoff_root/$edge_node_id"
[[ ! -e "$destination" ]] || { echo "handoff destination already exists" >&2; exit 1; }

stage=$(mktemp -d "$edge_dir/.edge-enrollment.XXXXXX")
cleanup() { rm -rf "$stage"; }
trap cleanup EXIT
cp "$acl" "$stage/acl"
cp "$passwords" "$stage/passwords"
mkdir -m 700 "$stage/handoff"
openssl rand -hex 24 >"$stage/handoff/mqtt-password"
cp "$edge_dir/tls/ca.pem" "$stage/handoff/broker-ca.pem"
cat >>"$stage/acl" <<EOF

user $edge_node_id
topic write iotkit/v1/edge-nodes/$edge_node_id/records
topic write iotkit/v1/edge-nodes/$edge_node_id/status
topic write iotkit/v1/edge-nodes/$edge_node_id/descriptors
topic write iotkit/v1/edge-nodes/$edge_node_id/activation/result
topic write iotkit/v1/edge-nodes/$edge_node_id/recovery/result
topic write iotkit/v1/edge-nodes/$edge_node_id/recovery/completion-ack
topic read iotkit/v1/edge-nodes/$edge_node_id/accepted-through
topic read iotkit/v1/edge-nodes/$edge_node_id/activation/request
topic read iotkit/v1/edge-nodes/$edge_node_id/recovery/request
topic read iotkit/v1/edge-nodes/$edge_node_id/recovery/completion
EOF
printf '%s:' "$edge_node_id" >"$stage/new-password"
tr -d '\r\n' <"$stage/handoff/mqtt-password" >>"$stage/new-password"
printf '\n' >>"$stage/new-password"
docker run --rm --user "$(id -u):$(id -g)" \
  -v "$stage:/work" "$IOTKIT_MOSQUITTO_IMAGE" \
  mosquitto_passwd -U /work/new-password >/dev/null
cat "$stage/new-password" >>"$stage/passwords"
rm "$stage/new-password"

broker_host=$(sed -n 's/^IOTKIT_BROKER_HOST=//p' "$edge_env")
broker_port=$(sed -n 's/^IOTKIT_BROKER_PORT=//p' "$edge_env")
cat >"$stage/handoff/edge-mqtt.toml" <<EOF
[exit.mqtt]
enabled = true
host = "$broker_host"
port = $broker_port
password_file = "/etc/iotkit/mqtt-password"
trust_mode = "bundle_only"
ca_file = "/etc/iotkit/broker-ca.pem"
EOF
cp "$binding" "$stage/handoff/edge-node-binding.json"
chmod 600 "$stage/acl" "$stage/passwords" "$stage/handoff"/*

cp "$acl" "$stage/acl.previous"
cp "$passwords" "$stage/passwords.previous"
mv "$stage/acl" "$acl"
mv "$stage/passwords" "$passwords"
if docker compose --env-file "$edge_env" -f "$repo_root/deploy/compose.edge.yaml" ps -q broker |
  grep -q .; then
  if ! docker compose --env-file "$edge_env" -f "$repo_root/deploy/compose.edge.yaml" \
    kill -s HUP broker >/dev/null; then
    mv "$stage/acl.previous" "$acl"
    mv "$stage/passwords.previous" "$passwords"
    echo "Broker reload failed; enrollment rolled back" >&2
    exit 1
  fi
fi
mv "$stage/handoff" "$destination"
echo "Edge enrolled. Protected handoff: $destination"
