#!/usr/bin/env bash
# verify.sh — Rust product verification gate.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

full=false
if [[ ${1:-} == "--full" ]]; then
  full=true
  shift
fi
(($# == 0)) || { echo "usage: scripts/verify.sh [--full]" >&2; exit 2; }

echo "== cargo fmt --all --check =="
cargo fmt --all --check

echo "== scripts/check-layers (crate layer rules) =="
scripts/check-layers

echo "== scripts/check-source-layout (source/test boundary) =="
scripts/check-source-layout

echo "== cargo test --workspace =="
cargo test --workspace

echo "== cargo clippy --workspace --all-targets -- -D warnings =="
cargo clippy --workspace --all-targets -- -D warnings

if [[ "$full" == true ]]; then
  echo "== scripts/test-certificate-hostname.sh =="
  scripts/test-certificate-hostname.sh
  echo "== scripts/test-edge-node-recovery-acl.sh =="
  scripts/test-edge-node-recovery-acl.sh
  echo "== scripts/test-edge-node-fence.sh =="
  scripts/test-edge-node-fence.sh
  echo "== scripts/test-mqtt-security.sh =="
  scripts/test-mqtt-security.sh
  echo "== scripts/test-broker-cert.sh =="
  scripts/test-broker-cert.sh
  echo "== scripts/test-broker-cert-pebble.sh =="
  scripts/test-broker-cert-pebble.sh
  echo "== scripts/test-edge-mqtt.sh =="
  scripts/test-edge-mqtt.sh
  echo "== scripts/test-edge-output.sh (embedded) =="
  scripts/test-edge-output.sh
  echo "== scripts/test-edge-output.sh (PostgreSQL) =="
  IOTKIT_TEST_STORAGE_PROFILE=postgres scripts/test-edge-output.sh
  echo "== scripts/test-edge-resilience.sh =="
  scripts/test-edge-resilience.sh
  echo "== scripts/test-edge-bootstrap.sh =="
  scripts/test-edge-bootstrap.sh
  echo "== scripts/test-edge-postgres.sh =="
  scripts/test-edge-postgres.sh
  echo "== scripts/test-edge-capacity.sh =="
  scripts/test-edge-capacity.sh
fi

echo "✔ verify.sh PASS"
