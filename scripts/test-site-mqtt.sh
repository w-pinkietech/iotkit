#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
project="iotkit-mqtt-test-$$"
scratch=$(mktemp -d)
gateway_pid=""

export IOTKIT_MOSQUITTO_PASSWORD_FILE="$scratch/passwords"
export IOTKIT_MOSQUITTO_ACL_FILE="$scratch/acl"
export IOTKIT_SITE_PASSWORD_FILE="$scratch/site-password"
export IOTKIT_SITE_DATA_DIR="$scratch/data"
export IOTKIT_DEV_UID="$(id -u)"
export IOTKIT_DEV_GID="$(id -g)"
mkdir -p "$IOTKIT_SITE_DATA_DIR"
chmod 700 "$scratch"
chmod 755 "$IOTKIT_SITE_DATA_DIR"

cleanup() {
  if [[ -n "$gateway_pid" ]] && kill -0 "$gateway_pid" 2>/dev/null; then
    kill -INT "$gateway_pid" 2>/dev/null || true
    wait "$gateway_pid" 2>/dev/null || true
  fi
  docker compose -p "$project" -f "$repo_root/compose.dev.yaml" down --volumes --remove-orphans >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT

openssl rand -hex 24 >"$IOTKIT_SITE_PASSWORD_FILE"
openssl rand -hex 24 >"$scratch/gateway-password"
chmod 600 "$IOTKIT_SITE_PASSWORD_FILE" "$scratch/gateway-password"

cargo build --manifest-path "$repo_root/Cargo.toml" -p iotkit-gateway --bin iotkit-gateway

cat >"$scratch/init.toml" <<EOF
[gateway]
db_path = "$scratch/gateway.db"
health_json_path = "$scratch/health.json"

[adapters.bravepi]
enabled = false

[adapters.rpi_local]
enabled = false

[api]
enabled = false

[exit.mqtt]
enabled = true
host = "127.0.0.1"
port = 18883
password_file = "$scratch/gateway-password"
allow_insecure = true
EOF

"$repo_root/target/debug/iotkit-gateway" --config "$scratch/init.toml" >"$scratch/gateway-init.log" 2>&1 &
gateway_pid=$!
initialized=false
for _ in $(seq 1 100); do
  if sqlite3 "$scratch/gateway.db" "SELECT value FROM ledger_meta WHERE key='gateway_identity'" 2>/dev/null | grep -q .; then
    initialized=true
    break
  fi
  if ! kill -0 "$gateway_pid" 2>/dev/null; then
    break
  fi
  sleep 0.1
done
if [[ "$initialized" != true ]] || ! kill -0 "$gateway_pid" 2>/dev/null; then
  wait "$gateway_pid" 2>/dev/null || true
  gateway_pid=""
  sed -n '1,240p' "$scratch/gateway-init.log"
  echo "Gateway database initialization failed" >&2
  exit 1
fi
kill -INT "$gateway_pid"
wait "$gateway_pid"
gateway_pid=""

gateway_identity=$(sqlite3 "$scratch/gateway.db" "SELECT value FROM ledger_meta WHERE key='gateway_identity'")
sqlite3 "$scratch/gateway.db" "INSERT INTO publication_log(epoch,kind,subtype,annotation_json,created_at) SELECT value,'annotation','epoch_start','{\"prior_epoch\":\"integration-prior\"}',unixepoch('subsec')*1000 FROM ledger_meta WHERE key='epoch'"

cat >"$IOTKIT_MOSQUITTO_ACL_FILE" <<EOF
user gateway-01
topic write iotkit/v1/gateways/gateway-01/records
topic read iotkit/v1/gateways/gateway-01/accepted-through

user $gateway_identity
topic write iotkit/v1/gateways/$gateway_identity/records
topic read iotkit/v1/gateways/$gateway_identity/accepted-through

user site
topic read iotkit/v1/gateways/+/records
topic write iotkit/v1/gateways/+/accepted-through
EOF
chmod 644 "$IOTKIT_MOSQUITTO_ACL_FILE"

printf 'site:' >"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
tr -d '\n' <"$IOTKIT_SITE_PASSWORD_FILE" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
printf '\ngateway-01:' >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
tr -d '\n' <"$scratch/gateway-password" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
printf '\n%s:' "$gateway_identity" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
tr -d '\n' <"$scratch/gateway-password" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
printf '\n' >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
chmod 600 "$IOTKIT_MOSQUITTO_PASSWORD_FILE"

docker run --rm --user "$(id -u):$(id -g)" \
  -v "$scratch:/work" \
  eclipse-mosquitto:2.0 \
  mosquitto_passwd -U /work/passwords
chmod 644 "$IOTKIT_MOSQUITTO_PASSWORD_FILE"

docker compose -p "$project" -f "$repo_root/compose.dev.yaml" up --build --detach

docker run --rm --user "$(id -u):$(id -g)" \
  --network "${project}_default" \
  -e HOME=/tmp \
  -e GOMODCACHE=/tmp/gomodcache \
  -e GOCACHE=/tmp/gocache \
  -e IOTKIT_TEST_BROKER_URL=tcp://broker:1883 \
  -e IOTKIT_TEST_GATEWAY_PASSWORD_FILE=/run/iotkit-test/gateway-password \
  -v /tmp/iotkit-go-mod:/tmp/gomodcache \
  -v /tmp/iotkit-go-cache:/tmp/gocache \
  -v "$repo_root:/src" \
  -v "$scratch:/run/iotkit-test:ro" \
  -w /src/iotkit-site-server \
  golang:1.25-bookworm \
  go test -tags=integration ./internal/mqttsite -run TestMQTTFixtureGetsApplicationAcknowledgement -count=1

query_output=$(docker compose -p "$project" -f "$repo_root/compose.dev.yaml" exec -T site \
  iotkit-site-server query --db /data/site.db --limit 10)
grep -q '"pub_seq": 1' <<<"$query_output"

cat >"$scratch/gateway.toml" <<EOF
[gateway]
db_path = "$scratch/gateway.db"
health_json_path = "$scratch/health.json"

[adapters.bravepi]
enabled = false

[adapters.rpi_local]
enabled = false

[api]
enabled = false

[exit.mqtt]
enabled = true
host = "127.0.0.1"
port = 18883
password_file = "$scratch/gateway-password"
allow_insecure = true
EOF

"$repo_root/target/debug/iotkit-gateway" --config "$scratch/gateway.toml" >"$scratch/gateway.log" 2>&1 &
gateway_pid=$!

delivered=false
for _ in $(seq 1 60); do
  cursor=$(sqlite3 "$scratch/gateway.db" "SELECT cursor_pub_seq FROM target_registry WHERE target_id='site'" 2>/dev/null || true)
  query_output=$(docker compose -p "$project" -f "$repo_root/compose.dev.yaml" exec -T site \
    iotkit-site-server query --db /data/site.db --limit 10 2>/dev/null || true)
  if [[ "$cursor" == "1" ]] && grep -q "\"gateway_identity\": \"$gateway_identity\"" <<<"$query_output"; then
    delivered=true
    break
  fi
  sleep 1
done

if [[ "$delivered" != true ]]; then
  docker compose -p "$project" -f "$repo_root/compose.dev.yaml" logs broker site
  sed -n '1,240p' "$scratch/gateway.log"
  echo "Gateway MQTT vertical slice timed out" >&2
  exit 1
fi

kill -INT "$gateway_pid"
wait "$gateway_pid"
gateway_pid=""

echo "Gateway -> MQTT -> Site -> accepted-through vertical slice: OK"
