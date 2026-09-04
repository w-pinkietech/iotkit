#!/usr/bin/env bash
set -euo pipefail

umask 077

script_path=$(realpath "${BASH_SOURCE[0]}")
repo_root=$(cd "$(dirname "$script_path")/.." && pwd -P)
# shellcheck disable=SC1091
source "$repo_root/scripts/lib/certificate-hostname.sh"
# shellcheck disable=SC1091
source "$repo_root/deploy/mosquitto-image.env"
binding=""
output_dir=""
broker_host=""
broker_bind=""
broker_port="8883"
edge_https_port="443"
storage_profile="embedded"
postgres_port="5432"
tls_cert=""
tls_key=""
tls_ca=""
edge_publish_topics=()

usage() {
  cat >&2 <<'EOF'
usage: bootstrap-edge.sh \
  --binding EDGE_MQTT_BINDING.json \
  --output-dir /path/outside/repository \
  --broker-host mqtt.example.internal \
  --broker-bind 192.0.2.10 \
  [--broker-port 8883] \
  [--edge-https-port 443] \
	[--storage-profile embedded|postgres] \
	[--postgres-port 5432] \
  --tls-cert server-fullchain.pem \
  --tls-key server.key \
  --tls-ca client-trust-ca.pem \
  [--edge-publish-topic exact/application/topic ...]
EOF
}

fail() {
  echo "$1" >&2
  exit 1
}

while (($#)); do
  case "$1" in
    --binding|--output-dir|--broker-host|--broker-bind|--broker-port|--edge-https-port|--storage-profile|--postgres-port|--tls-cert|--tls-key|--tls-ca|--edge-publish-topic)
      (($# >= 2)) || { usage; exit 2; }
      case "$1" in
        --binding) binding=$2 ;;
        --output-dir) output_dir=$2 ;;
        --broker-host) broker_host=$2 ;;
        --broker-bind) broker_bind=$2 ;;
        --broker-port) broker_port=$2 ;;
        --edge-https-port) edge_https_port=$2 ;;
		--storage-profile) storage_profile=$2 ;;
		--postgres-port) postgres_port=$2 ;;
        --tls-cert) tls_cert=$2 ;;
        --tls-key) tls_key=$2 ;;
        --tls-ca) tls_ca=$2 ;;
        --edge-publish-topic) edge_publish_topics+=("$2") ;;
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
[[ "$edge_https_port" =~ ^[0-9]+$ ]] && ((edge_https_port >= 1 && edge_https_port <= 65535)) \
  || fail "Edge HTTPS port must be between 1 and 65535"
[[ "$storage_profile" == "embedded" || "$storage_profile" == "postgres" ]] \
  || fail "storage profile must be embedded or postgres"
[[ "$postgres_port" =~ ^[0-9]+$ ]] && ((postgres_port >= 1 && postgres_port <= 65535)) \
  || fail "PostgreSQL port must be between 1 and 65535"
getent ahostsv4 "$broker_host" | awk '{print $1}' | grep -Fxq "$broker_bind" \
  || fail "broker host must resolve to the configured bind address on the Edge host"

jq -e '
  (keys | sort) == [
    "accepted_through_topic", "activation_request_topic", "activation_result_topic",
    "client_id", "descriptor_retain", "descriptor_topic", "edge_node_id", "qos",
    "records_topic", "recovery_completion_ack_topic", "recovery_completion_topic", "recovery_request_topic",
    "recovery_result_topic", "retain", "status_retain", "status_topic", "username"
  ]
  and (.edge_node_id | type == "string" and length > 0)
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
  and .qos == 1
  and .retain == false
  and .descriptor_retain == true
  and .status_retain == true
' "$binding" >/dev/null || fail "binding file is not an exact iotkit-edge-nodectl mqtt-binding document"
edge_node_id=$(jq -er '.edge_node_id' "$binding")
[[ "$edge_node_id" =~ ^[A-Za-z0-9._-]{1,128}$ ]] \
  || fail "edge_node_id is not safe for generated ACL/config files"
edge_id="edge-$(openssl rand -hex 16)"
edge_archive_principal="iotkit-edge-archive-$edge_id"
edge_output_principal="iotkit-edge-output-$edge_id"

