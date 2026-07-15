#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
project="iotkit-mqtt-test-$$"
scratch=$(mktemp -d)
edge_pid=""

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
  if [[ -n "$edge_pid" ]] && kill -0 "$edge_pid" 2>/dev/null; then
    kill -INT "$edge_pid" 2>/dev/null || true
    wait "$edge_pid" 2>/dev/null || true
  fi
  docker compose -p "$project" -f "$repo_root/compose.dev.yaml" down --volumes --remove-orphans >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT

openssl rand -hex 24 >"$IOTKIT_SITE_PASSWORD_FILE"
openssl rand -hex 24 >"$scratch/edge-password"
chmod 600 "$IOTKIT_SITE_PASSWORD_FILE" "$scratch/edge-password"

jq --version >/dev/null
cargo build --manifest-path "$repo_root/Cargo.toml" -p iotkit-edge -p iotkit-edgectl

init_output=$("$repo_root/target/debug/iotkit-edgectl" --db "$scratch/edge.db" init)
edge_node_id=$(jq -er '.edge_node_id | select(type == "string" and length > 0)' <<<"$init_output")
jq -er '.ledger_epoch | select(type == "string" and length > 0)' <<<"$init_output" >/dev/null
sqlite3 "$scratch/edge.db" "INSERT INTO publication_log(epoch,kind,subtype,annotation_json,created_at) SELECT value,'annotation','epoch_start','{\"prior_epoch\":\"integration-prior\"}',unixepoch('subsec')*1000 FROM ledger_meta WHERE key='epoch'"

cat >"$IOTKIT_MOSQUITTO_ACL_FILE" <<EOF
user edge-node-01
topic write iotkit/v1/edge-nodes/edge-node-01/records
topic read iotkit/v1/edge-nodes/edge-node-01/accepted-through

user $edge_node_id
topic write iotkit/v1/edge-nodes/$edge_node_id/records
topic read iotkit/v1/edge-nodes/$edge_node_id/accepted-through

user site
topic read iotkit/v1/edge-nodes/+/records
topic write iotkit/v1/edge-nodes/+/accepted-through
EOF
chmod 644 "$IOTKIT_MOSQUITTO_ACL_FILE"

printf 'site:' >"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
tr -d '\n' <"$IOTKIT_SITE_PASSWORD_FILE" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
printf '\nedge-node-01:' >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
tr -d '\n' <"$scratch/edge-password" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
printf '\n%s:' "$edge_node_id" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
tr -d '\n' <"$scratch/edge-password" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
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
  -e IOTKIT_TEST_EDGE_PASSWORD_FILE=/run/iotkit-test/edge-password \
  -v /tmp/iotkit-go-mod:/tmp/gomodcache \
  -v /tmp/iotkit-go-cache:/tmp/gocache \
  -v "$repo_root:/src" \
  -v "$scratch:/run/iotkit-test:ro" \
  -w /src/iotkit-site \
  golang:1.25-bookworm \
  go test -tags=integration ./internal/mqttsite -run TestMQTTFixtureGetsApplicationAcknowledgement -count=1

query_output=$(docker compose -p "$project" -f "$repo_root/compose.dev.yaml" exec -T site \
  iotkit-site query --db /data/site.db --limit 10)
grep -q '"pub_seq": 1' <<<"$query_output"

# Site uses a clean MQTT session, so a broker restart must be followed by a
# fresh records subscription before the real Edge batch is published.
docker compose -p "$project" -f "$repo_root/compose.dev.yaml" restart broker

cat >"$scratch/edge.toml" <<EOF
[edge]
db_path = "$scratch/edge.db"
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
password_file = "$scratch/edge-password"
allow_insecure = true
EOF

"$repo_root/target/debug/iotkit-edge" --config "$scratch/edge.toml" >"$scratch/edge.log" 2>&1 &
edge_pid=$!

delivered=false
for _ in $(seq 1 60); do
  cursor=$(sqlite3 "$scratch/edge.db" "SELECT cursor_pub_seq FROM target_registry WHERE target_id='site'" 2>/dev/null || true)
  query_output=$(docker compose -p "$project" -f "$repo_root/compose.dev.yaml" exec -T site \
    iotkit-site query --db /data/site.db --limit 10 2>/dev/null || true)
  if [[ "$cursor" == "1" ]] && grep -q "\"edge_node_id\": \"$edge_node_id\"" <<<"$query_output"; then
    delivered=true
    break
  fi
  sleep 1
done

if [[ "$delivered" != true ]]; then
  docker compose -p "$project" -f "$repo_root/compose.dev.yaml" logs broker site
  sed -n '1,240p' "$scratch/edge.log"
  echo "Edge MQTT vertical slice timed out" >&2
  exit 1
fi

kill -INT "$edge_pid"
wait "$edge_pid"
edge_pid=""

echo "Edge -> MQTT -> Site -> accepted-through vertical slice: OK"
