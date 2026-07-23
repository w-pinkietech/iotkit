#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

# shellcheck disable=SC1091
source "$repo_root/deploy/mosquitto-image.env"

mkdir -p "$repo_root/target/tmp"
scratch=$(mktemp -d "$repo_root/target/tmp/rust-edge-custody.XXXXXX")
postgres_name="iotkit-rust-edge-postgres-$$"
broker_name="iotkit-rust-edge-mosquitto-$$"

cleanup() {
  docker rm -f "$postgres_name" "$broker_name" >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT

cat >"$scratch/mosquitto.conf" <<'EOF'
listener 1883 0.0.0.0
allow_anonymous true
persistence false
EOF

docker run --rm -d --name "$postgres_name" \
  -e POSTGRES_USER=iotkit \
  -e POSTGRES_PASSWORD=iotkit-test-only \
  -e POSTGRES_DB=iotkit \
  -p 127.0.0.1::5432 \
  postgres:17-alpine@sha256:742f40ea20b9ff2ff31db5458d127452988a2164df9e17441e191f3b72252193 \
  >/dev/null
docker run --rm -d --name "$broker_name" \
  -p 127.0.0.1::1883 \
  -v "$scratch/mosquitto.conf:/mosquitto/config/mosquitto.conf:ro" \
  "$IOTKIT_MOSQUITTO_IMAGE" >/dev/null

for _ in $(seq 1 30); do
  if docker exec "$postgres_name" pg_isready -U iotkit -d iotkit >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
docker exec "$postgres_name" pg_isready -U iotkit -d iotkit >/dev/null

postgres_port=$(docker port "$postgres_name" 5432/tcp | head -1 | awk -F: '{print $NF}')
broker_port=$(docker port "$broker_name" 1883/tcp | head -1 | awk -F: '{print $NF}')
[[ "$postgres_port" =~ ^[0-9]+$ && "$broker_port" =~ ^[0-9]+$ ]]

export TMPDIR="$repo_root/target/tmp"
IOTKIT_REQUIRE_POSTGRES=1 \
IOTKIT_TEST_POSTGRES_DSN="postgres://iotkit:iotkit-test-only@127.0.0.1:$postgres_port/iotkit?sslmode=disable" \
  cargo test -p iotkit-edge --test storage_contract \
    postgres_obeys_the_same_raw_custody_contract_when_configured -- --ignored --nocapture

IOTKIT_TEST_MQTT_PORT="$broker_port" \
  cargo test -p iotkit-edge --test mqtt_runtime_broker -- --ignored --nocapture

echo "Rust Edge PostgreSQL and Mosquitto custody tests passed."
