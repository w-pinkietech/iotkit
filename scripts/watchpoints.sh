#!/usr/bin/env bash
# watchpoints.sh — flag Active Watchpoints past their revalidate-by date.
#
# The Maintenance rule ("expired watchpoints are deleted unless explicitly
# renewed", docs/eval/*-review.md) rotted in practice because nothing checked
# the dates mechanically — six watchpoints sat 11 days past expiry unnoticed
# (found 2026-07-08). This script is the mechanical eye. Run it when preparing
# a review prompt (codex-impl-loop step 4) or a curator pass
# (eval-perspectives-curator); adjudicate anything it lists before injecting
# the guides into a prompt.
#
# Exit: 0 = nothing expired, 1 = expired watchpoints listed on stdout.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

today="$(date +%F)"
expired=0

# "Revalidate by: YYYY-MM-DD" is the one true watchpoint date format
# (eval-perspectives-curator). ISO dates compare correctly as strings.
while IFS= read -r hit; do
  d="${hit##*Revalidate by: }"
  d="${d:0:10}"
  if [[ "$d" < "$today" ]]; then
    echo "EXPIRED ($d): ${hit%%:  *}"
    expired=1
  fi
done < <(grep -rn "Revalidate by:" docs/eval/*-review.md || true)

if [ "$expired" -eq 0 ]; then
  echo "✔ no expired watchpoints (today: $today)"
fi
exit "$expired"
