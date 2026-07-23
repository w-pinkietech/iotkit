#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
# shellcheck disable=SC1091
source "$repo_root/deploy/mosquitto-image.env"

storage_profile=${IOTKIT_TEST_STORAGE_PROFILE:-embedded}
[[ "$storage_profile" == "embedded" || "$storage_profile" == "postgres" ]] || {
  echo "IOTKIT_TEST_STORAGE_PROFILE must be embedded or postgres" >&2
  exit 1
}
mkdir -p "$repo_root/target/tmp"
scratch=$(mktemp -d "$repo_root/target/tmp/rust-edge-output.XXXXXX")
broker="iotkit-rust-output-mqtt-$$"
postgres="iotkit-rust-output-postgres-$$"
broker_port=$((20000 + $$ % 20000))
test_pid=""

cleanup() {
  if [[ -n "$test_pid" ]] && kill -0 "$test_pid" 2>/dev/null; then
    kill "$test_pid" 2>/dev/null || true
    wait "$test_pid" 2>/dev/null || true
  fi
  docker rm -f "$broker" "$postgres" >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT

openssl rand -hex 24 >"$scratch/output-password"
chmod 600 "$scratch/output-password"
{
  printf 'edge-output:'
  tr -d '\r\n' <"$scratch/output-password"
  printf '\n'
} >"$scratch/passwords"
chmod 600 "$scratch/passwords"
docker run --rm --user "$(id -u):$(id -g)" \
  -v "$scratch:/work" "$IOTKIT_MOSQUITTO_IMAGE" \
  mosquitto_passwd -U /work/passwords
cat >"$scratch/acl" <<'EOF'
user edge-output
topic write iotkit/v1/sources/+/signals/+/observations
EOF
cat >"$scratch/mosquitto.conf" <<'EOF'
listener 1883 0.0.0.0
allow_anonymous false
password_file /mosquitto/config/passwords
acl_file /mosquitto/config/acl
persistence true
persistence_location /mosquitto/data/
log_type all
EOF
mkdir "$scratch/mqtt-data"
chmod 600 "$scratch/passwords"
chmod 644 "$scratch/acl" "$scratch/mosquitto.conf"
docker run --detach --name "$broker" \
  --user "$(id -u):$(id -g)" \
  --publish "127.0.0.1:$broker_port:1883" \
  --volume "$scratch/mosquitto.conf:/mosquitto/config/mosquitto.conf:ro" \
  --volume "$scratch/passwords:/mosquitto/config/passwords:ro" \
  --volume "$scratch/acl:/mosquitto/config/acl:ro" \
  --volume "$scratch/mqtt-data:/mosquitto/data" \
  "$IOTKIT_MOSQUITTO_IMAGE" >/dev/null

rust_storage_env=()
if [[ "$storage_profile" == "postgres" ]]; then
  docker run --rm --detach --name "$postgres" \
    --env POSTGRES_DB=iotkit \
    --env POSTGRES_USER=iotkit \
    --env POSTGRES_PASSWORD=iotkit-test-only \
    --publish 127.0.0.1::5432 \
    postgres:17-alpine@sha256:742f40ea20b9ff2ff31db5458d127452988a2164df9e17441e191f3b72252193 >/dev/null
  ready=false
  for _ in $(seq 1 120); do
    if docker logs "$postgres" 2>&1 |
        grep -Fq 'PostgreSQL init process complete; ready for start up.' &&
      docker exec "$postgres" \
        pg_isready --username iotkit --dbname iotkit >/dev/null 2>&1; then
      ready=true
      break
    fi
    sleep 0.25
  done
  [[ "$ready" == true ]] || {
    docker logs "$postgres" >&2
    exit 1
  }
  postgres_port=$(docker port "$postgres" 5432/tcp | head -1 | awk -F: '{print $NF}')
  docker exec "$postgres" createdb --username iotkit iotkit_contract
  rust_storage_env+=(
    IOTKIT_TEST_RUST_OUTPUT_POSTGRES_DSN="postgres://iotkit:iotkit-test-only@127.0.0.1:$postgres_port/iotkit?sslmode=disable"
  )
else
  rust_storage_env+=(
    IOTKIT_TEST_RUST_OUTPUT_SQLITE="$scratch/rust-edge.db"
  )
fi

(
  cd "$repo_root"
  env \
    "${rust_storage_env[@]}" \
    IOTKIT_REQUIRE_RUST_OUTPUT_GATE=1 \
    IOTKIT_TEST_OUTPUT_BROKER_HOST=127.0.0.1 \
    IOTKIT_TEST_OUTPUT_BROKER_PORT="$broker_port" \
    IOTKIT_TEST_OUTPUT_PASSWORD="$(tr -d '\r\n' <"$scratch/output-password")" \
    IOTKIT_TEST_OUTPUT_CONTROL_DIR="$scratch" \
    TMPDIR="$repo_root/target/tmp" \
    cargo test -p iotkit-edge --test output_puback \
      actual_mosquitto_outage_retries_same_durable_export_until_puback \
      -- --ignored --exact --nocapture
) >"$scratch/output.log" 2>&1 &
test_pid=$!

wait_for_marker() {
  local marker=$1
  for _ in $(seq 1 300); do
    [[ -f "$scratch/$marker" ]] && return 0
    if ! kill -0 "$test_pid" 2>/dev/null; then
      cat "$scratch/output.log" >&2
      echo "Rust output test exited before marker: $marker" >&2
      return 1
    fi
    sleep 0.1
  done
  cat "$scratch/output.log" >&2
  echo "timed out waiting for Rust output marker: $marker" >&2
  return 1
}

wait_for_marker ready
docker stop --time 5 "$broker" >/dev/null
printf 'down\n' >"$scratch/broker-down"
wait_for_marker pending
docker start "$broker" >/dev/null
if ! wait "$test_pid"; then
  test_pid=""
  cat "$scratch/output.log" >&2
  docker logs "$broker" >&2 || true
  exit 1
fi
test_pid=""

cd "$repo_root"
TMPDIR="$repo_root/target/tmp" \
  cargo test -p iotkit-edge --test output_contract
if [[ "$storage_profile" == "postgres" ]]; then
  IOTKIT_TEST_RUST_OUTPUT_POSTGRES_CONTRACT_DSN="postgres://iotkit:iotkit-test-only@127.0.0.1:$postgres_port/iotkit_contract?sslmode=disable" \
  TMPDIR="$repo_root/target/tmp" \
    cargo test -p iotkit-edge --test output_contract \
      postgres_failed_routes_retry_fairly_and_converge_after_storage_restart \
      -- --ignored --exact --nocapture
fi

echo "Rust Edge Output Adapter ($storage_profile) outage -> reconnect -> MQTT PUBACK gate passed."
