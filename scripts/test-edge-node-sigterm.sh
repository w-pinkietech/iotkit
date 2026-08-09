#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
project="iotkit-edge-node-sigterm-$$"
scratch=$(mktemp -d)
# Must match deploy/compose.edge-node-sigterm.yaml.
shutdown_grace_ms=15000
edge_node_id=""
ledger_epoch=""
early_password_writer=""
early_password_ready="$scratch/early-password-ready"
early_password_release="$scratch/early-password-release"

for command in docker mkfifo openssl sqlite3; do
  command -v "$command" >/dev/null || {
    echo "required command not found: $command" >&2
    exit 1
  }
done
docker compose version >/dev/null

export IOTKIT_SIGTERM_STATE="$scratch"
export IOTKIT_SIGTERM_UID="$(id -u)"
export IOTKIT_SIGTERM_GID="$(id -g)"
mkdir -p "$scratch/node" "$scratch/secrets"
chmod 700 "$scratch" "$scratch/secrets"
chmod 755 "$scratch/node"
mqtt_password=$(openssl rand -hex 24)

compose=(
  docker compose -p "$project"
  -f "$repo_root/deploy/compose.edge-node-sigterm.yaml"
)

diagnostics() {
  "${compose[@]}" ps --all >&2 || true
  "${compose[@]}" logs --no-color edge-node >&2 || true
}

cleanup() {
  status=$?
  if [[ -n "$early_password_writer" ]] && kill -0 "$early_password_writer" 2>/dev/null; then
    kill "$early_password_writer" 2>/dev/null || true
    wait "$early_password_writer" 2>/dev/null || true
  fi
  if ((status != 0)); then
    diagnostics
  fi
  "${compose[@]}" down --volumes --remove-orphans >/dev/null 2>&1 || true
  rm -rf "$scratch"
  return "$status"
}
trap cleanup EXIT

cat >"$scratch/edge.toml" <<EOF
[edge_node]
db_path = "/data/node.db"
health_json_path = "/data/health.json"

[adapters.bravepi]
enabled = false

[adapters.rpi_local]
enabled = false

[api]
enabled = false

[exit.mqtt]
enabled = true
host = "192.0.2.1"
port = 1883
password_file = "/run/secrets/node-mqtt-password"
allow_insecure = true
EOF
chmod 600 "$scratch/edge.toml"

start_edge_node() {
  "${compose[@]}" up --build --detach edge-node
  local ready=false
  for _ in $(seq 1 120); do
    edge_node_id=$(sqlite3 "$scratch/node/node.db" \
      "SELECT value FROM ledger_meta WHERE key='edge_node_id'" 2>/dev/null || true)
    ledger_epoch=$(sqlite3 "$scratch/node/node.db" \
      "SELECT value FROM ledger_meta WHERE key='epoch'" 2>/dev/null || true)
    target_count=$(sqlite3 "$scratch/node/node.db" \
      "SELECT count(*) FROM target_registry WHERE target_id='edge'" 2>/dev/null || true)
    if [[ -n "$edge_node_id" && -n "$ledger_epoch" && "$target_count" == "1" ]]; then
      ready=true
      break
    fi
    sleep 0.25
  done
  if [[ "$ready" != true ]]; then
    echo "Edge Node did not initialize its durable MQTT target" >&2
    return 1
  fi

  container_id=$("${compose[@]}" ps -q edge-node)
  [[ -n "$container_id" ]]
  entrypoint=$(docker inspect --format '{{.Path}} {{range .Args}}{{.}} {{end}}' "$container_id")
  [[ "$entrypoint" == "/usr/local/bin/iotkit-edge-node --config /run/iotkit/edge.toml " ]] || {
    echo "Edge Node is not the container primary process: $entrypoint" >&2
    return 1
  }
}

send_sigterm_and_require_clean_exit() {
  local label=$1
  local container_id started_at
  container_id=$("${compose[@]}" ps -q edge-node)
  [[ -n "$container_id" ]]
  started_at=$(date +%s%3N)
  docker kill --signal SIGTERM "$container_id" >/dev/null
  wait_for_clean_exit "$label" "$container_id" "$started_at"
}

wait_for_clean_exit() {
  local label=$1
  local container_id=$2
  local started_at=$3
  local elapsed_ms running exit_code
  running=true
  while [[ "$running" == true ]]; do
    elapsed_ms=$(( $(date +%s%3N) - started_at ))
    if ((elapsed_ms >= shutdown_grace_ms)); then
      break
    fi
    running=$(docker inspect --format '{{.State.Running}}' "$container_id")
    [[ "$running" == true ]] && sleep 0.1
  done
  elapsed_ms=$(( $(date +%s%3N) - started_at ))
  if [[ "$running" == true ]]; then
    echo "$label SIGTERM shutdown exceeded ${shutdown_grace_ms}ms" >&2
    return 1
  fi
  exit_code=$(docker inspect --format '{{.State.ExitCode}}' "$container_id")
  if [[ "$exit_code" != "0" ]]; then
    echo "$label SIGTERM shutdown exit code = $exit_code, want 0" >&2
    return 1
  fi
  if ((elapsed_ms >= shutdown_grace_ms)); then
    echo "$label SIGTERM shutdown took ${elapsed_ms}ms, want < ${shutdown_grace_ms}ms" >&2
    return 1
  fi
  echo "$label SIGTERM shutdown: exit 0 in ${elapsed_ms}ms"
}

