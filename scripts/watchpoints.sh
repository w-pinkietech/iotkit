#!/usr/bin/env bash
# watchpoints.sh — flag Active Watchpoints past their revalidate-by date.
#
# The Maintenance rule ("expired watchpoints are deleted unless explicitly
# renewed", docs/eval/*-review.md) rotted in practice because nothing checked
# the dates mechanically — six watchpoints sat 11 days past expiry unnoticed
# (found 2026-07-08). This script is the mechanical eye. Run it before
# authoring ANY eval prompt (codex-eval-common) or a curator pass
# (eval-perspectives-curator); adjudicate anything it lists before injecting
# the guides into a prompt.
#
# Fails closed: a malformed date or missing guide files is an error, never a
# silent pass — a typo must not close the eye this script exists to be.
#
# Exit: 0 = nothing expired, 1 = expired/malformed lines listed, 2 = guides missing.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

compgen -G "docs/eval/*-review.md" >/dev/null \
  || { echo "review guides not found under docs/eval/ (wrong repo?)" >&2; exit 2; }

today="$(date +%F)"
bad=0

# Every "Watchpoint:" entry must carry a "Revalidate by:" line — an entry
# whose date line was forgotten would otherwise be invisible here forever.
for f in docs/eval/*-review.md; do
  w="$(grep -c "Watchpoint:" "$f" || true)"
  r="$(grep -c "Revalidate by:" "$f" || true)"
  if [ "$w" -ne "$r" ]; then
    echo "MALFORMED: $f has $w 'Watchpoint:' but $r 'Revalidate by:' lines (entry missing its date?)"
    bad=1
  fi
done

# "Revalidate by: YYYY-MM-DD" is the one true watchpoint date format
# (eval-perspectives-curator). ISO dates compare correctly as strings; the
# date(1) round-trip rejects shape-legal but impossible dates (2026-13-45).
# -H forces the file:line prefix even when only one file matches.
while IFS= read -r hit; do
  d="${hit##*Revalidate by: }"
  d="${d%%[[:space:]]*}"
  if [[ ! "$d" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] \
     || [ "$(date -d "$d" +%F 2>/dev/null || true)" != "$d" ]; then
    echo "MALFORMED date ('$d'): ${hit%%:  *}"
    bad=1
  elif [[ "$d" < "$today" ]]; then
    echo "EXPIRED ($d): ${hit%%:  *}"
    bad=1
  fi
done < <(grep -rnH "Revalidate by:" docs/eval/*-review.md)
# (zero grep matches = no watchpoints anywhere = legitimately clean; the
#  process-substitution exit status is intentionally not propagated)

if [ "$bad" -eq 0 ]; then
  echo "✔ no expired watchpoints (today: $today)"
fi
exit "$bad"
