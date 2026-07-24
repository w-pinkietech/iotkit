#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
report_dir=${1:-"$repo_root/.artifacts/edge-capacity"}
mkdir -p "$report_dir"
chmod 700 "$report_dir"

mkdir -p "$repo_root/target/tmp"
scratch=$(mktemp -d "$repo_root/target/tmp/rust-edge-capacity.XXXXXX")
cleanup() {
  rm -rf "$scratch"
}
trap cleanup EXIT
export TMPDIR="${TMPDIR:-$repo_root/target/tmp}"

cd "$repo_root"
IOTKIT_TEST_CAPACITY_PROFILE=embedded \
IOTKIT_TEST_CAPACITY_SQLITE="$scratch/embedded.db" \
IOTKIT_TEST_CAPACITY_BACKUP="$scratch/embedded.iotkit-backup" \
IOTKIT_CAPACITY_REPORT="$report_dir/embedded.json" \
  cargo test -p iotkit-edge --test capacity_regression \
    capacity_regression_smoke_emits_existing_evidence_schema \
    -- --ignored --exact --nocapture

cargo test -p iotkit-edge --test diagnostics_contract

"$repo_root/scripts/test-edge-postgres.sh" \
  capacity "$report_dir/postgres.json"

report_contract='
  .regression_smoke_passed == true
  and (.edge_nodes == 4)
  and (.sensors_per_edge == 8)
  and (.records == 8000)
  and (.payload_bytes > 0)
  and (.records_per_second > 0)
  and (.accept_p99_millis >= 0)
  and (.history_query_millis >= 0)
  and (.backup_millis >= 0)
  and (.database_bytes > 0)
  and (.pending_output == 0)
  and (.projection_failures == 0)
'
jq -e "$report_contract and .profile == \"embedded\"" \
  "$report_dir/embedded.json" >/dev/null
jq -e "$report_contract and .profile == \"postgres\"" \
  "$report_dir/postgres.json" >/dev/null
chmod 600 "$report_dir/embedded.json" "$report_dir/postgres.json"

echo "Edge storage capacity regression reports: $report_dir"
