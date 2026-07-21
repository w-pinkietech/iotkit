#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
container="iotkit-edge-postgres-test-$$"
test_pattern="${1:-^(TestPostgres|TestCreateInitialEdgeAccountIsAtomicAndAudited|TestSemanticV3ProjectionRunsCounterAndAlarmIndependently|TestOutputBindingPreviewMatchesDurableOutboxPublication|TestQueryHistoryFiltersAndUsesStableCursor|TestQueryHistorySeriesAggregatesBoundedBuckets)}"

cleanup() {
  docker rm -f "$container" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker run --detach --name "$container" \
  --env POSTGRES_DB=iotkit \
  --env POSTGRES_USER=iotkit \
  --env POSTGRES_PASSWORD=iotkit-test-only \
  --publish 127.0.0.1::5432 \
  postgres:17-alpine >/dev/null

ready=false
for _ in $(seq 1 60); do
  if docker exec "$container" pg_isready --username iotkit --dbname iotkit >/dev/null 2>&1; then
	ready=true
    break
  fi
  sleep 0.25
done
if [[ "$ready" != true ]]; then
  docker logs "$container"
  exit 1
fi

port=$(docker port "$container" 5432/tcp | sed 's/.*://')
export IOTKIT_TEST_POSTGRES_DSN="postgres://iotkit:iotkit-test-only@127.0.0.1:${port}/iotkit?sslmode=disable"
export GOTMPDIR="${GOTMPDIR:-$repo_root/../.tmp/go-build}"
export GOCACHE="${GOCACHE:-$repo_root/../.cache/iotkit-edge-go-cache}"
mkdir -p "$GOTMPDIR" "$GOCACHE"

cd "$repo_root/iotkit-edge"
go test ./internal/store -run "$test_pattern" -count=1
