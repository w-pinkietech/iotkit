#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
# shellcheck disable=SC1091
source "$repo_root/deploy/mosquitto-image.env"

scratch=$(mktemp -d)
container="iotkit-output-test-$$"
broker_port=$((20000 + $$ % 20000))
test_pid=""

cleanup() {
  if [[ -n "$test_pid" ]] && kill -0 "$test_pid" 2>/dev/null; then
    kill "$test_pid" 2>/dev/null || true
    wait "$test_pid" 2>/dev/null || true
  fi
  docker rm --force "$container" >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT

for command in docker openssl sqlite3; do
  command -v "$command" >/dev/null || {
    echo "required command missing: $command" >&2
    exit 1
  }
done

mkdir -m 700 "$scratch/config" "$scratch/data" "$scratch/control"
openssl rand -hex 24 >"$scratch/config/output-password"
openssl rand -hex 24 >"$scratch/config/observer-password"
chmod 600 "$scratch/config/output-password" "$scratch/config/observer-password"

{
  printf 'site-output:'
  tr -d '\r\n' <"$scratch/config/output-password"
  printf '\nobserver:'
  tr -d '\r\n' <"$scratch/config/observer-password"
  printf '\n'
} >"$scratch/config/passwords"
chmod 600 "$scratch/config/passwords"

docker run --rm --user "$(id -u):$(id -g)" \
  -v "$scratch/config:/work" \
  "$IOTKIT_MOSQUITTO_IMAGE" \
  mosquitto_passwd -U /work/passwords
chmod 600 "$scratch/config/passwords"

cat >"$scratch/config/acl" <<'EOF'
user site-output
topic write factory/line-a/production
topic write yokakit/v1/sources/iotkit-01/signals/press-running/observations
topic write yokakit/v1/sources/iotkit-01/status

user observer
topic read factory/line-a/production
topic read yokakit/v1/sources/iotkit-01/signals/press-running/observations
topic read yokakit/v1/sources/iotkit-01/status
EOF
chmod 600 "$scratch/config/acl"

cat >"$scratch/config/mosquitto.conf" <<'EOF'
listener 1883 0.0.0.0
protocol mqtt
allow_anonymous false
password_file /mosquitto/config/passwords
acl_file /mosquitto/config/acl
persistence true
persistence_location /mosquitto/data/
max_inflight_messages 20
max_queued_messages 1000
log_dest stdout
EOF
chmod 644 "$scratch/config/mosquitto.conf"

docker run --detach --name "$container" \
  --user "$(id -u):$(id -g)" \
  -p "127.0.0.1:$broker_port:1883" \
  -v "$scratch/config:/mosquitto/config:ro" \
  -v "$scratch/data:/mosquitto/data" \
  "$IOTKIT_MOSQUITTO_IMAGE" >/dev/null

broker_url="tcp://127.0.0.1:$broker_port"

(
  cd "$repo_root/iotkit-site"
  env \
    GOCACHE="${GOCACHE:-/tmp/iotkit-go-build}" \
    IOTKIT_TEST_OUTPUT_BROKER_URL="$broker_url" \
    IOTKIT_TEST_OUTPUT_CONTROL_DIR="$scratch/control" \
    IOTKIT_TEST_OUTPUT_PASSWORD_FILE="$scratch/config/output-password" \
    IOTKIT_TEST_OUTPUT_OBSERVER_PASSWORD_FILE="$scratch/config/observer-password" \
    go test -tags=integration ./internal/mqttsite \
      -run '^TestMQTTOutputAdaptersConvergeAcrossBrokerRestart$' -count=1 -v
) >"$scratch/test.log" 2>&1 &
test_pid=$!

wait_for_marker() {
  local marker=$1
  for _ in $(seq 1 300); do
    [[ -f "$scratch/control/$marker" ]] && return 0
    if ! kill -0 "$test_pid" 2>/dev/null; then
      cat "$scratch/test.log" >&2
      echo "output integration test exited before marker: $marker" >&2
      return 1
    fi
    sleep 0.1
  done
  cat "$scratch/test.log" >&2
  echo "timed out waiting for output integration marker: $marker" >&2
  return 1
}

wait_for_marker ready
docker stop --time 5 "$container" >/dev/null
printf 'ok\n' >"$scratch/control/broker-down"

wait_for_marker pending
mapfile -t pending_ids < <(
  sqlite3 "$scratch/control/site.db" \
    "SELECT export_id FROM output_outbox_v3 WHERE published_at IS NULL ORDER BY export_id"
)
(( ${#pending_ids[@]} >= 2 )) || {
  cat "$scratch/test.log" >&2
  echo "Broker outage did not leave adapter exports pending" >&2
  exit 1
}

docker start "$container" >/dev/null
if ! wait "$test_pid"; then
  test_pid=""
  cat "$scratch/test.log" >&2
  docker logs "$container" >&2 || true
  exit 1
fi
test_pid=""

for export_id in "${pending_ids[@]}"; do
  published=$(sqlite3 "$scratch/control/site.db" \
    "SELECT count(*) FROM output_outbox_v3
     WHERE export_id='$export_id' AND published_at IS NOT NULL")
  [[ "$published" == "1" ]] || {
    cat "$scratch/test.log" >&2
    echo "pending export did not converge with the same identity: $export_id" >&2
    exit 1
  }
done

grep -Fq -- '--- PASS: TestMQTTOutputAdaptersConvergeAcrossBrokerRestart' \
  "$scratch/test.log"
echo "Site Output Adapter -> MQTT PUBACK -> restart convergence: OK"
