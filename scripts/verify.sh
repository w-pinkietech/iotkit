#!/usr/bin/env bash
# verify.sh — opt-in Rust workspace diagnosis.
set -euo pipefail

[[ ${1:-} == "--workspace" && $# == 1 ]] || {
  echo "usage: scripts/verify.sh --workspace" >&2
  exit 2
}
cd "$(git rev-parse --show-toplevel)"

echo "== cargo fmt --all --check =="
cargo fmt --all --check

echo "== scripts/check-layers (crate layer rules) =="
scripts/check-layers

echo "== scripts/check-source-layout (source/test boundary) =="
scripts/check-source-layout

echo "== trial profile configuration contract =="
python3 -m unittest scripts.tests.test_iotkit_trial

echo "== cargo test --workspace =="
cargo test --workspace

echo "== cargo clippy --workspace --all-targets -- -D warnings =="
cargo clippy --workspace --all-targets -- -D warnings

echo "✔ verify.sh PASS"
