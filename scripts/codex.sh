#!/usr/bin/env bash
# codex.sh — cross-vendor codex dispatch wrapper.
#
# Single source of truth for the codex model / flags / sandbox mapping.
# Skills and prompts reference THIS script instead of hardcoding a model string,
# so a model bump happens in one place (no stale model constant scattered in docs).
#
# Usage:
#   scripts/codex.sh <mode> <prompt-file> <label>
#     mode = review  -> sandbox read-only     (adversarial review; never mutates)
#     mode = impl    -> sandbox workspace-write (implementation; no host-wide writes)
#
# The prompt is fed on STDIN (not argv) so prompts may start with '-' and exceed
# the 128KiB single-argument limit (large plan/spec pastes).
#
# Output goes to $CODEX_OUT_DIR/codex-<label>-<mode>-<timestamp>.txt (path printed on
# stderr). Timestamped so a re-review never clobbers earlier evidence.
#
# Env overrides:
#   CODEX_MODEL   (override; review defaults to gpt-5.6-sol, ordinary impl to
#                  gpt-5.6-terra. Use gpt-5.6-luna explicitly for clear,
#                  repeatable mechanical work.)
#   CODEX_EFFORT  (override; default medium for normal review and impl. Plan 6
#                  and other high-risk work use Sol/high per the workflow canon.
#                  Effort scale: low < medium < high < xhigh < max; "ultra"
#                  also exists but fans out subagents — a different execution/cost
#                  mode, never a silent default; opt in explicitly via CODEX_EFFORT)
#   CODEX_OUT_DIR (default /tmp/codex-runs; set to the session scratchpad if you prefer)
#   CODEX_BIN     (default /home/kenta/.local/bin/codex)
#   CODEX_REPO    (default: current git toplevel; set to point codex at a different
#                  checkout, e.g. the design corpus for codex-design-review)
set -euo pipefail
umask 077
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

MODE="${1:-}"; PROMPT="${2:-}"; LABEL="${3:-}"
if [ -z "$MODE" ] || [ -z "$PROMPT" ] || [ -z "$LABEL" ]; then
  echo "usage: scripts/codex.sh <review|impl> <prompt-file> <label>" >&2
  exit 2
fi
[[ "$LABEL" =~ ^[A-Za-z0-9._-]{1,80}$ ]] || { echo "unsafe label" >&2; exit 2; }

CODEX_BIN="${CODEX_BIN:-/home/kenta/.local/bin/codex}"
CODEX_OUT_DIR="${CODEX_OUT_DIR:-/tmp/codex-runs}"
REPO="${CODEX_REPO:-$(git rev-parse --show-toplevel)}"
[ -d "$REPO" ] || { echo "repo not found: $REPO" >&2; exit 2; }

case "$MODE" in
  review) SANDBOX="read-only";       DEFAULT_MODEL="gpt-5.6-sol";   DEFAULT_EFFORT="medium" ;;
  impl)   SANDBOX="workspace-write"; DEFAULT_MODEL="gpt-5.6-terra"; DEFAULT_EFFORT="medium" ;;
  *) echo "mode must be 'review' or 'impl', got: '$MODE'" >&2; exit 2 ;;
esac
CODEX_MODEL="${CODEX_MODEL:-$DEFAULT_MODEL}"
CODEX_EFFORT="${CODEX_EFFORT:-$DEFAULT_EFFORT}"
case "$CODEX_EFFORT" in low|medium|high|xhigh|max) ;; *) echo "unsupported Codex effort" >&2; exit 2 ;; esac

[ -s "$PROMPT" ] || { echo "prompt file missing or empty: $PROMPT" >&2; exit 2; }
if [ "$MODE" = review ] && [ -z "${REVIEW_MANIFEST:-}" ]; then echo "REVIEW_MANIFEST is required for review" >&2; exit 2; fi
if [ -n "${REVIEW_MANIFEST:-}" ]; then
  REVIEW_MANIFEST="$(readlink -f "$REVIEW_MANIFEST")"
  export REVIEW_MANIFEST
  (cd "$REPO" && "$SCRIPT_DIR/review-manifest.sh" --verify "$REVIEW_MANIFEST")
  MANIFEST_SHA256="$(sha256sum -- "$REVIEW_MANIFEST" | cut -d' ' -f1)"
fi
mkdir -p "$CODEX_OUT_DIR"
chmod 700 "$CODEX_OUT_DIR"
[ -O "$CODEX_OUT_DIR" ] || { echo "output directory is not owned by current user" >&2; exit 2; }
OUT="$CODEX_OUT_DIR/codex-${LABEL}-${MODE}-$(date +%Y%m%d-%H%M%S)-$$.txt"
OUT_PARTIAL="$OUT.partial"
PROMPT_SHA256="$(sha256sum -- "$PROMPT" | cut -d' ' -f1)"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

{
  echo "→ codex ${MODE} (sandbox=${SANDBOX}) model=${CODEX_MODEL} effort=${CODEX_EFFORT}"
  echo "  repo   = ${REPO}"
  echo "  prompt = ${PROMPT}"
  echo "  out    = ${OUT}"
} >&2

# Prompt on stdin ('-'); flags before it. Never expose a partial/empty result as
# review evidence.
trap 'rm -f "$OUT_PARTIAL"' EXIT
"$CODEX_BIN" exec \
  -s "$SANDBOX" \
  --skip-git-repo-check \
  -C "$REPO" \
  -m "$CODEX_MODEL" \
  -c "model_reasoning_effort=${CODEX_EFFORT}" \
  -o "$OUT_PARTIAL" \
  - < "$PROMPT"

[ -s "$OUT_PARTIAL" ] || { echo "x empty codex output — treat as FAILED" >&2; exit 1; }
mv "$OUT_PARTIAL" "$OUT"
trap 'rm -f "$OUT_PARTIAL" "$OUT" "$OUT.receipt" "$OUT.receipt.partial"' EXIT
[ "$(sha256sum -- "$PROMPT" | cut -d' ' -f1)" = "$PROMPT_SHA256" ] || { echo "prompt changed during review" >&2; exit 1; }
[ "$MODE" != review ] || (cd "$REPO" && "$SCRIPT_DIR/review-manifest.sh" --verify "$REVIEW_MANIFEST")
[ "$MODE" != review ] || [ "$(sha256sum -- "$REVIEW_MANIFEST" | cut -d' ' -f1)" = "$MANIFEST_SHA256" ] || { echo "manifest changed during review" >&2; exit 1; }
"$SCRIPT_DIR/review-receipt.sh" codex "$MODE" "$CODEX_MODEL" "$CODEX_EFFORT" \
  "$PROMPT" "$OUT" "$STARTED_AT"
trap - EXIT

echo "✔ codex ${MODE} done → ${OUT}" >&2
