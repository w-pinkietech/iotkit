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

expect_status_and_error() {
  local label=$1 expected_status=$2 expected_error=$3 status
  shift 3
  set +e
  "$@" >"$scratch/$label.stdout" 2>"$scratch/$label.stderr"
  status=$?
  set -e
  if ((status != expected_status)); then
    echo "unexpected MQTT rejection status: $label" >&2
    exit 1
  fi
  if ! grep -Fq "$expected_error" "$scratch/$label.stderr"; then
    echo "unexpected MQTT rejection class: $label" >&2
    exit 1
  fi
}

expect_publish_denied() {
  local label=$1 expected_topic=$2 status broker_log
  shift 2
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
  broker_log=$(docker logs "$broker" 2>&1)
  if ! grep -F 'Denied PUBLISH' <<<"$broker_log" | grep -Fq "'$expected_topic'"; then
    echo "Broker did not record the expected publish denial: $label" >&2
    exit 1
  fi
}

expect_tls_rejected() {
  local label=$1 expected_error=$2 status
  shift 2
  set +e
  "$@" >"$scratch/$label.stdout" 2>"$scratch/$label.stderr"
  status=$?
  set -e
  if ((status == 0 || status == 124)); then
    echo "TLS probe did not return an explicit rejection: $label" >&2
    exit 1
  fi
  if ! grep -Eqi "$expected_error" "$scratch/$label.stdout" "$scratch/$label.stderr"; then
    echo "TLS probe returned the wrong rejection class: $label" >&2
    exit 1
  fi
}

mqtt_client() {
  local home=$1 command=$2 capture_path capture_label
  shift 2
  capture_path=$(mktemp "$scratch/process/$home.XXXXXX")
  capture_label=$(basename "$capture_path")
  rm -f "$capture_path"
  timeout 8s docker run --rm --network host \
    --user "$(id -u):$(id -g)" \
    -e "HOME=/work/clients/$home" \
    -e "IOTKIT_CAPTURE_LABEL=$capture_label" \
    -v "$scratch:/work:ro" \
    -v "$scratch/process:/capture" \
    "$IOTKIT_MOSQUITTO_IMAGE" /work/client-process-probe.sh "$command" "$@"
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
  local name=$1 cert=$2 key=$3
  docker run --detach --name "$name" \
    --user "$(id -u):$(id -g)" \
    --cap-drop ALL \
    --security-opt no-new-privileges:true \
    -p "127.0.0.1::8883" \
    -v "$scratch/mosquitto.conf:/mosquitto/config/mosquitto.conf:ro" \
    -v "$scratch/acl:/mosquitto/config/acl:ro" \
    -v "$scratch/passwords.db:/mosquitto/config/passwords:ro" \
    -v "$cert:/mosquitto/config/server.pem:ro" \
    -v "$key:/mosquitto/config/server.key:ro" \
    "$IOTKIT_MOSQUITTO_IMAGE" >/dev/null
  started_containers+=("$name")
}

published_port() {
  local mapping
  mapping=$(docker port "$1" 8883/tcp)
  [[ "$mapping" == 127.0.0.1:* ]] || {
    echo "Broker published an unexpected address: $1" >&2
    exit 1
  }
  printf '%s\n' "${mapping##*:}"
}

tls_probe() {
  timeout 8s openssl s_client -brief "$@" </dev/null
}

assert_cross_namespace_subscription_isolated() {
  local marker="edge-b-ack-probe-$$"
  local unauthorized_pid authorized_pid unauthorized_status authorized_status

  mqtt_client edge-a mosquitto_sub -h localhost -p "$tls_port" -W 3 -C 1 \
    -i acl-edge-a-sub -t iotkit/v1/edge-nodes/edge-b/accepted-through \
    >"$scratch/edge-a-reads-edge-b-ack.stdout" \
    2>"$scratch/edge-a-reads-edge-b-ack.stderr" &
  unauthorized_pid=$!
  mqtt_client edge-b mosquitto_sub -h localhost -p "$tls_port" -W 3 -C 1 \
    -i acl-edge-b-sub -t iotkit/v1/edge-nodes/edge-b/accepted-through \
    >"$scratch/edge-b-reads-own-ack.stdout" \
    2>"$scratch/edge-b-reads-own-ack.stderr" &
  authorized_pid=$!

  subscriptions_ready=false
  for _ in $(seq 1 30); do
    broker_log=$(docker logs "$broker" 2>&1)
    if grep -Fq 'Received SUBSCRIBE from acl-edge-a-sub' <<<"$broker_log" \
      && grep -Fq 'Received SUBSCRIBE from acl-edge-b-sub' <<<"$broker_log"; then
      subscriptions_ready=true
      break
    fi
    sleep 0.1
  done
  if [[ "$subscriptions_ready" != true ]]; then
    echo "MQTT ACL probe subscribers did not become ready" >&2
    exit 1
  fi

  expect_success site-publishes-edge-b-ack mqtt_client site mosquitto_pub \
    -h localhost -p "$tls_port" -q 1 \
    -t iotkit/v1/edge-nodes/edge-b/accepted-through -m "$marker"

  set +e
  wait "$authorized_pid"
  authorized_status=$?
  wait "$unauthorized_pid"
  unauthorized_status=$?
  set -e

  if ((authorized_status != 0)) \
    || ! grep -Fxq "$marker" "$scratch/edge-b-reads-own-ack.stdout"; then
    echo "authorized Edge B did not receive the controlled acknowledgement" >&2
    exit 1
  fi
  if ((unauthorized_status != 27)) \
    || ! grep -Fq 'Timed out' "$scratch/edge-a-reads-edge-b-ack.stderr" \
    || [[ -s "$scratch/edge-a-reads-edge-b-ack.stdout" ]]; then
    echo "Edge A cross-namespace subscription was not isolated" >&2
    exit 1
  fi
}

