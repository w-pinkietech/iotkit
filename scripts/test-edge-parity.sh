#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
group=${1:-surface}

case "$group" in
  surface)
    node --test "$repo_root/scripts/tests/rust-edge-package.test.mjs"
    cargo test --manifest-path "$repo_root/Cargo.toml" \
      -p iotkit-edge --test parity_manifest
    (
      cd "$repo_root/edge"
      go test ./cmd/iotkit-edge -run '^TestRunUsageNamesIoTKitEdge$' -count=1
    )
    cargo run --quiet --manifest-path "$repo_root/Cargo.toml" \
      -p iotkit-edge -- --help >/dev/null
    ;;
  all)
    echo "full parity is unavailable until every manifest group has a Rust runner" >&2
    exit 2
    ;;
  *)
    echo "usage: scripts/test-edge-parity.sh {surface|all}" >&2
    exit 2
    ;;
esac

echo "IoTKit Edge parity $group: PASS"
