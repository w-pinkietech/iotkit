#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
if [[ -n "${YOKAKIT_REPO:-}" ]]; then
  yokakit_root=$YOKAKIT_REPO
else
  git_common_dir=$(git -C "$repo_root" rev-parse --path-format=absolute --git-common-dir)
  main_checkout=$(dirname "$git_common_dir")
  yokakit_root="$(dirname "$main_checkout")/yokakit-redesign"
fi
iotkit_fixture="$repo_root/testdata/output/v1/yokakit-production.json"
yokakit_fixture="$yokakit_root/testdata/mqtt/iotkit-yokakit-production.json"

[[ -f "$yokakit_root/go.mod" ]] || {
  echo "YokaKit repository not found: $yokakit_root" >&2
  exit 1
}
cmp "$iotkit_fixture" "$yokakit_fixture" || {
  echo "IoTKit and YokaKit contract fixtures differ" >&2
  exit 1
}

(
  cd "$yokakit_root"
  env GOCACHE="${GOCACHE:-/tmp/yokakit-contract-go-build}" \
    go test ./internal/adapters/inbound/mqttingest \
      -run '^TestIoTKitYokaKitProductionFixtureIsAccepted$' -count=1
)

echo "IoTKit YokaKit Adapter -> YokaKit decoder contract: OK"
