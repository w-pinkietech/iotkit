#!/usr/bin/env bash
# Validate the successful Codex JSONL lifecycle before it can become evidence.
set -euo pipefail

EVENTS="${1:-}"
[ "$#" -eq 1 ] && [ -n "$EVENTS" ] || {
  echo "usage: scripts/check-codex-events.sh <events-jsonl>" >&2
  exit 2
}
[ -s "$EVENTS" ] || { echo "event stream missing or empty: $EVENTS" >&2; exit 1; }

state=expect_thread_started
line_number=0

while IFS= read -r line || [ -n "$line" ]; do
  line_number=$((line_number + 1))
  [ -n "$line" ] || { echo "empty JSONL line at $line_number" >&2; exit 1; }
  if ! event_type="$(jq -er \
      'if (type == "object") and (((.type // null) | type) == "string") then .type else error("event type must be a string") end' \
      <<<"$line" 2>/dev/null)"; then
    echo "malformed JSON event at line $line_number" >&2
    exit 1
  fi
  event_type="${event_type,,}"
  if [[ "$event_type" =~ model[-_.]?reroute ]]; then
    echo "model reroute event rejected at line $line_number" >&2
    exit 1
  fi
  case "$event_type" in
    thread.started)
      [ "$state" = expect_thread_started ] || {
        echo "unexpected thread.started at line $line_number" >&2
        exit 1
      }
      state=expect_turn_started
      ;;
    turn.started)
      [ "$state" = expect_turn_started ] || {
        echo "unexpected turn.started at line $line_number" >&2
        exit 1
      }
      state=expect_turn_completed
      ;;
    turn.completed)
      [ "$state" = expect_turn_completed ] || {
        echo "unexpected turn.completed at line $line_number" >&2
        exit 1
      }
      state=complete
      ;;
    turn.failed|error)
      echo "unsuccessful Codex event rejected at line $line_number: $event_type" >&2
      exit 1
      ;;
  esac
done < "$EVENTS"

[ "$state" = complete ] || { echo "event stream ended before turn.completed" >&2; exit 1; }

printf 'model_reroute_observed=false\n'
