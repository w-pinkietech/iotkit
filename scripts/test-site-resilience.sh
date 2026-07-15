#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
project="iotkit-resilience-test-$$"
scratch=$(mktemp -d)
edge_pid=""
edge_run=0
edge_node_id=""
ledger_epoch=""

for command in cargo docker openssl sqlite3; do
  command -v "$command" >/dev/null || {
    echo "required command not found: $command" >&2
    exit 1
  }
done
docker compose version >/dev/null

export IOTKIT_MOSQUITTO_PASSWORD_FILE="$scratch/passwords"
export IOTKIT_MOSQUITTO_ACL_FILE="$scratch/acl"
export IOTKIT_SITE_PASSWORD_FILE="$scratch/site-password"
export IOTKIT_SITE_DATA_DIR="$scratch/data"
export IOTKIT_DEV_UID="$(id -u)"
export IOTKIT_DEV_GID="$(id -g)"
mkdir -p "$IOTKIT_SITE_DATA_DIR"
chmod 700 "$scratch"
chmod 755 "$IOTKIT_SITE_DATA_DIR"

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
  "$repo_root/target/debug/iotkit-edge" --config "$scratch/edge.toml" \
    >"$scratch/edge-$edge_run.log" 2>&1 &
  edge_pid=$!
}

restart_edge() {
  stop_edge
  start_edge
}

edge_cursor() {
  sqlite3 "$scratch/edge.db" \
    "SELECT cursor_pub_seq FROM target_registry WHERE target_id='site'" \
    2>/dev/null || true
}

site_stats() {
  if [[ ! -f "$IOTKIT_SITE_DATA_DIR/site.db" ]]; then
    return 0
  fi
  sqlite3 "$IOTKIT_SITE_DATA_DIR/site.db" \
    "SELECT count(*), coalesce(min(pub_seq), 0), coalesce(max(pub_seq), 0), count(DISTINCT pub_seq)
     FROM raw_records
     WHERE edge_node_id = '$edge_node_id' AND ledger_epoch = '$ledger_epoch'" \
    2>/dev/null || true
}

diagnostics() {
  echo "edge_cursor=$(edge_cursor) site_stats=$(site_stats)" >&2
  compose ps >&2 || true
  compose logs --no-color broker site >&2 || true
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
    if [[ "$(edge_cursor)" == "$expected" && "$(site_stats)" == "$expected_stats" ]]; then
      return 0
    fi
    sleep 0.5
  done
  diagnostics
  echo "convergence timed out at pub_seq $expected" >&2
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
  pub_seq, epoch, kind, subtype, annotation_json, created_at
)
SELECT
  value,
  '$ledger_epoch',
  'annotation',
  printf('resilience_%06d', value),
  '{"prior_epoch":"resilience-prior"}',
  1700000000000 + value
FROM n;
SQL
}

openssl rand -hex 24 >"$IOTKIT_SITE_PASSWORD_FILE"
openssl rand -hex 24 >"$scratch/edge-password"
chmod 600 "$IOTKIT_SITE_PASSWORD_FILE" "$scratch/edge-password"

cargo build --manifest-path "$repo_root/Cargo.toml" -p iotkit-edge --bin iotkit-edge

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

start_edge
for _ in $(seq 1 100); do
  edge_node_id=$(sqlite3 "$scratch/edge.db" \
    "SELECT value FROM ledger_meta WHERE key='edge_node_id'" 2>/dev/null || true)
  ledger_epoch=$(sqlite3 "$scratch/edge.db" \
    "SELECT value FROM ledger_meta WHERE key='epoch'" 2>/dev/null || true)
  target_count=$(sqlite3 "$scratch/edge.db" \
    "SELECT count(*) FROM target_registry WHERE target_id='site'" 2>/dev/null || true)
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
  echo "Edge identity or Site target was not initialized" >&2
  exit 1
fi

cat >"$IOTKIT_MOSQUITTO_ACL_FILE" <<EOF
user $edge_node_id
topic write iotkit/v1/edge-nodes/$edge_node_id/records
topic write iotkit/v1/edge-nodes/$edge_node_id/descriptors
topic read iotkit/v1/edge-nodes/$edge_node_id/accepted-through

user site
topic read iotkit/v1/edge-nodes/+/records
topic read iotkit/v1/edge-nodes/+/descriptors
topic write iotkit/v1/edge-nodes/+/accepted-through
EOF
chmod 644 "$IOTKIT_MOSQUITTO_ACL_FILE"

printf 'site:' >"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
tr -d '\n' <"$IOTKIT_SITE_PASSWORD_FILE" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
printf '\n%s:' "$edge_node_id" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
tr -d '\n' <"$scratch/edge-password" >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
printf '\n' >>"$IOTKIT_MOSQUITTO_PASSWORD_FILE"
chmod 600 "$IOTKIT_MOSQUITTO_PASSWORD_FILE"

docker run --rm --user "$(id -u):$(id -g)" \
  -v "$scratch:/work" \
  eclipse-mosquitto:2.0 \
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

compose up --build --detach broker site
wait_for_convergence 300

# The Broker remains available, but transport receipt cannot replace Site custody.
compose stop site
seed_range 301 301
restart_edge
sleep 2
assert_cursor 300
compose start site
restart_edge
wait_for_convergence 301

# Edge restart by itself.
seed_range 302 302
restart_edge
wait_for_convergence 302

# Broker restart by itself while Edge and Site retain their databases.
compose stop broker
seed_range 303 303
compose start broker
wait_for_convergence 303

# Site restart by itself. Edge stays alive and uses its normal bounded retry.
compose restart site
seed_range 304 304
wait_for_convergence 304

stop_edge
compose stop site broker

edge_check=$(sqlite3 "$scratch/edge.db" 'PRAGMA quick_check')
site_check=$(sqlite3 "$IOTKIT_SITE_DATA_DIR/site.db" 'PRAGMA quick_check')
if [[ "$edge_check" != "ok" || "$site_check" != "ok" ]]; then
  diagnostics
  echo "SQLite quick_check failed: Edge=$edge_check Site=$site_check" >&2
  exit 1
fi
assert_cursor 304
if [[ "$(site_stats)" != "304|1|304|304" ]]; then
  diagnostics
  echo "Site records are not one contiguous 1..304 prefix" >&2
  exit 1
fi

echo "Edge/Broker/Site resilience matrix: OK (304 contiguous records)"
