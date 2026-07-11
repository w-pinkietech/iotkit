#!/usr/bin/env bash
# Write an atomic, machine-readable receipt binding a result to its inputs.
set -euo pipefail

VENDOR="${1:-}"; MODE="${2:-}"; MODEL="${3:-}"; EFFORT="${4:-}"
PROMPT="${5:-}"; RESULT="${6:-}"; STARTED_AT="${7:-}"
[ -n "$STARTED_AT" ] && [ -s "$PROMPT" ] && [ -s "$RESULT" ] || {
  echo "usage: scripts/review-receipt.sh <vendor> <mode> <model> <effort> <prompt> <result> <started-at>" >&2
  exit 2
}

RECEIPT="$RESULT.receipt"
PARTIAL="$RECEIPT.partial"
MANIFEST="${REVIEW_MANIFEST:-}"
for raw in "$PROMPT" "$RESULT" "$MANIFEST" "$STARTED_AT"; do
  [[ "$raw" != *$'\n'* && "$raw" != *$'\r'* ]] || { echo "receipt input contains newline" >&2; exit 2; }
done
if [ "$MODE" = review ] && [ -z "$MANIFEST" ]; then
  echo "REVIEW_MANIFEST is required for review receipts" >&2
  exit 2
fi
if [ -n "$MANIFEST" ]; then
  [ -s "$MANIFEST" ] || { echo "REVIEW_MANIFEST missing or empty: $MANIFEST" >&2; exit 2; }
  MANIFEST_SHA256="$(sha256sum -- "$MANIFEST" | cut -d' ' -f1)"
  MANIFEST_PATH="$(readlink -f "$MANIFEST")"
else
  MANIFEST_SHA256="UNBOUND"
  MANIFEST_PATH="UNBOUND"
fi
PROMPT_PATH="$(readlink -f "$PROMPT")"
RESULT_PATH="$(readlink -f "$RESULT")"
MANIFEST_PATH="${MANIFEST_PATH:-UNBOUND}"
for value in "$VENDOR" "$MODE" "$MODEL" "$EFFORT" "$STARTED_AT" "$PROMPT_PATH" "$RESULT_PATH" "$MANIFEST_PATH"; do
  [[ "$value" != *$'\n'* && "$value" != *$'\r'* ]] || { echo "receipt field contains newline" >&2; exit 2; }
done

trap 'rm -f "$PARTIAL"' EXIT
{
  printf 'receipt_version=1\nstatus=success\n'
  printf 'vendor=%s\nmode=%s\nmodel=%s\neffort=%s\n' "$VENDOR" "$MODE" "$MODEL" "$EFFORT"
  printf 'prompt_path=%s\nprompt_sha256=%s\n' "$PROMPT_PATH" "$(sha256sum -- "$PROMPT" | cut -d' ' -f1)"
  printf 'artifact_manifest_path=%s\nartifact_manifest_sha256=%s\n' "$MANIFEST_PATH" "$MANIFEST_SHA256"
  printf 'result_path=%s\nresult_sha256=%s\n' "$RESULT_PATH" "$(sha256sum -- "$RESULT" | cut -d' ' -f1)"
  printf 'started_at=%s\ncompleted_at=%s\n' "$STARTED_AT" "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$PARTIAL"
mv "$PARTIAL" "$RECEIPT"
trap - EXIT
