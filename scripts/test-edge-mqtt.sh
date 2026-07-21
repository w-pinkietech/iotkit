#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cargo_target_dir=${CARGO_TARGET_DIR:-"$repo_root/target"}
go_mod_cache=${IOTKIT_TEST_GO_MOD_CACHE:-${GOMODCACHE:-/tmp/iotkit-go-mod}}
go_build_cache=${IOTKIT_TEST_GO_BUILD_CACHE:-${GOCACHE:-/tmp/iotkit-go-cache}}
# shellcheck disable=SC1091
source "$repo_root/deploy/mosquitto-image.env"
export IOTKIT_MOSQUITTO_IMAGE
project="iotkit-mqtt-test-$$"
broker_port=$((20000 + $$ % 20000))
scratch=$(mktemp -d)
edge_pid=""

command -v sqlite3 >/dev/null || { echo "required command missing: sqlite3" >&2; exit 1; }

export IOTKIT_MOSQUITTO_PASSWORD_FILE="$scratch/passwords"
export IOTKIT_MOSQUITTO_ACL_FILE="$scratch/acl"
export IOTKIT_EDGE_ARCHIVE_PASSWORD_FILE="$scratch/edge-password"
export IOTKIT_EDGE_DATA_DIR="$scratch/data"
export IOTKIT_DEV_UID="$(id -u)"
export IOTKIT_DEV_GID="$(id -g)"
export IOTKIT_DEV_BROKER_PORT="$broker_port"
mkdir -p "$IOTKIT_EDGE_DATA_DIR"
mkdir -p "$go_mod_cache" "$go_build_cache"
chmod 700 "$scratch"
chmod 755 "$IOTKIT_EDGE_DATA_DIR"

