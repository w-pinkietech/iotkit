#!/usr/bin/env bash
# codex.sh — cross-vendor codex dispatch wrapper.
#
# Single source of truth for the codex model / flags / sandbox mapping.
# Skills and prompts reference THIS script instead of hardcoding a model string,
# so a model bump happens in one place (no stale model constant scattered in docs).
#
# Usage:
#   scripts/codex.sh <mode> <prompt-file> <label>
#     mode = review  -> sandbox read-only          (adversarial review; never mutates)
#     mode = impl    -> sandbox danger-full-access  (implementation; codex self-tests build/test/clippy)
#
# The prompt is fed on STDIN (not argv) so prompts may start with '-' and exceed
# the 128KiB single-argument limit (large plan/spec pastes).
#
# Output goes to $CODEX_OUT_DIR/codex-<label>-<mode>-<timestamp>.txt (path printed on
# stderr). Timestamped so a re-review never clobbers earlier evidence.
#
# Env overrides:
#   CODEX_MODEL   (default gpt-5.5)
#   CODEX_EFFORT  (override; default xhigh for review, high for impl — reviews decide
#                  things and earn max reasoning; the impl grind trades some for speed)
#   CODEX_OUT_DIR (default /tmp/codex-runs; set to the session scratchpad if you prefer)
#   CODEX_BIN     (default /home/kenta/.local/bin/codex)
#   CODEX_REPO    (default: current git toplevel; set to point codex at a different
#                  checkout, e.g. the design corpus for codex-design-review)
set -euo pipefail

MODE="${1:-}"; PROMPT="${2:-}"; LABEL="${3:-}"
if [ -z "$MODE" ] || [ -z "$PROMPT" ] || [ -z "$LABEL" ]; then
  echo "usage: scripts/codex.sh <review|impl> <prompt-file> <label>" >&2
  exit 2
fi

CODEX_BIN="${CODEX_BIN:-/home/kenta/.local/bin/codex}"
CODEX_MODEL="${CODEX_MODEL:-gpt-5.5}"
CODEX_OUT_DIR="${CODEX_OUT_DIR:-/tmp/codex-runs}"
REPO="${CODEX_REPO:-$(git rev-parse --show-toplevel)}"
[ -d "$REPO" ] || { echo "repo not found: $REPO" >&2; exit 2; }

case "$MODE" in
  review) SANDBOX="read-only";          DEFAULT_EFFORT="xhigh" ;;
  impl)   SANDBOX="danger-full-access"; DEFAULT_EFFORT="high"  ;;
  *) echo "mode must be 'review' or 'impl', got: '$MODE'" >&2; exit 2 ;;
esac
CODEX_EFFORT="${CODEX_EFFORT:-$DEFAULT_EFFORT}"

[ -s "$PROMPT" ] || { echo "prompt file missing or empty: $PROMPT" >&2; exit 2; }
mkdir -p "$CODEX_OUT_DIR"
OUT="$CODEX_OUT_DIR/codex-${LABEL}-${MODE}-$(date +%Y%m%d-%H%M%S)-$$.txt"

{
  echo "→ codex ${MODE} (sandbox=${SANDBOX}) model=${CODEX_MODEL} effort=${CODEX_EFFORT}"
  echo "  repo   = ${REPO}"
  echo "  prompt = ${PROMPT}"
  echo "  out    = ${OUT}"
} >&2

# Prompt on stdin ('-'); flags before it.
"$CODEX_BIN" exec \
  -s "$SANDBOX" \
  --skip-git-repo-check \
  -C "$REPO" \
  -m "$CODEX_MODEL" \
  -c "model_reasoning_effort=${CODEX_EFFORT}" \
  -o "$OUT" \
  - < "$PROMPT"

echo "✔ codex ${MODE} done → ${OUT}" >&2
