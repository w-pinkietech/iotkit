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

network_args=(--publish 127.0.0.1::5432)
if [[ -n "${IOTKIT_TEST_POSTGRES_NETWORK_CONTAINER:-}" ]]; then
  network_args=(--network "container:${IOTKIT_TEST_POSTGRES_NETWORK_CONTAINER}")
fi
docker run --rm --detach --name "$container" \
  --env POSTGRES_DB=iotkit \
  --env POSTGRES_USER=iotkit \
  --env POSTGRES_PASSWORD=iotkit-test-only \
  "${network_args[@]}" \
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

if [[ -n "${IOTKIT_TEST_POSTGRES_NETWORK_CONTAINER:-}" ]]; then
  port=5432
else
  port=$(docker port "$container" 5432/tcp | head -1 | awk -F: '{print $NF}')
fi
[[ "$port" =~ ^[0-9]+$ ]]
export IOTKIT_TEST_POSTGRES_DSN="postgres://iotkit:iotkit-test-only@127.0.0.1:${port}/iotkit?sslmode=disable"
mkdir -p "$repo_root/target/tmp"
export TMPDIR="${TMPDIR:-$repo_root/target/tmp}"

reset_database() {
  docker exec "$container" \
    dropdb --force --if-exists --username iotkit iotkit >/dev/null
  docker exec "$container" createdb --username iotkit iotkit
}

cd "$repo_root"
if [[ "$mode" == "capacity" ]]; then
  scratch=$(mktemp -d "$repo_root/target/tmp/rust-edge-postgres-capacity.XXXXXX")
  trap 'rm -rf "$scratch"; cleanup' EXIT
  IOTKIT_TEST_CAPACITY_PROFILE=postgres \
  IOTKIT_TEST_CAPACITY_BACKUP="$scratch/postgres.iotkit-backup" \
  IOTKIT_CAPACITY_REPORT="$report_path" \
    cargo test -p iotkit-edge --test capacity_regression \
      capacity_regression_profile_emits_semantic_backlog_evidence \
      -- --ignored --exact --nocapture
  exit 0
fi

IOTKIT_REQUIRE_POSTGRES=1 \
  cargo test -p iotkit-edge --test storage_contract \
    postgres_obeys_the_same_raw_custody_contract_when_configured \
    -- --ignored --exact --nocapture

reset_database
cargo test -p iotkit-edge --test history_storage_contract \
  postgres_semantic_history_series_obeys_the_shared_contract \
  -- --ignored --exact --nocapture

reset_database
IOTKIT_REQUIRE_POSTGRES=1 \
  cargo test -p iotkit-edge --test semantic_projection_queue_contract \
    postgres_candidate_plan_uses_the_bounded_pending_queue_lookup \
    -- --ignored --exact --nocapture

reset_database
IOTKIT_REQUIRE_POSTGRES=1 \
  cargo test -p iotkit-edge --lib \
    storage::semantic_output::postgres_tests::ready_rule_plan_uses_indexes_and_sorts_only_rule_heads \
    -- --ignored --exact --nocapture

reset_database
IOTKIT_REQUIRE_POSTGRES=1 \
  cargo test -p iotkit-edge --test semantic_projection_queue_contract \
    postgres_first_unseen_epoch_accept_and_rule_creation_serialize_at_the_edge_lock \
    -- --ignored --exact --nocapture

reset_database
IOTKIT_REQUIRE_POSTGRES=1 \
  cargo test -p iotkit-edge --test semantic_projection_queue_contract \
    postgres_multiple_pending_resets_fence_a_new_epoch_until_each_boundary_is_applied \
    -- --ignored --exact --nocapture

reset_database
cargo test -p iotkit-edge --test auth_storage_contract \
  postgres_obeys_account_session_and_admin_safety_contract \
  -- --ignored --exact --nocapture

reset_database
cargo test -p iotkit-edge --test web_application_contract \
  postgres_enforces_the_same_web_revision_precondition \
  -- --ignored --exact --nocapture

reset_database
IOTKIT_REQUIRE_POSTGRES=1 \
  cargo test -p iotkit-edge --test schema_upgrade_contract \
    postgres_startup_upgrades_a_v6_database_without_losing_identity \
    -- --ignored --exact --nocapture

reset_database
IOTKIT_REQUIRE_POSTGRES=1 \
  cargo test -p iotkit-edge --test schema_upgrade_contract \
    postgres_startup_upgrades_v8_with_noncontiguous_receipts_and_snapshots_each_pending_pair \
    -- --ignored --exact --nocapture

reset_database
IOTKIT_REQUIRE_POSTGRES=1 \
  cargo test -p iotkit-edge --test recovery_activation \
    postgres_recovery_freezes_old_admission_and_replays_exactly \
    -- --ignored --exact --nocapture

reset_database
IOTKIT_REQUIRE_POSTGRES=1 \
  cargo test -p iotkit-edge --test cli_parity_contract \
    postgres_migration_copies_and_verifies_a_fresh_rust_schema_when_configured \
    -- --exact --nocapture

reset_database
IOTKIT_REQUIRE_POSTGRES=1 \
  cargo test -p iotkit-edge --test cli_parity_contract \
    postgres_migration_failure_rolls_back_every_copied_row_when_configured \
    -- --exact --nocapture

reset_database
IOTKIT_REQUIRE_POSTGRES=1 \
  cargo test -p iotkit-edge --test backup_contract \
    postgres_restored_gap_requires_audited_archive_loss_acceptance \
    -- --ignored --exact --nocapture

reset_database
docker exec "$container" dropdb --force --if-exists \
  --username iotkit iotkit_restore >/dev/null
docker exec "$container" createdb --username iotkit iotkit_restore
export IOTKIT_TEST_POSTGRES_RESTORE_DSN="postgres://iotkit:iotkit-test-only@127.0.0.1:${port}/iotkit_restore?sslmode=disable"
export IOTKIT_REQUIRE_POSTGRES=1
cargo test -p iotkit-edge --test backup_contract \
  postgres_custom_snapshot_round_trips_through_real_tools_when_required \
  -- --exact --nocapture

echo "Rust Edge PostgreSQL custody, semantic history, semantic projection, auth, revision, upgrade, migration, recovery, backup, and restore tests passed."
