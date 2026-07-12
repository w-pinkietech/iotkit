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
#   CODEX_MODEL   (override; review defaults to gpt-5.6-sol, impl to
#                  gpt-5.6-luna.)
#   CODEX_EFFORT  (override; review defaults to high, impl to max.
#                  Accepted scale: low < medium < high < xhigh < max.)
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
  review) SANDBOX="read-only";       APPROVAL="never"; DEFAULT_MODEL="gpt-5.6-sol";  DEFAULT_EFFORT="high" ;;
  impl)   SANDBOX="workspace-write"; APPROVAL="never"; DEFAULT_MODEL="gpt-5.6-luna"; DEFAULT_EFFORT="max" ;;
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
EVENT_STREAM="$OUT.events.jsonl"
EVENT_STREAM_PARTIAL="$EVENT_STREAM.partial"
PROMPT_SHA256="$(sha256sum -- "$PROMPT" | cut -d' ' -f1)"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

{
  echo "→ codex ${MODE} (sandbox=${SANDBOX}) model=${CODEX_MODEL} effort=${CODEX_EFFORT}"
  echo "  repo   = ${REPO}"
  echo "  prompt = ${PROMPT}"
  echo "  out    = ${OUT}"
} >&2

# Prompt on stdin ('-'); flags before it. Never expose a partial/empty result,
# event stream, or receipt as review evidence.
cleanup() {
  rm -f -- "$OUT_PARTIAL" "$EVENT_STREAM_PARTIAL" "$OUT" "$EVENT_STREAM" \
    "$OUT.receipt" "$OUT.receipt.partial"
}
trap cleanup EXIT
if ! "$CODEX_BIN" -a "$APPROVAL" exec \
  -s "$SANDBOX" \
  --skip-git-repo-check \
  -C "$REPO" \
  -m "$CODEX_MODEL" \
  -c "model_reasoning_effort=${CODEX_EFFORT}" \
  --json \
  -o "$OUT_PARTIAL" \
  - < "$PROMPT" > "$EVENT_STREAM_PARTIAL"; then
  echo "x Codex command failed — treat as FAILED" >&2
  exit 1
fi

[ -s "$OUT_PARTIAL" ] || { echo "x empty codex output — treat as FAILED" >&2; exit 1; }
[ -s "$EVENT_STREAM_PARTIAL" ] || { echo "x empty Codex event stream — treat as FAILED" >&2; exit 1; }
if ! EVENT_EVIDENCE="$("$SCRIPT_DIR/check-codex-events.sh" "$EVENT_STREAM_PARTIAL")"; then
  echo "x invalid Codex event stream — treat as FAILED" >&2
  exit 1
fi
MODEL_REROUTE_OBSERVED="${EVENT_EVIDENCE#model_reroute_observed=}"
[ "$MODEL_REROUTE_OBSERVED" = false ] || { echo "x unsupported event evidence" >&2; exit 1; }
[ "$(sha256sum -- "$PROMPT" | cut -d' ' -f1)" = "$PROMPT_SHA256" ] || { echo "prompt changed during review" >&2; exit 1; }
if [ -n "${REVIEW_MANIFEST:-}" ]; then
  (cd "$REPO" && "$SCRIPT_DIR/review-manifest.sh" --verify "$REVIEW_MANIFEST")
  [ "$(sha256sum -- "$REVIEW_MANIFEST" | cut -d' ' -f1)" = "$MANIFEST_SHA256" ] || {
    echo "manifest changed during review" >&2
    exit 1
  }
fi
mv "$OUT_PARTIAL" "$OUT"
mv "$EVENT_STREAM_PARTIAL" "$EVENT_STREAM"
# Test-only seam used by scripts/test-codex.sh to mutate an input in the final
# check-to-receipt interval; normal callers leave it unset.
if [ -n "${CODEX_TEST_PRE_RECEIPT_HOOK:-}" ]; then
  "$CODEX_TEST_PRE_RECEIPT_HOOK"
fi
"$SCRIPT_DIR/review-receipt.sh" codex "$MODE" "$CODEX_MODEL" "$CODEX_EFFORT" \
  "$PROMPT" "$OUT" "$STARTED_AT" \
  UNAVAILABLE UNAVAILABLE "$SANDBOX" "$APPROVAL" "$EVENT_STREAM" "$MODEL_REROUTE_OBSERVED" \
  "$PROMPT_SHA256" "${MANIFEST_SHA256:-UNBOUND}"
trap - EXIT

echo "✔ codex ${MODE} done → ${OUT}" >&2
