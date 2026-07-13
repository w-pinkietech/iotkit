#!/usr/bin/env bash
# verify.sh — Rust product verification gate.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

echo "== cargo fmt --all --check =="
cargo fmt --all --check

echo "== scripts/check-layers (crate layer rules) =="
scripts/check-layers

echo "== cargo test --workspace =="
cargo test --workspace

echo "== cargo clippy --workspace --all-targets -- -D warnings =="
cargo clippy --workspace --all-targets -- -D warnings

echo "✔ verify.sh PASS"
