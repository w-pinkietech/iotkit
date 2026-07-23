#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
report_dir=${1:-}
if [[ -z "$report_dir" ]]; then
  echo "usage: scripts/test-edge-host-release-gate.sh NEW_REPORT_DIRECTORY" >&2
  exit 2
fi
[[ ! -e "$report_dir" ]] || {
  echo "report directory already exists: $report_dir" >&2
  exit 1
}
mkdir -m 700 -p "$report_dir"

export TMPDIR="${TMPDIR:-$repo_root/../.tmp/runtime}"
mkdir -p "$TMPDIR"

echo "== PostgreSQL clean install, HTTPS login, two Edge Nodes, activation, raw custody =="
IOTKIT_TEST_STORAGE_PROFILE=postgres "$repo_root/scripts/test-edge-bootstrap.sh"

echo "== PostgreSQL Console operator journey =="
IOTKIT_TEST_STORAGE_PROFILE=postgres "$repo_root/scripts/test-edge-console-e2e.sh"

echo "== PostgreSQL semantics, Output Adapters, MQTT outage convergence =="
IOTKIT_TEST_STORAGE_PROFILE=postgres "$repo_root/scripts/test-edge-output.sh"

echo "== PostgreSQL schema upgrade, encrypted backup, restore, negative cases =="
"$repo_root/scripts/test-edge-postgres.sh"

echo "== Edge Node, Broker, and Edge restart/outage convergence =="
"$repo_root/scripts/test-edge-resilience.sh"

echo "== MQTT authentication, ACL, and TLS negative matrix =="
"$repo_root/scripts/test-mqtt-security.sh"

echo "== Broker certificate install/rollback and ACME renewal =="
"$repo_root/scripts/test-broker-cert.sh"
"$repo_root/scripts/test-broker-cert-pebble.sh"

echo "== embedded/PostgreSQL capacity regression reports =="
"$repo_root/scripts/test-edge-capacity.sh" "$report_dir/capacity"

jq -n \
  --arg commit "$(git -C "$repo_root" rev-parse HEAD)" \
  --arg working_tree_diff_sha256 "$(git -C "$repo_root" diff --binary HEAD | sha256sum | cut -d' ' -f1)" \
  --arg completed_at "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
  --arg kernel "$(uname -srm)" \
  --argjson logical_cpu_count "$(getconf _NPROCESSORS_ONLN)" \
  '{schema_version:1, gate:"host-integration", result:"passed",
    commit:$commit, working_tree_diff_sha256:$working_tree_diff_sha256,
    completed_at:$completed_at, kernel:$kernel,
    logical_cpu_count:$logical_cpu_count}' >"$report_dir/host-gate.json"
chmod 600 "$report_dir/host-gate.json"

echo "IoTKit Edge host release gate: PASS ($report_dir)"
