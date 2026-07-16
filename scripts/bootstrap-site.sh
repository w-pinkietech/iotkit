#!/usr/bin/env bash
set -euo pipefail

umask 077

script_path=$(realpath "${BASH_SOURCE[0]}")
repo_root=$(cd "$(dirname "$script_path")/.." && pwd -P)
# shellcheck disable=SC1091
source "$repo_root/deploy/mosquitto-image.env"
binding=""
output_dir=""
broker_host=""
broker_bind=""
broker_port="8883"
tls_cert=""
tls_key=""
tls_ca=""
site_publish_topics=()

usage() {
  cat >&2 <<'EOF'
usage: bootstrap-site.sh \
  --binding EDGE_MQTT_BINDING.json \
  --output-dir /path/outside/repository \
  --broker-host mqtt.example.internal \
  --broker-bind 192.0.2.10 \
  [--broker-port 8883] \
  --tls-cert server-fullchain.pem \
  --tls-key server.key \
  --tls-ca client-trust-ca.pem \
  [--site-publish-topic exact/application/topic ...]
EOF
}

fail() {
  echo "$1" >&2
  exit 1
}

while (($#)); do
  case "$1" in
    --binding|--output-dir|--broker-host|--broker-bind|--broker-port|--tls-cert|--tls-key|--tls-ca|--site-publish-topic)
      (($# >= 2)) || { usage; exit 2; }
      case "$1" in
        --binding) binding=$2 ;;
        --output-dir) output_dir=$2 ;;
        --broker-host) broker_host=$2 ;;
        --broker-bind) broker_bind=$2 ;;
        --broker-port) broker_port=$2 ;;
        --tls-cert) tls_cert=$2 ;;
        --tls-key) tls_key=$2 ;;
        --tls-ca) tls_ca=$2 ;;
        --site-publish-topic) site_publish_topics+=("$2") ;;
      esac
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      fail "unknown argument: $1"
      ;;
  esac
done

for required in binding output_dir broker_host broker_bind tls_cert tls_key tls_ca; do
  [[ -n "${!required}" ]] || { usage; fail "--${required//_/-} is required"; }
done

for command in jq openssl docker realpath stat getent; do
  command -v "$command" >/dev/null || fail "required command not found: $command"
done
docker compose version >/dev/null 2>&1 || fail "Docker Compose is required"

[[ -f "$binding" ]] || fail "binding file does not exist: $binding"
[[ -f "$tls_cert" ]] || fail "TLS certificate file does not exist: $tls_cert"
[[ -f "$tls_key" ]] || fail "TLS private key file does not exist: $tls_key"
[[ -f "$tls_ca" ]] || fail "TLS CA file does not exist: $tls_ca"

binding=$(realpath "$binding")
tls_cert=$(realpath "$tls_cert")
tls_key=$(realpath "$tls_key")
tls_ca=$(realpath "$tls_ca")
output_parent=$(realpath -m "$(dirname "$output_dir")")
[[ -d "$output_parent" ]] || fail "output parent directory does not exist: $output_parent"
output_dir="$output_parent/$(basename "$output_dir")"
[[ "$output_dir" =~ ^/[A-Za-z0-9._/-]+$ ]] \
  || fail "output directory path contains unsupported characters"
