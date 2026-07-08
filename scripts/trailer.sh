#!/usr/bin/env bash
# trailer.sh — standard commit trailer block.
#
# The Co-Authored-By line names the CURRENT assistant model, auto-detected from
# this Claude Code session's own transcript (via CLAUDE_CODE_SESSION_ID) so that
# nobody has to remember to update anything when sessions switch models.
#
# Usage:
#   git commit -m "feat(crate): ..." -m "$(scripts/trailer.sh codex)"   # codex-implemented, cross-vendor reviewed
#   git commit -m "docs: ..."        -m "$(scripts/trailer.sh docs)"    # main-agent authored (docs/harness)
#
# TRAILER_MODEL overrides detection — only needed when the detected name is wrong:
#   TRAILER_MODEL="Claude Opus 4.8" scripts/trailer.sh codex
#
# Detection failure falls back to FALLBACK_MODEL instead of erroring: callers
# invoke this via $(command substitution), which swallows exit codes — a hard
# fail here would silently commit an EMPTY trailer, worse than a stale name.
# If the transcript format ever changes, detection degrades to the fallback;
# update FALLBACK_MODEL when that happens.
set -euo pipefail

MODE="${1:-}"
[ -n "$MODE" ] || { echo "usage: scripts/trailer.sh <codex|docs>" >&2; exit 2; }

FALLBACK_MODEL="Claude Fable 5"

# Print the current session model's display name, or return 1 (caller falls back).
# The transcript's main-chain assistant lines look like
#   ..."isSidechain":false,...,"message":{"model":"claude-fable-5",...
# The LAST such line names the model in charge right now (sessions switch models
# mid-flight via /model, so last — not most frequent — is the correct pick).
# "isSidechain":false guards against subagent lines; requiring the claude- prefix
# skips "<synthetic>" bookkeeping entries; escaped copies of this pattern inside
# tool-result strings (\"model\":...) can never match the unescaped pattern.
detect_model() {
  local sid="${CLAUDE_CODE_SESSION_ID:-}" f hit id=""
  [ -n "$sid" ] || return 1
  for f in "$HOME"/.claude/projects/*/"$sid".jsonl; do
    [ -r "$f" ] || continue
    hit="$(tac "$f" 2>/dev/null \
           | grep -m1 -E '"isSidechain":false.*"message":\{"model":"claude-[a-z0-9-]+"' \
           || true)"
    id="$(printf '%s' "$hit" \
          | grep -oE '"message":\{"model":"claude-[a-z0-9-]+"' | head -n1 \
          | cut -d'"' -f6 || true)"
    [ -n "$id" ] && break
  done
  [ -n "$id" ] || return 1
  # Model id -> display name: claude-fable-5 -> Claude Fable 5,
  # claude-opus-4-8 -> Claude Opus 4.8, claude-haiku-4-5-20251001 -> Claude Haiku 4.5.
  id="${id#claude-}"
  id="${id%-[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]}"   # drop -YYYYMMDD date suffix
  local name="" ver="" tok toks
  IFS=- read -ra toks <<<"$id"
  for tok in "${toks[@]}"; do
    if [[ "$tok" =~ ^[0-9]+$ ]]; then
      ver="${ver:+$ver.}$tok"
    else
      name="${name:+$name }${tok^}"
    fi
  done
  [ -n "$name" ] || return 1
  printf 'Claude %s%s' "$name" "${ver:+ $ver}"
}

MODEL="${TRAILER_MODEL:-}"
if [ -z "$MODEL" ]; then
  MODEL="$(detect_model || true)"
fi
MODEL="${MODEL:-$FALLBACK_MODEL}"
MODEL="${MODEL//[$'\n\r']/ }"   # collapse newlines: a single-line display name, no trailer injection

case "$MODE" in
  codex)
    printf 'Implemented-by: codex\nReviewed-by: codex (read-only), Fable review-max\nCo-Authored-By: %s <noreply@anthropic.com>\n' "$MODEL" ;;
  docs)
    printf 'Co-Authored-By: %s <noreply@anthropic.com>\n' "$MODEL" ;;
  *) echo "usage: scripts/trailer.sh <codex|docs>" >&2; exit 2 ;;
esac
