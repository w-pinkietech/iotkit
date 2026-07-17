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

echo "== cargo test --workspace =="
cargo test --workspace

echo "== cargo clippy --workspace --all-targets -- -D warnings =="
cargo clippy --workspace --all-targets -- -D warnings

echo "== go test ./... =="
(cd iotkit-site && go test ./...)

if [[ "$full" == true ]]; then
  echo "== scripts/test-mqtt-security.sh =="
  scripts/test-mqtt-security.sh
  echo "== scripts/test-broker-cert.sh =="
  scripts/test-broker-cert.sh
  echo "== scripts/test-broker-cert-pebble.sh =="
  scripts/test-broker-cert-pebble.sh
  echo "== scripts/test-site-mqtt.sh =="
  scripts/test-site-mqtt.sh
  echo "== scripts/test-site-resilience.sh =="
  scripts/test-site-resilience.sh
  echo "== scripts/test-site-bootstrap.sh =="
  scripts/test-site-bootstrap.sh
fi

echo "✔ verify.sh PASS"
