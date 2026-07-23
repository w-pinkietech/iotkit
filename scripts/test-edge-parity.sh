#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
group=${1:-surface}
report_dir=${IOTKIT_EDGE_PARITY_REPORT_DIR:-"$repo_root/target/edge-parity"}
steps_file="$report_dir/steps.txt"
result_file="$report_dir/result.json"

mkdir -p "$report_dir"
: >"$steps_file"

write_result() {
  local status=$1
  local exit_code=$2
  local head
  head=$(git -C "$repo_root" rev-parse HEAD)
  cat >"$result_file" <<EOF
{
  "schema_version": 1,
  "group": "$group",
  "status": "$status",
  "exit_code": $exit_code,
  "git_head": "$head",
  "steps_file": "steps.txt"
}
EOF
}

on_exit() {
  local exit_code=$?
  if ((exit_code != 0)); then
    write_result failed "$exit_code"
  fi
}
trap on_exit EXIT

run_step() {
  local name=$1
  shift
  printf '%s\n' "$name" >>"$steps_file"
  "$@"
}

run_surface() {
  run_step rust-package \
    node --test "$repo_root/scripts/tests/rust-edge-package.test.mjs"
  run_step manifest \
    cargo test --manifest-path "$repo_root/Cargo.toml" \
      -p iotkit-edge --test parity_manifest
  run_step go-cli-surface \
    bash -c 'cd "$1/edge" && go test ./cmd/iotkit-edge -run "$2" -count=1' \
      parity "$repo_root" '^TestRunUsageNamesIoTKitEdge$'
  run_step rust-cli-surface \
    cargo run --quiet --manifest-path "$repo_root/Cargo.toml" \
      -p iotkit-edge -- --help
}

case "$group" in
  surface)
    run_surface
    ;;
  all)
    run_surface
    run_step go-oracle \
      bash -c 'cd "$1/edge" && go test ./... -count=1' parity "$repo_root"
    run_step rust-edge \
      cargo test --manifest-path "$repo_root/Cargo.toml" -p iotkit-edge
    run_step output-adapter-api \
      cargo test --manifest-path "$repo_root/Cargo.toml" \
        -p iotkit-output-adapter-api -p iotkit-output-adapter-testkit
    run_step frontend \
      "$repo_root/scripts/test-edge-console-frontend.sh"
    run_step console-browser \
      "$repo_root/scripts/test-edge-console-e2e.sh"
    run_step mqtt-output \
      "$repo_root/scripts/test-edge-output.sh"
    ;;
  *)
    echo "usage: scripts/test-edge-parity.sh {surface|all}" >&2
    exit 2
    ;;
esac

write_result passed 0
trap - EXIT
echo "IoTKit Edge parity $group: PASS"
echo "Evidence: $result_file"
