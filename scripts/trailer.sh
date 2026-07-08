#!/usr/bin/env bash
# trailer.sh — standard commit trailer block.
#
# The Co-Authored-By line names the CURRENT assistant model (default: this
# session's model). Codex tasks also carry provenance (Implemented-by / Reviewed-by).
#
# Usage:
#   git commit -m "feat(crate): ..." -m "$(scripts/trailer.sh codex)"   # codex-implemented, cross-vendor reviewed
#   git commit -m "docs: ..."        -m "$(scripts/trailer.sh docs)"    # main-agent authored (docs/harness)
#
# Override the model when a different assistant authored the commit — the default
# below WILL drift as sessions switch models, so the committing agent must pass
# TRAILER_MODEL whenever its session model differs:
#   TRAILER_MODEL="Claude Opus 4.8" scripts/trailer.sh codex
set -euo pipefail
MODE="${1:-}"
[ -n "$MODE" ] || { echo "usage: scripts/trailer.sh <codex|docs>" >&2; exit 2; }
MODEL="${TRAILER_MODEL:-Claude Fable 5}"
MODEL="${MODEL//[$'\n\r']/ }"   # collapse newlines: a single-line display name, no trailer injection
case "$MODE" in
  codex)
    printf 'Implemented-by: codex\nReviewed-by: codex (read-only), Fable review-max\nCo-Authored-By: %s <noreply@anthropic.com>\n' "$MODEL" ;;
  docs)
    printf 'Co-Authored-By: %s <noreply@anthropic.com>\n' "$MODEL" ;;
  *) echo "usage: scripts/trailer.sh <codex|docs>" >&2; exit 2 ;;
esac
