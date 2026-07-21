#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

cd "$repo_root/iotkit-edge"
IOTKIT_RUN_BROWSER_E2E=1 \
  go test ./internal/edgehttp \
    -run '^TestConsoleOperatorJourneyInBrowser$' \
    -count=1