for topic in "${edge_publish_topics[@]}"; do
  [[ -n "$topic" && "$topic" != /* && "$topic" != */ && "$topic" != *['+#']* \
    && "$topic" != *[[:space:]]* ]] || fail "invalid Edge publish topic: $topic"
  [[ "$topic" != iotkit/v1/edge-nodes/* ]] \
    || fail "Edge application topic must not overlap the Edge custody namespace"
done

openssl x509 -in "$tls_cert" -noout >/dev/null 2>&1 \
  || fail "TLS certificate is not a PEM X.509 certificate"
openssl pkey -in "$tls_key" -noout >/dev/null 2>&1 \
  || fail "TLS private key is not readable"
openssl x509 -in "$tls_ca" -noout >/dev/null 2>&1 \
  || fail "TLS CA file contains no leading PEM X.509 certificate"
openssl verify -CAfile "$tls_ca" "$tls_cert" >/dev/null 2>&1 \
  || fail "TLS certificate does not verify against the supplied CA file"
certificate_covers_hostname "$tls_cert" "$broker_host" \
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
mkdir -m 700 "$stage/mosquitto" "$stage/mosquitto/tls" "$stage/caddy" \
  "$stage/systemd" "$stage/secrets" "$stage/tls" \
  "$stage/edge-handoff" "$stage/recovery" "$stage/data" "$stage/data/edge" "$stage/data/mosquitto"
mkdir -m 700 "$stage/data/caddy"
mkdir -m 755 "$stage/data/acme-webroot"
cp "$binding" "$stage/edge-node-binding.json"
cp "$tls_cert" "$stage/tls/server.pem"
cp "$tls_key" "$stage/tls/server.key"
cp "$tls_ca" "$stage/tls/ca.pem"
cp "$tls_ca" "$stage/edge-handoff/broker-ca.pem"
openssl rand -hex 24 >"$stage/secrets/edge-archive-mqtt-password"
openssl rand -hex 24 >"$stage/edge-handoff/mqtt-password"
printf '{"profile":"%s"}\n' "$storage_profile" >"$stage/storage-profile.json"
if [[ "$storage_profile" == "postgres" ]]; then
  openssl rand -hex 24 >"$stage/secrets/postgres-password"
  postgres_password=$(<"$stage/secrets/postgres-password")
  printf '{"dsn":"postgres://iotkit:%s@127.0.0.1:%s/iotkit?sslmode=disable"}\n' \
    "$postgres_password" "$postgres_port" >"$stage/secrets/postgres.json"
  postgres_config=/run/iotkit/postgres.json
else
  : >"$stage/secrets/postgres-password"
  printf '{"dsn":""}\n' >"$stage/secrets/postgres.json"
  postgres_config=
fi

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
topic write iotkit/v1/edge-nodes/$edge_node_id/status
topic write iotkit/v1/edge-nodes/$edge_node_id/descriptors
topic write iotkit/v1/edge-nodes/$edge_node_id/activation/result
topic write iotkit/v1/edge-nodes/$edge_node_id/recovery/result
topic write iotkit/v1/edge-nodes/$edge_node_id/recovery/completion-ack
topic read iotkit/v1/edge-nodes/$edge_node_id/accepted-through
topic read iotkit/v1/edge-nodes/$edge_node_id/activation/request
topic read iotkit/v1/edge-nodes/$edge_node_id/recovery/request
topic read iotkit/v1/edge-nodes/$edge_node_id/recovery/completion

user $edge_archive_principal
topic read iotkit/v1/edge-nodes/+/records
topic read iotkit/v1/edge-nodes/+/status
topic read iotkit/v1/edge-nodes/+/descriptors
topic read iotkit/v1/edge-nodes/+/activation/result
topic read iotkit/v1/edge-nodes/+/recovery/result
topic read iotkit/v1/edge-nodes/+/recovery/completion-ack
topic write iotkit/v1/edge-nodes/+/accepted-through
topic write iotkit/v1/edge-nodes/+/activation/request
topic write iotkit/v1/edge-nodes/+/recovery/request
topic write iotkit/v1/edge-nodes/+/recovery/completion

user $edge_output_principal
topic write iotkit/v1/sources/$edge_id/signals/+/observations
topic write pinikiet/v1/sources/$edge_id/sensors/+/observations
topic write pinikiet/v1/sources/$edge_id/status
EOF
for topic in "${edge_publish_topics[@]}"; do
  printf 'topic write %s\n' "$topic" >>"$stage/mosquitto/acl"
done

printf '%s:' "$edge_archive_principal" >"$stage/mosquitto/passwords"
tr -d '\r\n' <"$stage/secrets/edge-archive-mqtt-password" >>"$stage/mosquitto/passwords"
openssl rand -hex 24 >"$stage/secrets/output-mqtt-password"
printf '\n%s:' "$edge_output_principal" >>"$stage/mosquitto/passwords"
tr -d '\r\n' <"$stage/secrets/output-mqtt-password" >>"$stage/mosquitto/passwords"
printf '\n%s:' "$edge_node_id" >>"$stage/mosquitto/passwords"
tr -d '\r\n' <"$stage/edge-handoff/mqtt-password" >>"$stage/mosquitto/passwords"
printf '\n' >>"$stage/mosquitto/passwords"
docker run --rm --user "$(id -u):$(id -g)" \
  -v "$stage/mosquitto:/work" "$IOTKIT_MOSQUITTO_IMAGE" \
  mosquitto_passwd -U /work/passwords >/dev/null

cat >"$stage/edge-handoff/edge-mqtt.toml" <<EOF

[output.mqtt]
enabled = true
host = "$broker_host"
port = $broker_port
password_file = "/etc/iotkit/mqtt-password"
trust_mode = "bundle_only"
ca_file = "/etc/iotkit/broker-ca.pem"
EOF

compose_project="iotkit-edge-$(printf '%s' "$output_dir" | sha256sum | cut -c1-12)"
cat >"$stage/edge.env" <<EOF
COMPOSE_PROJECT_NAME=$compose_project
IOTKIT_RUNTIME_UID=$(id -u)
IOTKIT_RUNTIME_GID=$(id -g)
IOTKIT_EDGE_ID=$edge_id
IOTKIT_STORAGE_PROFILE=$storage_profile
IOTKIT_STORAGE_METADATA_FILE=$output_dir/storage-profile.json
IOTKIT_POSTGRES_CONFIG=$postgres_config
IOTKIT_POSTGRES_CONFIG_FILE=$output_dir/secrets/postgres.json
IOTKIT_POSTGRES_PASSWORD_FILE=$output_dir/secrets/postgres-password
IOTKIT_POSTGRES_PORT=$postgres_port
IOTKIT_MOSQUITTO_IMAGE=$IOTKIT_MOSQUITTO_IMAGE
IOTKIT_BROKER_HOST=$broker_host
IOTKIT_EDGE_HOST=$broker_host
IOTKIT_EDGE_ORIGIN=https://$broker_host:$edge_https_port
IOTKIT_BROKER_BIND=$broker_bind
IOTKIT_BROKER_PORT=$broker_port
IOTKIT_MOSQUITTO_CONFIG_FILE=$output_dir/mosquitto/mosquitto.conf
IOTKIT_MOSQUITTO_ACL_FILE=$output_dir/mosquitto/acl
IOTKIT_MOSQUITTO_PASSWORD_FILE=$output_dir/mosquitto/passwords
IOTKIT_MOSQUITTO_DIR=$output_dir/mosquitto
IOTKIT_BROKER_CERT_FILE=$output_dir/tls/server.pem
IOTKIT_BROKER_KEY_FILE=$output_dir/tls/server.key
IOTKIT_BROKER_CA_FILE=$output_dir/tls/ca.pem
IOTKIT_BROKER_TLS_DIR=$output_dir/tls
IOTKIT_BROKER_DATA_DIR=$output_dir/data/mosquitto
IOTKIT_EDGE_CLIENT_ID=iotkit-edge-archive-$edge_id
IOTKIT_EDGE_USERNAME=$edge_archive_principal
IOTKIT_EDGE_ARCHIVE_PASSWORD_FILE=$output_dir/secrets/edge-archive-mqtt-password
IOTKIT_EDGE_DATA_DIR=$output_dir/data/edge
IOTKIT_RECOVERY_DIR=$output_dir/recovery
IOTKIT_EDGE_STORAGE_WARNING_PERCENT=90
IOTKIT_OUTPUT_BROKER_URL=ssl://$broker_host:$broker_port
IOTKIT_OUTPUT_CLIENT_ID=iotkit-edge-output-$edge_id
IOTKIT_OUTPUT_USERNAME=$edge_output_principal
IOTKIT_OUTPUT_PASSWORD_FILE=$output_dir/secrets/output-mqtt-password
IOTKIT_OUTPUT_CA_FILE=$output_dir/tls/ca.pem
IOTKIT_CADDY_CONFIG_FILE=$output_dir/caddy/Caddyfile
IOTKIT_CADDY_DATA_DIR=$output_dir/data/caddy
IOTKIT_ACME_WEBROOT=$output_dir/data/acme-webroot
EOF

cat >"$stage/caddy/Caddyfile" <<EOF
https://$broker_host:$edge_https_port {
	handle /.well-known/acme-challenge/* {
		root * /srv/acme
		file_server
	}
	tls /etc/caddy/tls/server.pem /etc/caddy/tls/server.key
	reverse_proxy 127.0.0.1:8080
	header {
		Strict-Transport-Security "max-age=31536000"
	}
}
EOF

cat >"$stage/broker-cert.env" <<EOF
IOTKIT_CERT_DOMAIN=$broker_host
IOTKIT_CERT_FILE=$output_dir/tls/server.pem
IOTKIT_CERT_KEY_FILE=$output_dir/tls/server.key
IOTKIT_CERT_CA_FILE=$output_dir/tls/ca.pem
IOTKIT_CERT_EDGE_ENV=$output_dir/edge.env
IOTKIT_CERT_COMPOSE_FILE=$repo_root/deploy/compose.edge.yaml
IOTKIT_CERT_BROKER_PORT=$broker_port
IOTKIT_CERT_EDGE_HTTPS_PORT=$edge_https_port
IOTKIT_CERT_EDGE_ARCHIVE_PASSWORD_FILE=$output_dir/secrets/edge-archive-mqtt-password
IOTKIT_CERT_COMPOSE_PROJECT=$compose_project
IOTKIT_CERT_LEGO_PATH=$output_dir/data/lego
IOTKIT_CERT_LEGO_WEBROOT=$output_dir/data/acme-webroot
IOTKIT_CERT_LEGO_CHALLENGE=http
EOF

cat >"$stage/systemd/iotkit-broker-cert-renew.service" <<EOF
[Unit]
Description=Renew and safely activate the IoTKit MQTT certificate
After=docker.service network-online.target

[Service]
Type=oneshot
User=$(id -un)
ExecStart=$repo_root/scripts/iotkit-broker-cert renew --config $output_dir/broker-cert.env
EOF

cat >"$stage/systemd/iotkit-broker-cert-renew.timer" <<'EOF'
[Unit]
Description=Check the IoTKit MQTT certificate every day

[Timer]
OnCalendar=daily
RandomizedDelaySec=2h
Persistent=true

[Install]
WantedBy=timers.target
EOF

find "$stage" -type d -exec chmod 700 {} +
chmod 755 "$stage/data/acme-webroot"
find "$stage" -type f -exec chmod 600 {} +
mv "$stage" "$output_dir"
committed=true
trap - EXIT

echo "IoTKit Edge bootstrap created: $output_dir"
if [[ "$storage_profile" == "postgres" ]]; then
  echo "Start Edge: docker compose --env-file $output_dir/edge.env -f $repo_root/deploy/compose.edge.yaml -f $repo_root/deploy/compose.edge-postgres.yaml up --build --detach"
else
  echo "Start Edge: docker compose --env-file $output_dir/edge.env -f $repo_root/deploy/compose.edge.yaml up --build --detach"
fi
echo "Edge handoff directory (contains a plaintext credential): $output_dir/edge-handoff"
echo "Certificate timer templates: $output_dir/systemd (enable after ACME settings are added to broker-cert.env)"
