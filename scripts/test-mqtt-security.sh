#!/usr/bin/env bash
set -euo pipefail

umask 077

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
# shellcheck disable=SC1091
source "$repo_root/deploy/mosquitto-image.env"

for command in docker openssl grep timeout; do
  command -v "$command" >/dev/null || {
    echo "required command not found: $command" >&2
    exit 1
  }
done

scratch=$(mktemp -d /tmp/iotkit-mqtt-security.XXXXXX)
broker="iotkit-mqtt-security-$$"
expired_broker="iotkit-mqtt-security-expired-$$"
tls_port=$((30000 + $$ % 10000))
expired_port=$((tls_port + 1))
unused_port=$((tls_port + 2))
started_containers=()

cleanup() {
  local container
  for container in "${started_containers[@]}"; do
    docker rm --force "$container" >/dev/null 2>&1 || true
  done
  rm -rf "$scratch"
}
trap cleanup EXIT

expect_success() {
  local label=$1
  shift
  if ! "$@" >"$scratch/$label.stdout" 2>"$scratch/$label.stderr"; then
    echo "expected MQTT success: $label" >&2
    exit 1
  fi
}

expect_rejected() {
  local label=$1 status
  shift
  set +e
  "$@" >"$scratch/$label.stdout" 2>"$scratch/$label.stderr"
  status=$?
  set -e
  if ((status == 0)); then
    echo "expected MQTT rejection: $label" >&2
    exit 1
  fi
  if ((status == 124)); then
    echo "MQTT rejection timed out without an explicit result: $label" >&2
    exit 1
  fi
}

expect_publish_denied() {
  local label=$1 status
  shift
  set +e
  "$@" >"$scratch/$label.stdout" 2>"$scratch/$label.stderr"
  status=$?
  set -e
  if ((status == 124)); then
    echo "MQTT publish denial timed out without an explicit PUBACK: $label" >&2
    exit 1
  fi
  if ! grep -Fq 'failed: Not authorized.' "$scratch/$label.stderr"; then
    echo "MQTT publish did not return an explicit Not authorized result: $label" >&2
    exit 1
  fi
}

mqtt_client() {
  local home=$1 command=$2
  shift 2
  timeout 8s docker run --rm --network host \
    --user "$(id -u):$(id -g)" \
    -e "HOME=/work/clients/$home" \
    -v "$scratch:/work:ro" \
    "$IOTKIT_MOSQUITTO_IMAGE" "$command" "$@"
}

write_client_config() {
  local label=$1 username=$2 password=$3 ca_file=$4
  local config_dir="$scratch/clients/$label/.config"
  mkdir -p "$config_dir"
  cat >"$config_dir/mosquitto_pub" <<EOF
-u $username
-P $password
--cafile /work/$ca_file
-V mqttv5
EOF
  cp "$config_dir/mosquitto_pub" "$config_dir/mosquitto_sub"
  chmod 600 "$config_dir/mosquitto_pub" "$config_dir/mosquitto_sub"
}

write_anonymous_tls_config() {
  local label=$1
  local config_dir="$scratch/clients/$label/.config"
  mkdir -p "$config_dir"
  cat >"$config_dir/mosquitto_pub" <<'EOF'
--cafile /work/ca.pem
-V mqttv5
EOF
  cp "$config_dir/mosquitto_pub" "$config_dir/mosquitto_sub"
  chmod 600 "$config_dir/mosquitto_pub" "$config_dir/mosquitto_sub"
}

write_plain_config() {
  local label=$1
  local config_dir="$scratch/clients/$label/.config"
  mkdir -p "$config_dir"
  : >"$config_dir/mosquitto_pub"
  cp "$config_dir/mosquitto_pub" "$config_dir/mosquitto_sub"
  chmod 600 "$config_dir/mosquitto_pub" "$config_dir/mosquitto_sub"
}

start_broker() {
  local name=$1 port=$2 cert=$3 key=$4
  docker run --detach --name "$name" \
    --user "$(id -u):$(id -g)" \
    --cap-drop ALL \
    --security-opt no-new-privileges:true \
    -p "127.0.0.1:$port:8883" \
    -v "$scratch/mosquitto.conf:/mosquitto/config/mosquitto.conf:ro" \
    -v "$scratch/acl:/mosquitto/config/acl:ro" \
    -v "$scratch/passwords.db:/mosquitto/config/passwords:ro" \
    -v "$cert:/mosquitto/config/server.pem:ro" \
    -v "$key:/mosquitto/config/server.key:ro" \
    "$IOTKIT_MOSQUITTO_IMAGE" >/dev/null
  started_containers+=("$name")
}

