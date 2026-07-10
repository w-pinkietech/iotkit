#!/usr/bin/env bash
# claude-review.sh — cross-vendor review dispatch for the Claude side.
#
# Symmetric COUNTERPART to codex.sh, for when codex is the MAIN driver: codex
# writes code, then dispatches BOTH review sides — `codex.sh review` (OpenAI) and
# this script (Claude) — with the SAME prompt, so cross-vendor review keeps working
# after the 2026-07-11 driver handoff
# (docs/superpowers/HANDOFF-2026-07-11-to-codex-driver.md).
#
# IMPORTANT — this is a STATIC reviewer, NOT the same shape as codex's sandbox.
# codex's read-only sandbox still EXECUTES commands (it can run `cargo test`, grep,
# boundary-probe scripts against a read-only filesystem). This script gives the
# reviewer Read/Grep/Glob only — Bash is technically removed (see the guard below),
# so it reads code and reasons but CANNOT run a single test. The run-it-and-test
# angle (runtime/data-loss/concurrency bugs that only surface on execution) is
# codex's job. Do not over-trust a clean Claude pass on execution-dependent findings.
#
# Read-only is enforced by TECHNICAL exclusion, not model cooperation (verified):
#   - `--disallowedTools Bash Edit Write NotebookEdit` removes those tools outright —
#     the reviewer has no Bash at all (proven: it returns NO_BASH_TOOL even under
#     bypassPermissions), so it genuinely cannot execute or mutate. This is the
#     load-bearing guard; plan mode alone only makes the MODEL refuse Bash, not the
#     tool unavailable.
#   - `--strict-mcp-config` so no MCP server loads (MCP tools are NOT gated by
#     --setting-sources and could otherwise execute/mutate).
#   - `--setting-sources ''` so a reviewed repo's own `.claude/settings.json`
#     hooks/permissions cannot load and escalate (a SessionStart hook in reviewed
#     content would otherwise run before the model starts).
#   - `--permission-mode plan` as a redundant fourth layer.
# CONSEQUENCE of `--setting-sources ''`: project CLAUDE.md / rules / skills are NOT
# auto-loaded, so the review PROMPT must supply the context the reviewer needs
# (design authority, the eval guides, architecture.md). The cross-vendor review
# briefs already inject these explicitly — keep doing so.
#
# Usage:
#   scripts/claude-review.sh <prompt-file> <label>
#
# The prompt is fed on STDIN so it may start with '-' and exceed arg limits.
# Output is written to a `.partial` and atomically renamed only on a non-empty
# success, so a crashed/auth-failed run leaves NO file a caller could misread as a
# clean, zero-findings review.
#
# Env overrides:
#   CLAUDE_REVIEW_MODEL   (default: unset → runtime/account default. NOTE: because
#                          `--setting-sources ''` suppresses user settings, this is
#                          NOT reliably your configured default — PIN a strong
#                          reviewer here (e.g. "opus") for any serious review)
#   CLAUDE_REVIEW_EFFORT  (default max for review — mirrors codex.sh's max; reviews
#                          earn the deepest reasoning. Scale: low<medium<high<xhigh<max)
#   CLAUDE_REVIEW_REPO    (default: current git toplevel; point at another checkout,
#                          e.g. the design corpus, symmetric to codex.sh CODEX_REPO)
#   CLAUDE_OUT_DIR        (default /tmp/codex-runs — same dir as codex.sh so both
#                          vendors' evidence lands together)
#   CLAUDE_BIN            (default: claude on PATH)
set -euo pipefail

PROMPT="${1:-}"; LABEL="${2:-}"
if [ -z "$PROMPT" ] || [ -z "$LABEL" ]; then
  echo "usage: scripts/claude-review.sh <prompt-file> <label>" >&2
  exit 2
fi

CLAUDE_BIN="${CLAUDE_BIN:-claude}"
CLAUDE_OUT_DIR="${CLAUDE_OUT_DIR:-/tmp/codex-runs}"
CLAUDE_REVIEW_EFFORT="${CLAUDE_REVIEW_EFFORT:-max}"
REPO="${CLAUDE_REVIEW_REPO:-$(git rev-parse --show-toplevel)}"
[ -d "$REPO" ] || { echo "repo not found: $REPO" >&2; exit 2; }

[ -s "$PROMPT" ] || { echo "prompt file missing or empty: $PROMPT" >&2; exit 2; }
# Absolutize before we cd into REPO to launch the reviewer.
PROMPT_ABS="$(cd "$(dirname "$PROMPT")" && pwd)/$(basename "$PROMPT")"
mkdir -p "$CLAUDE_OUT_DIR"
CLAUDE_OUT_DIR="$(cd "$CLAUDE_OUT_DIR" && pwd)"
OUT="$CLAUDE_OUT_DIR/claude-${LABEL}-review-$(date +%Y%m%d-%H%M%S)-$$.txt"
OUT_PARTIAL="$OUT.partial"

MODEL_ARGS=()
[ -n "${CLAUDE_REVIEW_MODEL:-}" ] && MODEL_ARGS=(--model "$CLAUDE_REVIEW_MODEL")

{
  echo "→ claude review (static, read-only: plan+disallow+no-settings)"
  echo "  model  = ${CLAUDE_REVIEW_MODEL:-<default>}  effort = ${CLAUDE_REVIEW_EFFORT}"
  echo "  repo   = ${REPO}"
  echo "  prompt = ${PROMPT_ABS}"
  echo "  out    = ${OUT}"
} >&2

# Clean up the partial unless we successfully rename it into place.
trap 'rm -f "$OUT_PARTIAL"' EXIT

# Launch from REPO root so relative Read/Grep/Glob resolve against the whole repo
# (not the caller's cwd — else a sibling crate is silently outside the search).
( cd "$REPO" && "$CLAUDE_BIN" -p \
    --permission-mode plan \
    --setting-sources "" \
    --strict-mcp-config \
    --disallowedTools Bash Edit Write NotebookEdit \
    --effort "$CLAUDE_REVIEW_EFFORT" \
    --output-format text \
    "${MODEL_ARGS[@]}" ) < "$PROMPT_ABS" > "$OUT_PARTIAL"

# Fail-closed: an empty output is a FAILED review, never a clean one.
[ -s "$OUT_PARTIAL" ] || { echo "x empty review output — treat as FAILED, not clean" >&2; exit 1; }
mv "$OUT_PARTIAL" "$OUT"
trap - EXIT

echo "✔ claude review done → ${OUT}" >&2
