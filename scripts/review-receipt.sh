#!/usr/bin/env bash
# Write an atomic, machine-readable receipt binding a result to its inputs.
set -euo pipefail

VENDOR="${1:-}"; MODE="${2:-}"; REQUESTED_MODEL="${3:-}"; REQUESTED_EFFORT="${4:-}"
PROMPT="${5:-}"; RESULT="${6:-}"; STARTED_AT="${7:-}"
OBSERVED_MODEL="${8:-UNAVAILABLE}"
OBSERVED_EFFORT="${9:-UNAVAILABLE}"
SANDBOX_MODE="${10:-UNAVAILABLE}"
APPROVAL_POLICY="${11:-UNAVAILABLE}"
EVENT_STREAM="${12:-UNAVAILABLE}"
MODEL_REROUTE_OBSERVED="${13:-UNAVAILABLE}"
EXPECTED_PROMPT_SHA256="${14:-}"
EXPECTED_MANIFEST_SHA256="${15:-}"
[ "$#" -ge 7 ] && [ "$#" -le 15 ] && [ -n "$STARTED_AT" ] && [ -s "$PROMPT" ] && [ -s "$RESULT" ] || {
  echo "usage: scripts/review-receipt.sh <vendor> <mode> <requested-model> <requested-effort> <prompt> <result> <started-at> [observed-model] [observed-effort] [sandbox-mode] [approval-policy] [event-stream] [model-reroute-observed] [expected-prompt-sha256 expected-manifest-sha256]" >&2
  exit 2
}
if [ "$#" -ge 14 ] && [ "$#" -ne 15 ]; then
  echo "expected prompt and manifest hashes must be supplied together" >&2
  exit 2
fi
if [ "$VENDOR" = codex ] && [ "$#" -ne 15 ]; then
  echo "Codex receipts require frozen prompt and manifest hashes" >&2
  exit 2
fi

RECEIPT="$RESULT.receipt"
PARTIAL="$RECEIPT.partial"
MANIFEST="${REVIEW_MANIFEST:-}"
trap 'rm -f -- "$PARTIAL"' EXIT
for raw in "$PROMPT" "$RESULT" "$MANIFEST" "$STARTED_AT" "$VENDOR" "$MODE" \
  "$REQUESTED_MODEL" "$REQUESTED_EFFORT" "$OBSERVED_MODEL" "$OBSERVED_EFFORT" \
  "$SANDBOX_MODE" "$APPROVAL_POLICY" "$EVENT_STREAM" "$MODEL_REROUTE_OBSERVED" \
  "$EXPECTED_PROMPT_SHA256" "$EXPECTED_MANIFEST_SHA256"; do
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
PROMPT_SHA256="$(sha256sum -- "$PROMPT" | cut -d' ' -f1)"
RESULT_SHA256="$(sha256sum -- "$RESULT" | cut -d' ' -f1)"

if [ "$VENDOR" = codex ]; then
  [ "$EVENT_STREAM" != UNAVAILABLE ] && [ -s "$EVENT_STREAM" ] || {
    echo "Codex receipts require a bound event stream" >&2
    exit 2
  }
  [ "$MODEL_REROUTE_OBSERVED" = false ] || {
    echo "Codex receipts require model_reroute_observed=false" >&2
    exit 2
  }
  EVENT_STREAM_PATH="$(readlink -f "$EVENT_STREAM")"
  EVENT_STREAM_SHA256="$(sha256sum -- "$EVENT_STREAM" | cut -d' ' -f1)"
  [[ "$EXPECTED_PROMPT_SHA256" =~ ^[0-9a-f]{64}$ ]] || {
    echo "Codex receipt expected prompt hash is invalid" >&2
    exit 2
  }
  [[ "$EXPECTED_MANIFEST_SHA256" = UNBOUND || "$EXPECTED_MANIFEST_SHA256" =~ ^[0-9a-f]{64}$ ]] || {
    echo "Codex receipt expected manifest hash is invalid" >&2
    exit 2
  }
  [ "$PROMPT_SHA256" = "$EXPECTED_PROMPT_SHA256" ] || {
    echo "prompt changed between dispatch verification and receipt creation" >&2
    exit 1
  }
  [ "$MANIFEST_SHA256" = "$EXPECTED_MANIFEST_SHA256" ] || {
    echo "manifest changed between dispatch verification and receipt creation" >&2
    exit 1
  }
else
  OBSERVED_MODEL=UNAVAILABLE
  OBSERVED_EFFORT=UNAVAILABLE
  EVENT_STREAM_PATH=UNAVAILABLE
  EVENT_STREAM_SHA256=UNAVAILABLE
  MODEL_REROUTE_OBSERVED=UNAVAILABLE
fi

for value in "$VENDOR" "$MODE" "$REQUESTED_MODEL" "$REQUESTED_EFFORT" "$STARTED_AT" \
  "$PROMPT_PATH" "$RESULT_PATH" "$MANIFEST_PATH" "$OBSERVED_MODEL" "$OBSERVED_EFFORT" \
  "$SANDBOX_MODE" "$APPROVAL_POLICY" "$EVENT_STREAM_PATH" "$EVENT_STREAM_SHA256" \
  "$MODEL_REROUTE_OBSERVED"; do
  [[ "$value" != *$'\n'* && "$value" != *$'\r'* ]] || { echo "receipt field contains newline" >&2; exit 2; }
done

trap 'rm -f "$PARTIAL"' EXIT
{
  printf 'receipt_version=2\nstatus=success\n'
  printf 'vendor=%s\nmode=%s\nrequested_model=%s\nrequested_effort=%s\n' \
    "$VENDOR" "$MODE" "$REQUESTED_MODEL" "$REQUESTED_EFFORT"
  printf 'observed_model=%s\nobserved_effort=%s\nsandbox_mode=%s\napproval_policy=%s\n' \
    "$OBSERVED_MODEL" "$OBSERVED_EFFORT" "$SANDBOX_MODE" "$APPROVAL_POLICY"
  printf 'model_reroute_observed=%s\n' "$MODEL_REROUTE_OBSERVED"
  printf 'prompt_path=%s\nprompt_sha256=%s\n' "$PROMPT_PATH" "$PROMPT_SHA256"
  printf 'artifact_manifest_path=%s\nartifact_manifest_sha256=%s\n' "$MANIFEST_PATH" "$MANIFEST_SHA256"
  printf 'result_path=%s\nresult_sha256=%s\n' "$RESULT_PATH" "$RESULT_SHA256"
  printf 'event_stream_path=%s\nevent_stream_sha256=%s\n' "$EVENT_STREAM_PATH" "$EVENT_STREAM_SHA256"
  printf 'started_at=%s\ncompleted_at=%s\n' "$STARTED_AT" "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$PARTIAL"
mv "$PARTIAL" "$RECEIPT"
trap - EXIT