case "$output_dir" in
  "$repo_root"|"$repo_root"/*) fail "output directory must be outside the Git repository" ;;
esac
[[ ! -e "$output_dir" ]] || fail "output directory already exists: $output_dir"

key_mode=$(stat -c %a "$tls_key")
if (( (8#$key_mode & 077) != 0 )); then
  fail "TLS private key must not be group/world accessible"
fi

validate_hostname() {
  local hostname=$1 label
  [[ ${#hostname} -le 253 && "$hostname" != *..* ]] || return 1
  IFS='.' read -r -a labels <<<"$hostname"
  for label in "${labels[@]}"; do
    [[ "$label" =~ ^[A-Za-z0-9]([A-Za-z0-9-]*[A-Za-z0-9])?$ ]] || return 1
  done
}

validate_ipv4() {
  local address=$1 octet
  local -a octets
  IFS='.' read -r -a octets <<<"$address"
  [[ ${#octets[@]} -eq 4 ]] || return 1
  for octet in "${octets[@]}"; do
    [[ "$octet" =~ ^[0-9]{1,3}$ ]] || return 1
    ((10#$octet <= 255)) || return 1
  done
}

validate_hostname "$broker_host" || fail "broker host must be a valid DNS hostname"
validate_ipv4 "$broker_bind" || fail "broker bind must be an explicit IPv4 address"
[[ "$broker_port" =~ ^[0-9]+$ ]] && ((broker_port >= 1 && broker_port <= 65535)) \
  || fail "broker port must be between 1 and 65535"
getent ahostsv4 "$broker_host" | awk '{print $1}' | grep -Fxq "$broker_bind" \
  || fail "broker host must resolve to the configured bind address on the Site host"

jq -e '
  (keys | sort) == [
    "accepted_through_topic", "client_id", "descriptor_retain", "descriptor_topic",
    "edge_node_id", "qos", "records_topic", "retain", "username"
  ]
  and (.edge_node_id | type == "string" and length > 0)
  and .username == .edge_node_id
  and .client_id == ("iotkit-edge-" + .edge_node_id)
  and .records_topic == ("iotkit/v1/edge-nodes/" + .edge_node_id + "/records")
  and .accepted_through_topic == ("iotkit/v1/edge-nodes/" + .edge_node_id + "/accepted-through")
  and .descriptor_topic == ("iotkit/v1/edge-nodes/" + .edge_node_id + "/descriptors")
  and .qos == 1
  and .retain == false
  and .descriptor_retain == true
' "$binding" >/dev/null || fail "binding file is not an exact iotkit-edgectl mqtt-binding document"
edge_node_id=$(jq -er '.edge_node_id' "$binding")
[[ "$edge_node_id" =~ ^[A-Za-z0-9._-]{1,128}$ ]] \
  || fail "edge_node_id is not safe for generated ACL/config files"

for topic in "${site_publish_topics[@]}"; do
  [[ -n "$topic" && "$topic" != /* && "$topic" != */ && "$topic" != *['+#']* \
    && "$topic" != *[[:space:]]* ]] || fail "invalid Site publish topic: $topic"
  [[ "$topic" != iotkit/v1/edge-nodes/* ]] \
    || fail "Site application topic must not overlap the Edge custody namespace"
done

openssl x509 -in "$tls_cert" -noout >/dev/null 2>&1 \
  || fail "TLS certificate is not a PEM X.509 certificate"
openssl pkey -in "$tls_key" -noout >/dev/null 2>&1 \
  || fail "TLS private key is not readable"
openssl x509 -in "$tls_ca" -noout >/dev/null 2>&1 \
  || fail "TLS CA file contains no leading PEM X.509 certificate"
openssl verify -CAfile "$tls_ca" "$tls_cert" >/dev/null 2>&1 \
  || fail "TLS certificate does not verify against the supplied CA file"
openssl x509 -in "$tls_cert" -noout -checkhost "$broker_host" >/dev/null 2>&1 \
  || fail "TLS certificate does not cover broker host $broker_host"
openssl x509 -in "$tls_cert" -noout -checkend 86400 >/dev/null 2>&1 \
  || fail "TLS certificate expires within 24 hours"
cert_public_key=$(openssl x509 -in "$tls_cert" -pubkey -noout \
  | openssl pkey -pubin -outform DER 2>/dev/null | openssl dgst -sha256)
private_public_key=$(openssl pkey -in "$tls_key" -pubout -outform DER 2>/dev/null \
  | openssl dgst -sha256)
[[ "$cert_public_key" == "$private_public_key" ]] \
  || fail "TLS certificate and private key do not match"

stage="$output_parent/.$(basename "$output_dir").tmp.$$"
[[ ! -e "$stage" ]] || fail "temporary output path already exists: $stage"
committed=false
cleanup() {
  if [[ "$committed" != true ]]; then
    rm -rf "$stage"
  fi
}
trap cleanup EXIT

mkdir -m 700 "$stage"
mkdir -m 700 "$stage/mosquitto" "$stage/secrets" "$stage/tls" \
  "$stage/edge-handoff" "$stage/data" "$stage/data/site" "$stage/data/mosquitto"
cp "$binding" "$stage/edge-binding.json"
cp "$tls_cert" "$stage/tls/server.pem"
cp "$tls_key" "$stage/tls/server.key"
cp "$tls_ca" "$stage/tls/ca.pem"
cp "$tls_ca" "$stage/edge-handoff/broker-ca.pem"
openssl rand -hex 24 >"$stage/secrets/site-mqtt-password"
openssl rand -hex 24 >"$stage/edge-handoff/mqtt-password"

cat >"$stage/mosquitto/mosquitto.conf" <<'EOF'
listener 8883 0.0.0.0
protocol mqtt
allow_anonymous false
password_file /mosquitto/config/passwords
acl_file /mosquitto/config/acl
certfile /mosquitto/config/tls/server.pem
keyfile /mosquitto/config/tls/server.key
tls_version tlsv1.2
require_certificate false
message_size_limit 1048576
max_packet_size 1114112
max_inflight_messages 20
max_queued_messages 1000
max_connections 128
memory_limit 268435456
persistence true
persistence_location /mosquitto/data/
log_dest stdout
EOF

cat >"$stage/mosquitto/acl" <<EOF
user $edge_node_id
topic write iotkit/v1/edge-nodes/$edge_node_id/records
topic write iotkit/v1/edge-nodes/$edge_node_id/descriptors
topic read iotkit/v1/edge-nodes/$edge_node_id/accepted-through

user site
topic read iotkit/v1/edge-nodes/+/records
topic read iotkit/v1/edge-nodes/+/descriptors
topic write iotkit/v1/edge-nodes/+/accepted-through
EOF
for topic in "${site_publish_topics[@]}"; do
  printf 'topic write %s\n' "$topic" >>"$stage/mosquitto/acl"
done

printf 'site:' >"$stage/mosquitto/passwords"
tr -d '\r\n' <"$stage/secrets/site-mqtt-password" >>"$stage/mosquitto/passwords"
printf '\n%s:' "$edge_node_id" >>"$stage/mosquitto/passwords"
tr -d '\r\n' <"$stage/edge-handoff/mqtt-password" >>"$stage/mosquitto/passwords"
printf '\n' >>"$stage/mosquitto/passwords"
docker run --rm --user "$(id -u):$(id -g)" \
  -v "$stage/mosquitto:/work" "$IOTKIT_MOSQUITTO_IMAGE" \
  mosquitto_passwd -U /work/passwords >/dev/null

cat >"$stage/edge-handoff/edge-mqtt.toml" <<EOF

[exit.mqtt]
enabled = true
host = "$broker_host"
port = $broker_port
password_file = "/etc/iotkit/mqtt-password"
trust_mode = "bundle_only"
ca_file = "/etc/iotkit/broker-ca.pem"
EOF

cat >"$stage/site.env" <<EOF
IOTKIT_RUNTIME_UID=$(id -u)
IOTKIT_RUNTIME_GID=$(id -g)
IOTKIT_MOSQUITTO_IMAGE=$IOTKIT_MOSQUITTO_IMAGE
IOTKIT_BROKER_HOST=$broker_host
IOTKIT_BROKER_BIND=$broker_bind
IOTKIT_BROKER_PORT=$broker_port
IOTKIT_MOSQUITTO_CONFIG_FILE=$output_dir/mosquitto/mosquitto.conf
IOTKIT_MOSQUITTO_ACL_FILE=$output_dir/mosquitto/acl
IOTKIT_MOSQUITTO_PASSWORD_FILE=$output_dir/mosquitto/passwords
IOTKIT_BROKER_CERT_FILE=$output_dir/tls/server.pem
IOTKIT_BROKER_KEY_FILE=$output_dir/tls/server.key
IOTKIT_BROKER_CA_FILE=$output_dir/tls/ca.pem
IOTKIT_BROKER_DATA_DIR=$output_dir/data/mosquitto
IOTKIT_SITE_PASSWORD_FILE=$output_dir/secrets/site-mqtt-password
IOTKIT_SITE_DATA_DIR=$output_dir/data/site
EOF

find "$stage" -type d -exec chmod 700 {} +
find "$stage" -type f -exec chmod 600 {} +
mv "$stage" "$output_dir"
committed=true
trap - EXIT

echo "IoTKit Site bootstrap created: $output_dir"
echo "Start Site: docker compose --env-file $output_dir/site.env -f $repo_root/deploy/compose.site.yaml up --build --detach"
echo "Edge handoff directory (contains a plaintext credential): $output_dir/edge-handoff"
