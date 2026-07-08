#!/usr/bin/env bash
# codex.sh — cross-vendor codex dispatch wrapper.
#
# Single source of truth for the codex model / flags / sandbox mapping.
# Skills and prompts reference THIS script instead of hardcoding a model string,
# so a model bump happens in one place (no stale `gpt-5.4` scattered in docs).
#
# Usage:
#   scripts/codex.sh <mode> <prompt-file> <label>
#     mode = review  -> sandbox read-only          (adversarial review; never mutates)
#     mode = impl    -> sandbox danger-full-access  (implementation; codex self-tests build/test/clippy)
#
# Output goes to $CODEX_OUT_DIR/codex-<label>-<mode>.txt (path printed on stderr).
#
# Env overrides:
#   CODEX_MODEL   (default gpt-5.5)
#   CODEX_EFFORT  (default high; bump to xhigh for deep reviews)
#   CODEX_OUT_DIR (default /tmp/codex-runs; set to the session scratchpad if you prefer)
#   CODEX_BIN     (default /home/kenta/.local/bin/codex)
set -euo pipefail

MODE="${1:-}"; PROMPT="${2:-}"; LABEL="${3:-}"
if [ -z "$MODE" ] || [ -z "$PROMPT" ] || [ -z "$LABEL" ]; then
  echo "usage: scripts/codex.sh <review|impl> <prompt-file> <label>" >&2
  exit 2
fi

CODEX_BIN="${CODEX_BIN:-/home/kenta/.local/bin/codex}"
CODEX_MODEL="${CODEX_MODEL:-gpt-5.5}"
CODEX_EFFORT="${CODEX_EFFORT:-high}"
CODEX_OUT_DIR="${CODEX_OUT_DIR:-/tmp/codex-runs}"
REPO="$(git rev-parse --show-toplevel)"

case "$MODE" in
  review) SANDBOX="read-only" ;;
  impl)   SANDBOX="danger-full-access" ;;
  *) echo "mode must be 'review' or 'impl', got: '$MODE'" >&2; exit 2 ;;
esac

[ -f "$PROMPT" ] || { echo "prompt file not found: $PROMPT" >&2; exit 2; }
mkdir -p "$CODEX_OUT_DIR"
OUT="$CODEX_OUT_DIR/codex-${LABEL}-${MODE}.txt"

{
  echo "→ codex ${MODE} (sandbox=${SANDBOX}) model=${CODEX_MODEL} effort=${CODEX_EFFORT}"
  echo "  prompt = ${PROMPT}"
  echo "  out    = ${OUT}"
} >&2

"$CODEX_BIN" exec \
  -s "$SANDBOX" \
  --skip-git-repo-check \
  -C "$REPO" \
  -m "$CODEX_MODEL" \
  -c "model_reasoning_effort=${CODEX_EFFORT}" \
  -o "$OUT" \
  "$(cat "$PROMPT")"

echo "✔ codex ${MODE} done → ${OUT}" >&2