start_early_sigterm_probe() {
  local password_fifo="$scratch/secrets/node-mqtt-password"
  local container_id started_at target_count
  mkfifo "$password_fifo"
  (
    exec 3>"$password_fifo"
    : >"$early_password_ready"
    while [[ ! -e "$early_password_release" ]]; do
      sleep 0.05
    done
    printf '%s\n' "$mqtt_password" >&3 || true
    exec 3>&-
  ) &
  early_password_writer=$!

  "${compose[@]}" up --build --detach edge-node
  for _ in $(seq 1 120); do
    [[ -e "$early_password_ready" ]] && break
    sleep 0.05
  done
  [[ -e "$early_password_ready" ]] || {
    echo "Edge Node did not reach the early MQTT password read" >&2
    return 1
  }
  target_count=$(sqlite3 "$scratch/node/node.db" \
    "SELECT count(*) FROM target_registry WHERE target_id='edge'")
  [[ "$target_count" == "0" ]] || {
    echo "Edge Node mutated the MQTT target before the early SIGTERM probe" >&2
    return 1
  }

  container_id=$("${compose[@]}" ps -q edge-node)
  [[ -n "$container_id" ]]
  started_at=$(date +%s%3N)
  docker kill --signal SIGTERM "$container_id" >/dev/null
  : >"$early_password_release"
  wait "$early_password_writer" || true
  early_password_writer=""
  wait_for_clean_exit "early-start" "$container_id" "$started_at"

  rm "$password_fifo"
  printf '%s\n' "$mqtt_password" >"$password_fifo"
  chmod 600 "$password_fifo"
}

assert_durable_unacknowledged_state() {
  [[ "$(sqlite3 "$scratch/node/node.db" 'PRAGMA quick_check')" == "ok" ]]
  [[ "$(sqlite3 "$scratch/node/node.db" \
    "SELECT state FROM edge_node_activation WHERE singleton=1")" == "active" ]]
  [[ "$(sqlite3 "$scratch/node/node.db" \
    "SELECT cursor_pub_seq FROM target_registry WHERE target_id='edge'")" == "0" ]]
  [[ "$(sqlite3 "$scratch/node/node.db" \
    "SELECT count(*) FROM publication_log WHERE epoch='$ledger_epoch' AND pub_seq > 0")" == "1" ]]
}

start_early_sigterm_probe
start_edge_node
send_sigterm_and_require_clean_exit "initial"

activation_id="act-0123456789abcdef0123456789abcdef"
activated_at=1700000000000
request_json=$(printf '{"schema_version":1,"activation_id":"%s","edge_id":"edge-0123456789abcdef0123456789abcdef","edge_node_id":"%s","expected_ledger_epoch":"%s","grant_revision":1,"issued_at":%s}' \
  "$activation_id" "$edge_node_id" "$ledger_epoch" "$activated_at")
result_json=$(printf '{"schema_version":1,"activation_id":"%s","edge_id":"edge-0123456789abcdef0123456789abcdef","edge_node_id":"%s","ledger_epoch":"%s","status":"applied","discard_through_reading_seq":0,"first_publication_seq":1,"applied_at":%s}' \
  "$activation_id" "$edge_node_id" "$ledger_epoch" "$activated_at")
sqlite3 "$scratch/node/node.db" <<SQL
UPDATE edge_node_activation
SET state = 'active',
    edge_id = 'edge-0123456789abcdef0123456789abcdef',
    activation_id = '$activation_id',
    ledger_epoch = '$ledger_epoch',
    discard_through_reading_seq = 0,
    cleanup_through_reading_seq = 0,
    request_json = '$request_json',
    result_json = '$result_json',
    activated_at = $activated_at
WHERE singleton = 1 AND state = 'discovery_only';
INSERT INTO publication_log(pub_seq, epoch, kind, annotation_json, created_at)
VALUES (1, '$ledger_epoch', 'commissioning_smoke',
        json_object('test_id', 'smoke-0123456789abcdef0123456789abcdef'), 1700000000001);
SQL

start_edge_node
sleep 1
assert_durable_unacknowledged_state
send_sigterm_and_require_clean_exit "unacknowledged-outage"
start_edge_node
assert_durable_unacknowledged_state
send_sigterm_and_require_clean_exit "restart"

echo "Edge Node SIGTERM shutdown: OK (durable unacknowledged row retained)"
