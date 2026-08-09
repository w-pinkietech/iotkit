#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cargo_target_dir=${CARGO_TARGET_DIR:-"$repo_root/target"}
# shellcheck disable=SC1091
source "$repo_root/deploy/mosquitto-image.env"
export IOTKIT_MOSQUITTO_IMAGE
project="iotkit-resilience-test-$$"
broker_port=$((20000 + $$ % 20000))
scratch=$(mktemp -d)
edge_pid=""
edge_run=0
edge_node_id=""
ledger_epoch=""

for command in cargo docker jq openssl sqlite3; do
  command -v "$command" >/dev/null || {
    echo "required command not found: $command" >&2
    exit 1
  }
done
docker compose version >/dev/null

export IOTKIT_MOSQUITTO_PASSWORD_FILE="$scratch/passwords"
export IOTKIT_MOSQUITTO_ACL_FILE="$scratch/acl"
export IOTKIT_EDGE_ARCHIVE_PASSWORD_FILE="$scratch/edge-password"
export IOTKIT_EDGE_DATA_DIR="$scratch/data"
export IOTKIT_RECOVERY_DIR="$scratch/recovery"
export IOTKIT_DEV_UID="$(id -u)"
export IOTKIT_DEV_GID="$(id -g)"
export IOTKIT_DEV_BROKER_PORT="$broker_port"
export IOTKIT_EDGE_ID="edge-0123456789abcdef0123456789abcdef"
mkdir -p "$IOTKIT_EDGE_DATA_DIR" "$IOTKIT_RECOVERY_DIR"
chmod 700 "$scratch"
chmod 755 "$IOTKIT_EDGE_DATA_DIR"
chmod 700 "$IOTKIT_RECOVERY_DIR"

compose() {
  docker compose -p "$project" -f "$repo_root/compose.dev.yaml" "$@"
}

stop_edge() {
  if [[ -n "$edge_pid" ]] && kill -0 "$edge_pid" 2>/dev/null; then
    kill -INT "$edge_pid" 2>/dev/null || true
    wait "$edge_pid" 2>/dev/null || true
  fi
  edge_pid=""
}