openssl req -x509 -newkey rsa:2048 -nodes -days 2 \
  -subj '/CN=IoTKit MQTT Security Test CA' \
  -keyout "$scratch/ca.key" -out "$scratch/ca.pem" >/dev/null 2>&1
openssl req -x509 -newkey rsa:2048 -nodes -days 2 \
  -subj '/CN=Unrelated MQTT Test CA' \
  -keyout "$scratch/unrelated-ca.key" -out "$scratch/unrelated-ca.pem" >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes -subj '/CN=localhost' \
  -keyout "$scratch/server.key" -out "$scratch/server.csr" >/dev/null 2>&1
cat >"$scratch/server.ext" <<'EOF'
subjectAltName=DNS:localhost
extendedKeyUsage=serverAuth
EOF
openssl x509 -req -days 2 -in "$scratch/server.csr" \
  -CA "$scratch/ca.pem" -CAkey "$scratch/ca.key" -CAcreateserial \
  -extfile "$scratch/server.ext" -out "$scratch/server.pem" >/dev/null 2>&1

mkdir -p "$scratch/expired-ca-db/newcerts"
: >"$scratch/expired-ca-db/index.txt"
printf '1000\n' >"$scratch/expired-ca-db/serial"
cat >"$scratch/expired-ca.cnf" <<EOF
[ca]
default_ca=CA_default

[CA_default]
dir=$scratch/expired-ca-db
database=\$dir/index.txt
new_certs_dir=\$dir/newcerts
certificate=$scratch/ca.pem
private_key=$scratch/ca.key
serial=\$dir/serial
default_md=sha256
policy=policy_any
x509_extensions=server_ext

[policy_any]
commonName=supplied

[server_ext]
basicConstraints=CA:false
keyUsage=digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=DNS:localhost
EOF
openssl ca -batch -config "$scratch/expired-ca.cnf" \
  -startdate 20200101000000Z -enddate 20200102000000Z \
  -in "$scratch/server.csr" -out "$scratch/expired-server.pem" >/dev/null 2>&1
cp "$scratch/server.key" "$scratch/expired-server.key"

