#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

# shellcheck disable=SC1091
source "$repo_root/deploy/mosquitto-image.env"

mkdir -p "$repo_root/target/tmp"
scratch=$(mktemp -d "$repo_root/target/tmp/rust-edge-runtime.XXXXXX")
broker_name="iotkit-rust-edge-runtime-mqtt-$$"
postgres_name="iotkit-rust-edge-runtime-postgres-$$"
storage_profile=${IOTKIT_TEST_STORAGE_PROFILE:-embedded}

cleanup() {
  docker rm -f "$broker_name" "$postgres_name" >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT

[[ "$storage_profile" == "embedded" || "$storage_profile" == "postgres" ]] || {
  echo "IOTKIT_TEST_STORAGE_PROFILE must be embedded or postgres" >&2
  exit 1
}
command -v docker >/dev/null

cat >"$scratch/mosquitto.conf" <<'EOF'
listener 1883 0.0.0.0
allow_anonymous true
persistence false
EOF

docker run --rm -d --name "$broker_name" \
  -p 127.0.0.1::1883 \
  -v "$scratch/mosquitto.conf:/mosquitto/config/mosquitto.conf:ro" \
  "$IOTKIT_MOSQUITTO_IMAGE" >/dev/null
broker_port=$(docker port "$broker_name" 1883/tcp | head -1 | awk -F: '{print $NF}')
[[ "$broker_port" =~ ^[0-9]+$ ]]

test_environment=(
  IOTKIT_TEST_RUNTIME_MQTT_PORT="$broker_port"
  IOTKIT_TEST_RUNTIME_STORAGE_PROFILE="$storage_profile"
)
if [[ "$storage_profile" == "postgres" ]]; then
  docker run --rm -d --name "$postgres_name" \
    -e POSTGRES_USER=iotkit \
    -e POSTGRES_PASSWORD=iotkit-test-only \
    -e POSTGRES_DB=iotkit_runtime \
    -p 127.0.0.1::5432 \
    postgres:17-alpine@sha256:742f40ea20b9ff2ff31db5458d127452988a2164df9e17441e191f3b72252193 \
    >/dev/null
  postgres_ready=false
  for _ in $(seq 1 120); do
    if docker logs "$postgres_name" 2>&1 |
        grep -Fq 'PostgreSQL init process complete; ready for start up.' &&
      docker exec "$postgres_name" \
        pg_isready -U iotkit -d iotkit_runtime >/dev/null 2>&1; then
      postgres_ready=true
      break
    fi
    sleep 0.25
  done
  [[ "$postgres_ready" == true ]] || {
    docker logs "$postgres_name" >&2
    echo "PostgreSQL did not become ready" >&2
    exit 1
  }
  postgres_port=$(docker port "$postgres_name" 5432/tcp | head -1 | awk -F: '{print $NF}')
  [[ "$postgres_port" =~ ^[0-9]+$ ]]
  test_environment+=(
    IOTKIT_TEST_RUNTIME_POSTGRES_DSN="postgres://iotkit:iotkit-test-only@127.0.0.1:$postgres_port/iotkit_runtime?sslmode=disable"
  )
fi

env "${test_environment[@]}" TMPDIR="$repo_root/target/tmp" \
  cargo test -p iotkit-edge --test runtime_composition_broker \
    composed_runtime_custodies_projects_serves_and_marks_output_puback \
    -- --ignored --exact --nocapture

echo "Rust Edge composed runtime gate passed ($storage_profile)."