mkdir -p "$scratch/process"
cat >"$scratch/client-process-probe.sh" <<'EOF'
#!/bin/sh
set -eu
tr '\000' '\n' <"/proc/$$/cmdline" >"/capture/$IOTKIT_CAPTURE_LABEL.cmdline"
tr '\000' '\n' <"/proc/$$/environ" >"/capture/$IOTKIT_CAPTURE_LABEL.environ"
exec "$@"
EOF
chmod 700 "$scratch/client-process-probe.sh"

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
topic write iotkit/v1/edge-nodes/edge-a/activation/result
topic read iotkit/v1/edge-nodes/edge-a/accepted-through
topic read iotkit/v1/edge-nodes/edge-a/activation/request

user edge-b
topic write iotkit/v1/edge-nodes/edge-b/records
topic write iotkit/v1/edge-nodes/edge-b/descriptors
topic write iotkit/v1/edge-nodes/edge-b/activation/result
topic read iotkit/v1/edge-nodes/edge-b/accepted-through
topic read iotkit/v1/edge-nodes/edge-b/activation/request

user site
topic read iotkit/v1/edge-nodes/+/records
topic read iotkit/v1/edge-nodes/+/descriptors
topic read iotkit/v1/edge-nodes/+/activation/result
topic write iotkit/v1/edge-nodes/+/accepted-through
topic write iotkit/v1/edge-nodes/+/activation/request
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
write_client_config edge-b edge-b "$edge_b_password" ca.pem
write_client_config edge-a-wrong edge-a "$wrong_password" ca.pem
write_client_config site site "$site_password" ca.pem
write_anonymous_tls_config anonymous
write_plain_config plaintext

start_broker "$broker" "$scratch/server.pem" "$scratch/server.key"
start_broker "$expired_broker" "$scratch/expired-server.pem" "$scratch/expired-server.key"
tls_port=$(published_port "$broker")
expired_port=$(published_port "$expired_broker")

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
expect_success edge-a-own-activation-result mqtt_client edge-a mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-a/activation/result -m '{}'
expect_success site-own-activation-request mqtt_client site mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-a/activation/request -m '{}'
expect_status_and_error anonymous 135 'Connection error: Not authorized' \
  mqtt_client anonymous mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 -t test/anonymous -m '{}'
expect_status_and_error wrong-password 135 'Connection error: Not authorized' \
  mqtt_client edge-a-wrong mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-a/records -m '{}'
expect_publish_denied edge-a-writes-edge-b \
  iotkit/v1/edge-nodes/edge-b/records mqtt_client edge-a mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-b/records -m '{}'
assert_cross_namespace_subscription_isolated
expect_publish_denied site-writes-edge-records \
  iotkit/v1/edge-nodes/edge-a/records mqtt_client site mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-a/records -m '{}'
expect_publish_denied edge-a-writes-edge-b-activation-result \
  iotkit/v1/edge-nodes/edge-b/activation/result mqtt_client edge-a mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-b/activation/result -m '{}'
expect_publish_denied edge-a-writes-activation-request \
  iotkit/v1/edge-nodes/edge-a/activation/request mqtt_client edge-a mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-a/activation/request -m '{}'
expect_publish_denied site-writes-activation-result \
  iotkit/v1/edge-nodes/edge-a/activation/result mqtt_client site mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 \
  -t iotkit/v1/edge-nodes/edge-a/activation/result -m '{}'
expect_tls_rejected wrong-ca 'unable to get local issuer certificate|self-signed certificate' \
  tls_probe -connect "localhost:$tls_port" -servername localhost \
  -CAfile "$scratch/unrelated-ca.pem" -verify_return_error -verify_hostname localhost
expect_tls_rejected wrong-hostname 'hostname mismatch' \
  tls_probe -connect "localhost:$tls_port" -servername localhost \
  -CAfile "$scratch/ca.pem" -verify_return_error -verify_hostname 127.0.0.1
expect_tls_rejected expired-certificate 'certificate has expired' \
  tls_probe -connect "localhost:$expired_port" -servername localhost \
  -CAfile "$scratch/ca.pem" -verify_return_error -verify_hostname localhost
expect_status_and_error plaintext-to-tls 7 'Error: The connection was lost.' \
  mqtt_client plaintext mosquitto_pub \
  -h localhost -p "$tls_port" -q 1 -t test/plaintext -m '{}'

docker logs "$broker" >"$scratch/broker.log" 2>&1
docker logs "$expired_broker" >"$scratch/expired-broker.log" 2>&1
grep -Fq 'wrong version number' "$scratch/broker.log" || {
  echo "plaintext probe did not reach the TLS-only listener" >&2
  exit 1
}
for secret_file in "$scratch"/passwords/*.txt; do
  secret=$(<"$secret_file")
  if grep -Fq "$secret" "$scratch"/*.stdout \
    || grep -Fq "$secret" "$scratch"/*.stderr \
    || grep -Fq "$secret" "$scratch/broker.log" \
    || grep -Fq "$secret" "$scratch/expired-broker.log" \
    || grep -Fq "$secret" "$scratch/process"/*.cmdline \
    || grep -Fq "$secret" "$scratch/process"/*.environ; then
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
