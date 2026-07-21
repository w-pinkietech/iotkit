#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
report_dir=${1:-"$repo_root/.artifacts/edge-capacity"}
mkdir -p "$report_dir"
chmod 700 "$report_dir"

export GOTMPDIR="${GOTMPDIR:-$repo_root/../.tmp/go-build}"
export GOCACHE="${GOCACHE:-$repo_root/../.cache/iotkit-edge-go-cache}"
export TMPDIR="${TMPDIR:-$repo_root/../.tmp/runtime}"
mkdir -p "$GOTMPDIR" "$GOCACHE" "$TMPDIR"

cd "$repo_root/iotkit-edge"
IOTKIT_CAPACITY_REPORT="$report_dir/embedded.json" \
  go test ./internal/store -run '^TestStorageCapacityRegressionSmoke$' -count=1

cd "$repo_root"
IOTKIT_CAPACITY_REPORT="$report_dir/postgres.json" \
  "$repo_root/scripts/test-edge-postgres.sh" '^TestStorageCapacityRegressionSmoke$'

jq -e '.regression_smoke_passed == true and .profile == "embedded"' \
  "$report_dir/embedded.json" >/dev/null
jq -e '.regression_smoke_passed == true and .profile == "postgres"' \
  "$report_dir/postgres.json" >/dev/null

echo "Edge storage capacity regression reports: $report_dir"