cleanup() {
  if [[ -n "$edge_pid" ]] && kill -0 "$edge_pid" 2>/dev/null; then
    kill -INT "$edge_pid" 2>/dev/null || true
    wait "$edge_pid" 2>/dev/null || true
  fi
  docker compose -p "$project" -f "$repo_root/compose.dev.yaml" down --volumes --remove-orphans >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT

openssl rand -hex 24 >"$IOTKIT_EDGE_ARCHIVE_PASSWORD_FILE"
openssl rand -hex 24 >"$scratch/edge-password"
chmod 600 "$IOTKIT_EDGE_ARCHIVE_PASSWORD_FILE" "$scratch/edge-password"

jq --version >/dev/null
cargo build --manifest-path "$repo_root/Cargo.toml" -p iotkit-edge-node -p iotkit-edge-nodectl

"$cargo_target_dir/debug/iotkit-edge-nodectl" --db "$scratch/edge.db" init >/dev/null
identity_output=$("$cargo_target_dir/debug/iotkit-edge-nodectl" --db "$scratch/edge.db" identity)
edge_node_id=$(jq -er '.edge_node_id | select(type == "string" and length > 0)' <<<"$identity_output")
jq -er '.ledger_epoch | select(type == "string" and length > 0)' <<<"$identity_output" >/dev/null
binding_output=$("$cargo_target_dir/debug/iotkit-edge-nodectl" --db "$scratch/edge.db" mqtt-binding)
edge_username=$(jq -er '.username | select(type == "string" and length > 0)' <<<"$binding_output")
edge_records_topic=$(jq -er '.records_topic | select(type == "string" and length > 0)' <<<"$binding_output")
edge_ack_topic=$(jq -er '.accepted_through_topic | select(type == "string" and length > 0)' <<<"$binding_output")
edge_descriptor_topic=$(jq -er '.descriptor_topic | select(type == "string" and length > 0)' <<<"$binding_output")
jq -e --arg edge_node_id "$edge_node_id" \
  '.edge_node_id == $edge_node_id and .username == $edge_node_id and .qos == 1 and .retain == false and .descriptor_retain == true and (.client_id | type == "string" and length > 0)' \
  <<<"$binding_output" >/dev/null

cat >"$IOTKIT_MOSQUITTO_ACL_FILE" <<EOF
user edge-node-01
topic write iotkit/v1/edge-nodes/edge-node-01/records
topic write iotkit/v1/edge-nodes/edge-node-01/descriptors
topic write iotkit/v1/edge-nodes/edge-node-01/activation/result
topic read iotkit/v1/edge-nodes/edge-node-01/accepted-through
topic read iotkit/v1/edge-nodes/edge-node-01/activation/request

user $edge_username
topic write $edge_records_topic
topic write $edge_descriptor_topic
topic write iotkit/v1/edge-nodes/$edge_node_id/activation/result
topic read $edge_ack_topic
topic read iotkit/v1/edge-nodes/$edge_node_id/activation/request

user edge
topic read iotkit/v1/edge-nodes/+/records
topic read iotkit/v1/edge-nodes/+/descriptors
topic read iotkit/v1/edge-nodes/+/activation/result
topic write iotkit/v1/edge-nodes/+/accepted-through
topic write iotkit/v1/edge-nodes/+/activation/request
EOF
chmod 644 "$IOTKIT_MOSQUITTO_ACL_FILE"

printf 'edge:' >"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
tr -d '\n' <"$IOTKIT_EDGE_ARCHIVE_PASSWORD_FILE" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
printf '\nedge-node-01:' >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
tr -d '\n' <"$scratch/edge-password" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
printf '\n%s:' "$edge_username" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
tr -d '\n' <"$scratch/edge-password" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
printf '\n' >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
chmod 600 "$IOTKIT_MOSQUITTO_PASSWORD_FILE"

docker run --rm --user "$(id -u):$(id -g)" \
  -v "$scratch:/work" \
  "$IOTKIT_MOSQUITTO_IMAGE" \
  mosquitto_passwd -U /work/passwords
chmod 644 "$IOTKIT_MOSQUITTO_PASSWORD_FILE"

docker compose -p "$project" -f "$repo_root/compose.dev.yaml" up --build --detach

if ! docker run --rm --user "$(id -u):$(id -g)" \
  --network "${project}_default" \
  -e HOME=/tmp \
  -e GOMODCACHE=/tmp/gomodcache \
  -e GOCACHE=/tmp/gocache \
  -e IOTKIT_TEST_BROKER_URL=tcp://broker:1883 \
  -e IOTKIT_TEST_EDGE_PASSWORD_FILE=/run/iotkit-test/edge-password \
  -v "$go_mod_cache:/tmp/gomodcache" \
  -v "$go_build_cache:/tmp/gocache" \
  -v "$repo_root:/src" \
  -v "$scratch:/run/iotkit-test:ro" \
  -w /src/iotkit-edge \
  golang:1.25-bookworm \
  go test -tags=integration ./internal/mqttedge \
    -run TestMQTTPreActivationFixtureGetsNoApplicationAcknowledgement -count=1; then
  docker compose -p "$project" -f "$repo_root/compose.dev.yaml" logs broker edge
  exit 1
fi

query_output=$(docker compose -p "$project" -f "$repo_root/compose.dev.yaml" exec -T edge \
  iotkit-edge query --db /data/edge.db --limit 10)
if grep -q '"pub_seq": 1' <<<"$query_output"; then
  echo "Edge accepted a pre-activation record" >&2
  exit 1
fi

# Edge uses a clean MQTT session, so a broker restart must be followed by a
# fresh records subscription before the real Edge batch is published.
docker compose -p "$project" -f "$repo_root/compose.dev.yaml" restart broker

cat >"$scratch/edge.toml" <<EOF
[edge_node]
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
port = $broker_port
password_file = "$scratch/edge-password"
allow_insecure = true
EOF

"$cargo_target_dir/debug/iotkit-edge-node" --config "$scratch/edge.toml" >"$scratch/edge.log" 2>&1 &
edge_pid=$!

if ! docker run --rm --user "$(id -u):$(id -g)" \
  --network "${project}_default" \
  -e HOME=/tmp \
  -e GOMODCACHE=/tmp/gomodcache \
  -e GOCACHE=/tmp/gocache \
  -e IOTKIT_TEST_BROKER_URL=tcp://broker:1883 \
  -e IOTKIT_TEST_EDGE_ARCHIVE_PASSWORD_FILE=/run/iotkit-test/edge-password \
  -e IOTKIT_TEST_EDGE_NODE_ID="$edge_node_id" \
  -v "$go_mod_cache:/tmp/gomodcache" \
  -v "$go_build_cache:/tmp/gocache" \
  -v "$repo_root:/src" \
  -v "$scratch:/run/iotkit-test:ro" \
  -w /src/iotkit-edge \
  golang:1.25-bookworm \
  go test -tags=integration ./internal/mqttedge \
    -run TestMQTTRetainedDescriptorIsAvailableToLateSubscriber -count=1; then
  docker compose -p "$project" -f "$repo_root/compose.dev.yaml" logs broker edge
  sed -n '1,240p' "$scratch/edge.log"
  exit 1
fi

descriptor_stored=false
for _ in $(seq 1 30); do
  descriptor_count=$(sqlite3 "$IOTKIT_EDGE_DATA_DIR/edge.db" \
    "SELECT count(*) FROM edge_descriptor_state WHERE edge_node_id='$edge_node_id'" 2>/dev/null || true)
  if [[ "$descriptor_count" == "1" ]]; then
    descriptor_stored=true
    break
  fi
  sleep 1
done
if [[ "$descriptor_stored" != true ]]; then
  docker compose -p "$project" -f "$repo_root/compose.dev.yaml" logs broker edge
  sed -n '1,240p' "$scratch/edge.log"
  echo "Edge did not durably replicate the Edge descriptor" >&2
  exit 1
fi

now_ms=$(date +%s%3N)
sqlite3 "$scratch/edge.db" <<SQL
PRAGMA foreign_keys = ON;
INSERT INTO devices(system_id, hardware_id, kind, state, created_at)
VALUES(X'0123456789abcdef0123456789abcdef', 'vertical-preactivation',
       'individual', 'active', $now_ms);
INSERT INTO series(
  system_id, measurement_key, channel_index, variant, quarantined,
  value_semantics, unit, created_at, calibration_review
) VALUES(
  X'0123456789abcdef0123456789abcdef', 'temperature_c', -1, 'primary', 0,
  'calibrated', 'Cel', $now_ms, 0
);
INSERT INTO readings(
  series_id, received_at, time_source, time_quality, values_json,
  quarantined, event_time, event_time_source
) VALUES(
  last_insert_rowid(), $now_ms, 'edge', 'unsynced', '[19.5]',
  0, $now_ms, 'received_at'
);
SQL
[[ "$(sqlite3 "$scratch/edge.db" 'SELECT count(*) FROM readings')" == "1" ]]
[[ "$(sqlite3 "$scratch/edge.db" 'SELECT count(*) FROM publication_log')" == "0" ]]

if ! docker run --rm --user "$(id -u):$(id -g)" \
  --network "${project}_default" \
  -e HOME=/tmp \
  -e GOMODCACHE=/tmp/gomodcache \
  -e GOCACHE=/tmp/gocache \
  -e IOTKIT_TEST_EDGE_DB=/run/iotkit-test/data/edge.db \
  -e IOTKIT_TEST_EDGE_NODE_ID="$edge_node_id" \
  -v "$go_mod_cache:/tmp/gomodcache" \
  -v "$go_build_cache:/tmp/gocache" \
  -v "$repo_root:/src" \
  -v "$scratch:/run/iotkit-test:rw" \
  -w /src/iotkit-edge \
  golang:1.25-bookworm \
  go test -tags=integration ./internal/mqttedge \
    -run TestEdgeNodeActivationCommandConvergesWithEdge -count=1; then
  docker compose -p "$project" -f "$repo_root/compose.dev.yaml" logs broker edge
  sed -n '1,240p' "$scratch/edge.log"
  exit 1
fi

cleanup_complete=false
for _ in $(seq 1 30); do
  activation_state=$(sqlite3 "$scratch/edge.db" \
    "SELECT state || ':' || discard_through_reading_seq FROM edge_node_activation")
  reading_count=$(sqlite3 "$scratch/edge.db" 'SELECT count(*) FROM readings')
  if [[ "$activation_state" == "active:1" && "$reading_count" == "0" ]]; then
    cleanup_complete=true
    break
  fi
  sleep 0.2
done
[[ "$cleanup_complete" == true ]] || {
  echo "pre-activation boundary did not converge or clean up" >&2
  exit 1
}

kill -INT "$edge_pid"
wait "$edge_pid"
edge_pid=""
"$cargo_target_dir/debug/iotkit-edge-node" --config "$scratch/edge.toml" \
  >>"$scratch/edge.log" 2>&1 &
edge_pid=$!
sleep 1
[[ "$(sqlite3 "$scratch/edge.db" \
  "SELECT state || ':' || discard_through_reading_seq FROM edge_node_activation")" == "active:1" ]]

smoke_output=""
for _ in $(seq 1 60); do
  if smoke_output=$("$cargo_target_dir/debug/iotkit-edge-nodectl" \
    --db "$scratch/edge.db" smoke enqueue 2>"$scratch/smoke-enqueue.err"); then
    break
  fi
  sleep 1
done
if [[ -z "$smoke_output" ]]; then
  sed -n '1,240p' "$scratch/edge.log"
  sed -n '1,120p' "$scratch/smoke-enqueue.err"
  echo "Commissioning smoke could not be enqueued" >&2
  exit 1
fi
smoke_test_id=$(jq -er '.test_id | select(type == "string" and length > 0)' <<<"$smoke_output")
smoke_epoch=$(jq -er '.ledger_epoch | select(type == "string" and length > 0)' <<<"$smoke_output")
smoke_pub_seq=$(jq -er '.pub_seq | select(type == "number" and . > 0)' <<<"$smoke_output")

delivered=false
for _ in $(seq 1 60); do
  status_output=$("$cargo_target_dir/debug/iotkit-edge-nodectl" --db "$scratch/edge.db" smoke status \
    --ledger-epoch "$smoke_epoch" --pub-seq "$smoke_pub_seq" 2>/dev/null || true)
  query_output=$(docker compose -p "$project" -f "$repo_root/compose.dev.yaml" exec -T edge \
    iotkit-edge query --db /data/edge.db --limit 10 2>/dev/null || true)
  if jq -e --argjson pub_seq "$smoke_pub_seq" \
    '.status == "delivered" and .accepted_through >= $pub_seq' <<<"$status_output" >/dev/null 2>&1 \
    && grep -Fq "$smoke_test_id" <<<"$query_output"; then
    delivered=true
    break
  fi
  sleep 1
done

if [[ "$delivered" != true ]]; then
  docker compose -p "$project" -f "$repo_root/compose.dev.yaml" logs broker edge
  sed -n '1,240p' "$scratch/edge.log"
  echo "Edge MQTT vertical slice timed out" >&2
  exit 1
fi

kill -INT "$edge_pid"
wait "$edge_pid"
edge_pid=""

echo "Edge -> MQTT -> Edge -> accepted-through vertical slice: OK"
