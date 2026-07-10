#!/usr/bin/env bash
# verify.sh — host verification after a codex impl task.
#
# Test-green is NECESSARY, not SUFFICIENT (see CLAUDE.md 「検証と実行の規律」):
# data-loss / concurrency regressions / spec drift slip past green tests.
# Always pair this with independent cross-vendor review — never treat a green
# verify.sh as "done".
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

echo "✔ verify.sh PASS (green — now run independent review before calling it done)"
