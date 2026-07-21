#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
frontend_root="$repo_root/iotkit-edge/frontend"

test -f "$repo_root/iotkit-edge/openapi/edge-console-v1.yaml"
test -f "$frontend_root/package-lock.json"

npm --prefix "$frontend_root" run check
