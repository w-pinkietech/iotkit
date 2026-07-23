#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
container="iotkit-edge-postgres-test-$$"
mode=${1:-contracts}
report_path=${2:-}
[[ "$mode" == "contracts" || "$mode" == "capacity" ]] || {
  echo "usage: scripts/test-edge-postgres.sh [capacity REPORT_PATH]" >&2
  exit 2
}
if [[ "$mode" == "capacity" && -z "$report_path" ]]; then
  echo "usage: scripts/test-edge-postgres.sh capacity REPORT_PATH" >&2
  exit 2
fi

cleanup() {
  docker rm -f "$container" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker run --rm --detach --name "$container" \
  --env POSTGRES_DB=iotkit \
  --env POSTGRES_USER=iotkit \
  --env POSTGRES_PASSWORD=iotkit-test-only \
  --publish 127.0.0.1::5432 \
  postgres:17-alpine@sha256:742f40ea20b9ff2ff31db5458d127452988a2164df9e17441e191f3b72252193 >/dev/null

ready=false
for _ in $(seq 1 120); do
  if docker logs "$container" 2>&1 |
      grep -Fq 'PostgreSQL init process complete; ready for start up.' &&
    docker exec "$container" \
      pg_isready --username iotkit --dbname iotkit >/dev/null 2>&1; then
    ready=true
    break
  fi
  sleep 0.25
done
[[ "$ready" == true ]] || {
  docker logs "$container" >&2
  echo "PostgreSQL did not become ready" >&2
  exit 1
}

port=$(docker port "$container" 5432/tcp | head -1 | awk -F: '{print $NF}')
[[ "$port" =~ ^[0-9]+$ ]]
export IOTKIT_TEST_POSTGRES_DSN="postgres://iotkit:iotkit-test-only@127.0.0.1:${port}/iotkit?sslmode=disable"
mkdir -p "$repo_root/target/tmp"
export TMPDIR="${TMPDIR:-$repo_root/target/tmp}"

cd "$repo_root"
if [[ "$mode" == "capacity" ]]; then
  scratch=$(mktemp -d "$repo_root/target/tmp/rust-edge-postgres-capacity.XXXXXX")
  trap 'rm -rf "$scratch"; cleanup' EXIT
  IOTKIT_TEST_CAPACITY_PROFILE=postgres \
  IOTKIT_TEST_CAPACITY_BACKUP="$scratch/postgres.iotkit-backup" \
  IOTKIT_CAPACITY_REPORT="$report_path" \
    cargo test -p iotkit-edge --test capacity_regression \
      capacity_regression_smoke_emits_existing_evidence_schema \
      -- --ignored --exact --nocapture
  exit 0
fi

IOTKIT_REQUIRE_POSTGRES=1 \
  cargo test -p iotkit-edge --test storage_contract \
    postgres_obeys_the_same_raw_custody_contract_when_configured \
    -- --ignored --exact --nocapture

cargo test -p iotkit-edge --test auth_storage_contract \
  postgres_obeys_account_session_and_admin_safety_contract \
  -- --ignored --exact --nocapture

cargo test -p iotkit-edge --test web_application_contract \
  postgres_enforces_the_same_web_revision_precondition \
  -- --ignored --exact --nocapture

docker exec "$container" dropdb --if-exists --username iotkit iotkit >/dev/null
docker exec "$container" createdb --username iotkit iotkit
docker exec "$container" createdb --username iotkit iotkit_restore
export IOTKIT_TEST_POSTGRES_RESTORE_DSN="postgres://iotkit:iotkit-test-only@127.0.0.1:${port}/iotkit_restore?sslmode=disable"
export IOTKIT_REQUIRE_POSTGRES=1
cargo test -p iotkit-edge --test backup_contract \
  postgres_custom_snapshot_round_trips_through_real_tools_when_required \
  -- --exact --nocapture

echo "Rust Edge PostgreSQL custody, auth, revision, backup, and restore tests passed."