mkdir -p "$scratch/passwords"
openssl rand -hex 24 >"$scratch/passwords/edge-a.txt"
openssl rand -hex 24 >"$scratch/passwords/edge-b.txt"
openssl rand -hex 24 >"$scratch/passwords/site.txt"
openssl rand -hex 24 >"$scratch/passwords/wrong.txt"
chmod 600 "$scratch/passwords"/*.txt "$scratch"/*.key

edge_a_password=$(<"$scratch/passwords/edge-a.txt")
edge_b_password=$(<"$scratch/passwords/edge-b.txt")
site_password=$(<"$scratch/passwords/site.txt")
wrong_password=$(<"$scratch/passwords/wrong.txt")

cat >"$scratch/passwords.db" <<EOF
edge-a:$edge_a_password
edge-b:$edge_b_password
site:$site_password
EOF
docker run --rm --user "$(id -u):$(id -g)" \
  -v "$scratch:/work" "$IOTKIT_MOSQUITTO_IMAGE" \
  mosquitto_passwd -U /work/passwords.db >/dev/null

cat >"$scratch/acl" <<'EOF'
user edge-a
topic write iotkit/v1/edge-nodes/edge-a/records
topic write iotkit/v1/edge-nodes/edge-a/descriptors
topic read iotkit/v1/edge-nodes/edge-a/accepted-through

user edge-b
topic write iotkit/v1/edge-nodes/edge-b/records
topic write iotkit/v1/edge-nodes/edge-b/descriptors
topic read iotkit/v1/edge-nodes/edge-b/accepted-through

user site
topic read iotkit/v1/edge-nodes/+/records
topic read iotkit/v1/edge-nodes/+/descriptors
topic write iotkit/v1/edge-nodes/+/accepted-through
EOF

cat >"$scratch/mosquitto.conf" <<'EOF'
listener 8883 0.0.0.0
protocol mqtt
allow_anonymous false
password_file /mosquitto/config/passwords
acl_file /mosquitto/config/acl
certfile /mosquitto/config/server.pem
keyfile /mosquitto/config/server.key
tls_version tlsv1.2
require_certificate false
message_size_limit 1048576
max_packet_size 1114112
max_inflight_messages 20
max_queued_messages 1000
max_connections 128
memory_limit 268435456
persistence false
log_dest stdout
log_type all
EOF

write_client_config edge-a edge-a "$edge_a_password" ca.pem
write_client_config edge-a-wrong edge-a "$wrong_password" ca.pem
write_client_config edge-a-wrong-ca edge-a "$edge_a_password" unrelated-ca.pem
write_client_config site site "$site_password" ca.pem
write_client_config expired edge-a "$edge_a_password" ca.pem
write_anonymous_tls_config anonymous
write_plain_config plaintext

start_broker "$broker" "$tls_port" "$scratch/server.pem" "$scratch/server.key"
start_broker "$expired_broker" "$expired_port" \
  "$scratch/expired-server.pem" "$scratch/expired-server.key"

broker_ready=false
for _ in $(seq 1 30); do
  if mqtt_client edge-a mosquitto_pub -h localhost -p "$tls_port" -q 1 \
    -t iotkit/v1/edge-nodes/edge-a/records -m '{}' >/dev/null 2>&1; then
    broker_ready=true
    break
  fi
  sleep 0.2
done
if [[ "$broker_ready" != true ]]; then
  echo "MQTT security Broker did not become ready" >&2
  exit 1
fi

expect_success edge-a-own-records mqtt_client edge-a mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-a/records -m '{}'
expect_success site-own-ack mqtt_client site mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-a/accepted-through -m '{}'
expect_rejected anonymous mqtt_client anonymous mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 -t test/anonymous -m '{}'
expect_rejected wrong-password mqtt_client edge-a-wrong mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-a/records -m '{}'
expect_publish_denied edge-a-writes-edge-b mqtt_client edge-a mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-b/records -m '{}'
expect_rejected edge-a-reads-edge-b-ack mqtt_client edge-a mosquitto_sub \
  -h localhost -p "$tls_port" -W 3 \
  -t iotkit/v1/edge-nodes/edge-b/accepted-through
expect_publish_denied site-writes-edge-records mqtt_client site mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-a/records -m '{}'
expect_rejected wrong-ca mqtt_client edge-a-wrong-ca mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-a/records -m '{}'
expect_rejected wrong-hostname mqtt_client edge-a mosquitto_pub \
  -h 127.0.0.1 -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-a/records -m '{}'
expect_rejected expired-certificate mqtt_client expired mosquitto_pub \
  -h localhost -p "$expired_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-a/records -m '{}'
expect_rejected plaintext-to-tls mqtt_client plaintext mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 -t test/plaintext -m '{}'
expect_rejected no-plaintext-listener mqtt_client plaintext mosquitto_pub \
  -h localhost -p "$unused_port" -q 1 -t test/plaintext -m '{}'

docker logs "$broker" >"$scratch/broker.log" 2>&1
docker logs "$expired_broker" >"$scratch/expired-broker.log" 2>&1
for secret_file in "$scratch"/passwords/*.txt; do
  secret=$(<"$secret_file")
  if grep -Fq "$secret" "$scratch"/*.stdout \
    || grep -Fq "$secret" "$scratch"/*.stderr \
    || grep -Fq "$secret" "$scratch/broker.log" \
    || grep -Fq "$secret" "$scratch/expired-broker.log"; then
    echo "MQTT credential leaked into diagnostics" >&2
    exit 1
  fi
done

[[ "$(docker inspect --format '{{.Config.Image}}' "$broker")" == \
  "eclipse-mosquitto:2.0.22" ]] || {
  echo "MQTT security Broker did not use the fixed image" >&2
  exit 1
}
[[ "$(docker port "$broker" 8883/tcp)" == "127.0.0.1:$tls_port" ]] || {
  echo "MQTT security Broker published an unexpected port" >&2
  exit 1
}
[[ "$(grep -Ec '^listener 8883( |$)' "$scratch/mosquitto.conf")" == "1" ]]
! grep -Eq '^listener 1883( |$)' "$scratch/mosquitto.conf"

echo "MQTT security matrix: OK"
