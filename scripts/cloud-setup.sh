#!/usr/bin/env bash
# Codex Cloud environment setup/maintenance entrypoint.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

command -v rustup >/dev/null 2>&1 || {
  echo "rustup is required by the pinned rust-toolchain.toml" >&2
  exit 2
}
command -v cargo >/dev/null 2>&1 || {
  echo "cargo is required" >&2
  exit 2
}

# rustup honors rust-toolchain.toml and installs the exact pinned toolchain when needed.
rustup show active-toolchain
cargo fetch --locked

echo "Codex Cloud environment ready for scripts/verify.sh"
