#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_tmp_root=${IOTKIT_EDGE_E2E_TMPDIR:-${XDG_CACHE_HOME:-$HOME/.cache}/iotkit-edge-console-e2e}
mkdir -p "$test_tmp_root"
export TMPDIR="$test_tmp_root"
export IOTKIT_EDGE_E2E_TMPDIR="$test_tmp_root"

cd "$repo_root"
cargo test -p iotkit-edge \
  --test http_contract \
  --test console_contract \
  --test history_contract

storage_profile=${IOTKIT_TEST_STORAGE_PROFILE:-embedded}
[[ "$storage_profile" == "embedded" || "$storage_profile" == "postgres" ]] || {
  echo "IOTKIT_TEST_STORAGE_PROFILE must be embedded or postgres" >&2
  exit 1
}
postgres_container="iotkit-console-postgres-test-$$"
broker_container="iotkit-console-broker-test-$$"
postgres_dsn=""

cleanup() {
  status=$?
  if ((status != 0)) && [[ -n "${e2e_dir:-}" ]]; then
    if [[ -f "$e2e_dir/commissioning-fixture.log" ]]; then
      echo "== commissioning fixture log ==" >&2
      cat "$e2e_dir/commissioning-fixture.log" >&2
    fi
    if [[ -f "$e2e_dir/edge.log" ]]; then
      echo "== IoTKit Edge log ==" >&2
      cat "$e2e_dir/edge.log" >&2
    fi
    if [[ "$storage_profile" == "postgres" ]]; then
      echo "== PostgreSQL log ==" >&2
      docker logs "$postgres_container" >&2 || true
    fi
    echo "== Mosquitto log ==" >&2
    docker logs "$broker_container" >&2 || true
  fi
  if [[ -n "${commissioning_fixture_pid:-}" ]]; then
    kill "$commissioning_fixture_pid" >/dev/null 2>&1 || true
    wait "$commissioning_fixture_pid" >/dev/null 2>&1 || true
  fi
  if [[ -n "${edge_pid:-}" ]]; then
    kill "$edge_pid" >/dev/null 2>&1 || true
    wait "$edge_pid" >/dev/null 2>&1 || true
  fi
  docker rm --force "$postgres_container" >/dev/null 2>&1 || true
  docker rm --force "$broker_container" >/dev/null 2>&1 || true
  rm -rf "${e2e_dir:-}"
}
trap cleanup EXIT

