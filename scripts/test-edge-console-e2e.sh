#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
storage_profile=${IOTKIT_TEST_STORAGE_PROFILE:-embedded}
[[ "$storage_profile" == "embedded" || "$storage_profile" == "postgres" ]] || {
  echo "IOTKIT_TEST_STORAGE_PROFILE must be embedded or postgres" >&2
  exit 1
}
postgres_container="iotkit-console-postgres-test-$$"
postgres_dsn=""

cleanup() {
  docker rm --force "$postgres_container" >/dev/null 2>&1 || true
}
trap cleanup EXIT

if [[ "$storage_profile" == "postgres" ]]; then
  postgres_port=$((20000 + $$ % 20000))
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

cd "$repo_root/iotkit-edge"
IOTKIT_RUN_BROWSER_E2E=1 \
  IOTKIT_TEST_CONSOLE_POSTGRES_DSN="$postgres_dsn" \
  go test ./internal/edgehttp \
    -run '^TestConsoleOperatorJourneyInBrowser$' \
    -count=1