cleanup() {
  stop_edge
  compose down --volumes --remove-orphans >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT

start_edge() {
  edge_run=$((edge_run + 1))
  "$cargo_target_dir/debug/iotkit-edge-node" --config "$scratch/edge.toml" \
    >"$scratch/edge-$edge_run.log" 2>&1 &
  edge_pid=$!
}

restart_edge() {
  stop_edge
  start_edge
}

edge_cursor() {
  sqlite3 "$scratch/edge.db" \
    "SELECT cursor_pub_seq FROM target_registry WHERE target_id='edge'" \
    2>/dev/null || true
}

edge_stats() {
  if [[ ! -f "$IOTKIT_EDGE_DATA_DIR/edge.db" ]]; then
    return 0
  fi
  sqlite3 "$IOTKIT_EDGE_DATA_DIR/edge.db" \
    "SELECT count(*), coalesce(min(pub_seq), 0), coalesce(max(pub_seq), 0), count(DISTINCT pub_seq)
     FROM raw_records
     WHERE edge_node_id = '$edge_node_id' AND ledger_epoch = '$ledger_epoch'" \
    2>/dev/null || true
}

edge_node_live_status() {
  if [[ ! -f "$IOTKIT_EDGE_DATA_DIR/edge.db" ]]; then
    return 0
  fi
  sqlite3 -separator '|' "$IOTKIT_EDGE_DATA_DIR/edge.db" \
    "SELECT boot_id, status_seq
     FROM edge_node_status
     WHERE edge_node_id = '$edge_node_id'
       AND ledger_epoch = '$ledger_epoch'
       AND last_live_received_at IS NOT NULL" \
    2>/dev/null || true
}

edge_node_live_status_detail() {
  if [[ ! -f "$IOTKIT_EDGE_DATA_DIR/edge.db" ]]; then
    return 0
  fi
  sqlite3 -separator '|' "$IOTKIT_EDGE_DATA_DIR/edge.db" \
    "SELECT boot_id,status_seq,collector_state,json_array_length(CAST(adapters_json AS TEXT))
     FROM edge_node_status
     WHERE edge_node_id = '$edge_node_id'
       AND ledger_epoch = '$ledger_epoch'
       AND last_live_received_at IS NOT NULL" \
    2>/dev/null || true
}

diagnostics() {
  echo "edge_cursor=$(edge_cursor) edge_stats=$(edge_stats)" >&2
  compose ps >&2 || true
  compose logs --no-color broker edge >&2 || true
  for log in "$scratch"/edge-*.log; do
    if [[ -f "$log" ]]; then
      echo "== $log ==" >&2
      sed -n '1,240p' "$log" >&2
    fi
  done
}

wait_for_convergence() {
  local expected=$1
  local expected_stats="$expected|1|$expected|$expected"
  for _ in $(seq 1 180); do
    if [[ "$(edge_cursor)" == "$expected" && "$(edge_stats)" == "$expected_stats" ]]; then
      return 0
    fi
    sleep 0.5
  done
  diagnostics
  echo "convergence timed out at pub_seq $expected" >&2
  return 1
}

wait_for_convergence_within_five_seconds() {
  local expected=$1
  local expected_stats="$expected|1|$expected|$expected"
  for _ in $(seq 1 20); do
    if [[ "$(edge_cursor)" == "$expected" && "$(edge_stats)" == "$expected_stats" ]]; then
      return 0
    fi
    sleep 0.25
  done
  diagnostics
  echo "convergence did not complete within five seconds at pub_seq $expected" >&2
  return 1
}

wait_for_live_status() {
  for _ in $(seq 1 20); do
    if [[ -n "$(edge_node_live_status)" ]]; then
      return 0
    fi
    sleep 0.25
  done
  diagnostics
  echo "Edge Node did not publish a live status immediately after MQTT readiness" >&2
  return 1
}

wait_for_next_status_heartbeat() {
  local boot_id=$1 status_seq=$2 current_boot_id current_status_seq
  for _ in $(seq 1 80); do
    IFS='|' read -r current_boot_id current_status_seq <<<"$(edge_node_live_status)"
    if [[ "$current_boot_id" == "$boot_id" && "$current_status_seq" -gt "$status_seq" ]]; then
      return 0
    fi
    sleep 0.5
  done
  diagnostics
  echo "Edge Node did not publish its 30-second status heartbeat" >&2
  return 1
}

assert_cursor() {
  local expected=$1
  local actual
  actual=$(edge_cursor)
  if [[ "$actual" != "$expected" ]]; then
    diagnostics
    echo "edge cursor = $actual, want $expected" >&2
    return 1
  fi
}

seed_range() {
  local first=$1
  local last=$2
  sqlite3 "$scratch/edge.db" <<SQL
PRAGMA busy_timeout = 5000;
WITH RECURSIVE n(value) AS (
  SELECT $first
  UNION ALL
  SELECT value + 1 FROM n WHERE value < $last
)
INSERT INTO publication_log(
  pub_seq, epoch, kind, annotation_json, created_at
)
SELECT
  value,
  '$ledger_epoch',
  'commissioning_smoke',
  json_object('test_id', printf('smoke-%032x', value)),
  1700000000000 + value
FROM n;
SQL
}

openssl rand -hex 24 >"$IOTKIT_EDGE_ARCHIVE_PASSWORD_FILE"
openssl rand -hex 24 >"$scratch/edge-password"
chmod 600 "$IOTKIT_EDGE_ARCHIVE_PASSWORD_FILE" "$scratch/edge-password"

cargo build --manifest-path "$repo_root/Cargo.toml" -p iotkit-edge-node --bin iotkit-edge-node

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

start_edge
for _ in $(seq 1 100); do
  edge_node_id=$(sqlite3 "$scratch/edge.db" \
    "SELECT value FROM ledger_meta WHERE key='edge_node_id'" 2>/dev/null || true)
  ledger_epoch=$(sqlite3 "$scratch/edge.db" \
    "SELECT value FROM ledger_meta WHERE key='epoch'" 2>/dev/null || true)
  target_count=$(sqlite3 "$scratch/edge.db" \
    "SELECT count(*) FROM target_registry WHERE target_id='edge'" 2>/dev/null || true)
  if [[ -n "$edge_node_id" && -n "$ledger_epoch" && "$target_count" == "1" ]]; then
    break
  fi
  if ! kill -0 "$edge_pid" 2>/dev/null; then
    diagnostics
    echo "Edge database initialization failed" >&2
    exit 1
  fi
  sleep 0.1
done
stop_edge
if [[ -z "$edge_node_id" || -z "$ledger_epoch" || "$target_count" != "1" ]]; then
  diagnostics
  echo "Edge identity or Edge target was not initialized" >&2
  exit 1
fi

# The activation handshake itself is covered by test-edge-mqtt.sh. This test
# starts from an already activated Edge Node so it can focus on transport and
# custody convergence across process restarts.
activation_id="act-0123456789abcdef0123456789abcdef"
edge_id="$IOTKIT_EDGE_ID"
activated_at=1700000000000
request_json=$(jq -cn \
  --arg activation_id "$activation_id" \
  --arg edge_id "$edge_id" \
  --arg edge_node_id "$edge_node_id" \
  --arg ledger_epoch "$ledger_epoch" \
  --argjson issued_at "$activated_at" \
  '{schema_version:1, activation_id:$activation_id, edge_id:$edge_id,
    edge_node_id:$edge_node_id, expected_ledger_epoch:$ledger_epoch,
    grant_revision:1, issued_at:$issued_at}')
result_json=$(jq -cn \
  --arg activation_id "$activation_id" \
  --arg edge_id "$edge_id" \
  --arg edge_node_id "$edge_node_id" \
  --arg ledger_epoch "$ledger_epoch" \
  --argjson applied_at "$activated_at" \
  '{schema_version:1, activation_id:$activation_id, edge_id:$edge_id,
    edge_node_id:$edge_node_id, ledger_epoch:$ledger_epoch, status:"applied",
    discard_through_reading_seq:0, first_publication_seq:1, applied_at:$applied_at}')
sqlite3 "$scratch/edge.db" <<SQL
UPDATE edge_node_activation
SET state = 'active',
    edge_id = '$edge_id',
    activation_id = '$activation_id',
    ledger_epoch = '$ledger_epoch',
    discard_through_reading_seq = 0,
    cleanup_through_reading_seq = 0,
    request_json = '$request_json',
    result_json = '$result_json',
    activated_at = $activated_at
WHERE singleton = 1 AND state = 'discovery_only';
SQL
[[ "$(sqlite3 "$scratch/edge.db" \
  'SELECT state FROM edge_node_activation WHERE singleton = 1')" == "active" ]]

cat >"$IOTKIT_MOSQUITTO_ACL_FILE" <<EOF
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

user edge
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
EOF
chmod 644 "$IOTKIT_MOSQUITTO_ACL_FILE"

printf 'edge:' >"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
tr -d '\n' <"$IOTKIT_EDGE_ARCHIVE_PASSWORD_FILE" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
printf '\n%s:' "$edge_node_id" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
tr -d '\n' <"$scratch/edge-password" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
printf '\n' >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
chmod 600 "$IOTKIT_MOSQUITTO_PASSWORD_FILE"

docker run --rm --user "$(id -u):$(id -g)" \
  -v "$scratch:/work" \
  "$IOTKIT_MOSQUITTO_IMAGE" \
  mosquitto_passwd -U /work/passwords
chmod 644 "$IOTKIT_MOSQUITTO_PASSWORD_FILE"

seed_range 1 300
outbox_count=$(sqlite3 "$scratch/edge.db" \
  "SELECT count(*) FROM publication_log WHERE epoch='$ledger_epoch'")
if [[ "$outbox_count" != "300" ]]; then
  echo "outbox count = $outbox_count, want 300" >&2
  exit 1
fi

start_edge
sleep 1
restart_edge
sleep 1
assert_cursor 0
outbox_count=$(sqlite3 "$scratch/edge.db" \
  "SELECT count(*) FROM publication_log WHERE epoch='$ledger_epoch' AND pub_seq > 0")
if [[ "$outbox_count" != "300" ]]; then
  echo "outbox count after Edge restart = $outbox_count, want 300" >&2
  exit 1
fi

compose up --build --detach broker edge

# Complete the central side of the already-activated fixture after the retained
# descriptor has created its Edge Node inventory row.
central_activation_ready=false
for _ in $(seq 1 60); do
  if [[ "$(sqlite3 "$IOTKIT_EDGE_DATA_DIR/edge.db" \
    "SELECT count(*) FROM edge_node_activations
     WHERE edge_node_id='$edge_node_id' AND ledger_epoch='$ledger_epoch'" \
     2>/dev/null || true)" == "1" ]]; then
    sqlite3 "$IOTKIT_EDGE_DATA_DIR/edge.db" \
      "UPDATE edge_node_activations SET state='active'
       WHERE edge_node_id='$edge_node_id' AND ledger_epoch='$ledger_epoch';"
    central_activation_ready=true
    break
  fi
  sleep 0.5
done
if [[ "$central_activation_ready" != true ]]; then
  diagnostics
  echo "Edge did not discover the Edge Node descriptor" >&2
  exit 1
fi
restart_edge
wait_for_live_status
IFS='|' read -r status_boot_id status_seq <<<"$(edge_node_live_status)"
IFS='|' read -r first_boot_id first_status_seq first_collector_state first_adapter_count \
  <<<"$(edge_node_live_status_detail)"
if [[ "$first_boot_id" != "$status_boot_id" || "$first_status_seq" != "1" || \
  "$first_collector_state" != "running" || "$first_adapter_count" != "0" ]]; then
  diagnostics
  echo "first accepted status did not follow collector initialization: $first_boot_id|$first_status_seq|$first_collector_state|$first_adapter_count" >&2
  exit 1
fi
wait_for_next_status_heartbeat "$status_boot_id" "$status_seq"
wait_for_convergence 300

# Establish a freshly connected, acknowledged idle state. The marker proves the
# restarted Edge Node is subscribed; after its acknowledgement there is no
# inflight batch when the next row is inserted.
restart_edge
seed_range 301 301
wait_for_convergence 301
sleep 1

# Keep Edge Node, Broker, and IoTKit Edge connected. A row arriving while the
# publisher is idle must not wait for the 30-second inflight retry cadence.
seed_range 302 302
wait_for_convergence_within_five_seconds 302

# The Broker remains available, but transport receipt cannot replace Edge custody.
compose stop edge
seed_range 303 303
restart_edge
sleep 2
assert_cursor 302
compose start edge
restart_edge
wait_for_convergence 303

# Edge restart by itself.
seed_range 304 304
restart_edge
wait_for_convergence 304

# Broker restart by itself while Edge and Edge retain their databases.
compose stop broker
seed_range 305 305
compose start broker
wait_for_convergence 305

# Edge restart by itself. Edge stays alive and uses its normal bounded retry.
compose restart edge
seed_range 306 306
wait_for_convergence 306

stop_edge
compose stop edge broker

edge_node_check=$(sqlite3 "$scratch/edge.db" 'PRAGMA quick_check')
central_edge_check=$(sqlite3 "$IOTKIT_EDGE_DATA_DIR/edge.db" 'PRAGMA quick_check')
if [[ "$edge_node_check" != "ok" || "$central_edge_check" != "ok" ]]; then
  diagnostics
  echo "SQLite quick_check failed: Edge Node=$edge_node_check Edge=$central_edge_check" >&2
  exit 1
fi
assert_cursor 306
if [[ "$(edge_stats)" != "306|1|306|306" ]]; then
  diagnostics
  echo "Edge records are not one contiguous 1..306 prefix" >&2
  exit 1
fi

echo "Edge/Broker/Edge resilience matrix: OK (306 contiguous records)"