if [[ "$storage_profile" == "postgres" ]]; then
  postgres_port=$(node -e '
    const { createServer } = require("node:net");
    const server = createServer();
    server.listen(0, "127.0.0.1", () => {
      console.log(server.address().port);
      server.close();
    });
  ')
  docker run --detach --name "$postgres_container" \
    --env POSTGRES_DB=iotkit \
    --env POSTGRES_USER=iotkit \
    --env POSTGRES_PASSWORD=iotkit-test-only \
    --publish "127.0.0.1:$postgres_port:5432" \
    postgres:17-alpine@sha256:742f40ea20b9ff2ff31db5458d127452988a2164df9e17441e191f3b72252193 \
    >/dev/null
  postgres_ready=false
  for _ in $(seq 1 60); do
    if docker exec "$postgres_container" \
      pg_isready --username iotkit --dbname iotkit >/dev/null 2>&1; then
      postgres_ready=true
      break
    fi
    sleep 0.25
  done
  [[ "$postgres_ready" == true ]] || {
    docker logs "$postgres_container" >&2
    echo "PostgreSQL did not become ready" >&2
    exit 1
  }
  postgres_dsn="postgres://iotkit:iotkit-test-only@127.0.0.1:$postgres_port/iotkit?sslmode=disable"
fi

e2e_dir=$(mktemp -d "$test_tmp_root/run.XXXXXX")
password_file="$e2e_dir/password"
printf '%s' '現場担当者の 十分に長いパスワード' >"$password_file"
chmod 600 "$password_file"
storage_args=(--storage-profile "$storage_profile")
if [[ "$storage_profile" == "postgres" ]]; then
  postgres_config="$e2e_dir/postgres.json"
  printf '{"dsn":"%s"}' "$postgres_dsn" >"$postgres_config"
  chmod 600 "$postgres_config"
  storage_args+=(--postgres-config "$postgres_config")
else
  storage_args+=(--db "$e2e_dir/edge.db")
fi

TMPDIR="$test_tmp_root" cargo build -p iotkit-edge --bin iotkit-edge
edge_binary="$repo_root/target/debug/iotkit-edge"
"$edge_binary" account bootstrap \
  "${storage_args[@]}" \
  --login-id owner \
  --display-name "第一工場 システム管理者" \
  --password-file "$password_file" >/dev/null
if [[ "$storage_profile" == "postgres" ]]; then
  fixture_location="$postgres_dsn"
else
  fixture_location="$e2e_dir/edge.db"
fi
fixture_edge_id=$(TMPDIR="$test_tmp_root" cargo run --quiet -p iotkit-edge \
  --example console_fixture -- "$storage_profile" "$fixture_location")

broker_port=$(node -e '
  const { createServer } = require("node:net");
  const server = createServer();
  server.listen(0, "127.0.0.1", () => {
    console.log(server.address().port);
    server.close();
  });
')
broker_dir="$e2e_dir/mosquitto"
mkdir -p "$broker_dir"
broker_username="iotkit-edge"
broker_password="e2e-broker-password"
printf '%s\n' \
  'listener 1883 0.0.0.0' \
  'allow_anonymous false' \
  'password_file /mosquitto/config/passwords' \
  'persistence false' >"$broker_dir/mosquitto.conf"
printf '%s\n%s\n' "$broker_password" "$broker_password" | docker run --rm --user "$(id -u):$(id -g)" --interactive \
  --volume "$broker_dir:/mosquitto/config" \
  eclipse-mosquitto:2.0.22 \
  mosquitto_passwd -c /mosquitto/config/passwords "$broker_username"
chmod 600 "$broker_dir/passwords"
docker run --detach --name "$broker_container" --user "$(id -u):$(id -g)" \
  --publish "127.0.0.1:$broker_port:1883" \
  --volume "$broker_dir:/mosquitto/config:ro" \
  eclipse-mosquitto:2.0.22 >/dev/null
broker_ready=false
for _ in $(seq 1 100); do
  if node -e "
    const socket = require('node:net').connect($broker_port, '127.0.0.1');
    socket.once('connect', () => { socket.destroy(); process.exit(0); });
    socket.once('error', () => process.exit(1));
  "; then
    broker_ready=true
    break
  fi
  sleep 0.05
done
[[ "$broker_ready" == true ]] || {
  docker logs "$broker_container" >&2
  echo "Mosquitto did not become ready" >&2
  exit 1
}
broker_password_file="$e2e_dir/broker-password"
printf '%s' "$broker_password" >"$broker_password_file"
chmod 600 "$broker_password_file"

port=$(node -e '
  const { createServer } = require("node:net");
  const server = createServer();
  server.listen(0, "127.0.0.1", () => {
    console.log(server.address().port);
    server.close();
  });
')
origin="http://127.0.0.1:$port"
"$edge_binary" serve \
  "${storage_args[@]}" \
  --edge-id "$fixture_edge_id" \
  --broker-url "tcp://127.0.0.1:$broker_port" \
  --username "$broker_username" \
  --password-file "$broker_password_file" \
  --allow-insecure \
  --http-listen "127.0.0.1:$port" \
  --public-origin "$origin" \
  --development-http \
  >"$e2e_dir/edge.log" 2>&1 &
edge_pid=$!

ready=false
for _ in $(seq 1 100); do
  if node -e "fetch('$origin/login').then(r => process.exit(r.ok ? 0 : 1)).catch(() => process.exit(1))"; then
    ready=true
    break
  fi
  sleep 0.05
done
if [[ "$ready" != true ]]; then
  cat "$e2e_dir/edge.log" >&2
  echo "Rust IoTKit Edge did not become ready" >&2
  exit 1
fi

TMPDIR="$test_tmp_root" cargo build -p iotkit-edge --example console_commissioning_fixture
commissioning_fixture="$repo_root/target/debug/examples/console_commissioning_fixture"
"$commissioning_fixture" \
  127.0.0.1 "$broker_port" "$broker_username" "$broker_password_file" \
  >"$e2e_dir/commissioning-fixture.log" 2>&1 &
commissioning_fixture_pid=$!

IOTKIT_EDGE_E2E_URL="$origin" \
  IOTKIT_EDGE_E2E_PASSWORD="$(<"$password_file")" \
  IOTKIT_TEST_STORAGE_PROFILE="$storage_profile" \
  node "$repo_root/edge/frontend/e2e/rust-console-journey.mjs"
