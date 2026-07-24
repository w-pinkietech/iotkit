#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

echo "== Rust MQTT custody, malformed input, pre-activation rejection, activation, and ACK =="
"$repo_root/scripts/test-rust-edge-custody.sh"

echo "== Composed Rust runtime MQTT custody, projection, HTTP, and output PUBACK (embedded) =="
"$repo_root/scripts/test-rust-edge-runtime.sh"

echo "== Composed Rust runtime MQTT custody, projection, HTTP, and output PUBACK (PostgreSQL) =="
IOTKIT_TEST_STORAGE_PROFILE=postgres \
  "$repo_root/scripts/test-rust-edge-runtime.sh"

echo "Rust Edge MQTT operational gate passed."
