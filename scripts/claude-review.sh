#!/usr/bin/env bash
# claude-review.sh — optional static Claude review dispatch.
#
# This is the Claude counterpart to `codex.sh review` when Claude access is available
# and the run is explicitly opted in. Required-vendor status is governed only by
# docs/development-workflow.md; in the current degraded mode this wrapper is optional
# and creates no review debt unless selected before dispatch.
#
# IMPORTANT — this is a STATIC reviewer, NOT the same shape as codex's sandbox.
# codex's read-only sandbox still EXECUTES commands (it can run `cargo test`, grep,
# boundary-probe scripts against a read-only filesystem). This script gives the
# reviewer no tools. Manifest-listed files are bundled into its stdin, so it reasons
# over frozen content but cannot read the host or run a test. The run-it-and-test
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
#   - `--permission-mode dontAsk` prevents approval prompts; tool removal remains load-bearing.
# CONSEQUENCE of `--setting-sources ''`: project CLAUDE.md / rules / skills are NOT
# auto-loaded, so the review PROMPT must supply the context the reviewer needs
# (design authority, the eval guides, architecture.md). The cross-vendor review
# briefs already inject these explicitly — keep doing so.
#
# Usage:
#   REVIEW_MANIFEST=<manifest> scripts/claude-review.sh <prompt-file> <label>
#
# The prompt is fed on STDIN so it may start with '-' and exceed arg limits.
# Output is written to a `.partial` and atomically renamed only on a non-empty
# success, so a crashed/auth-failed run leaves NO file a caller could misread as a
# clean, zero-findings review.
#
# Env overrides:
#   CLAUDE_REVIEW_MODEL   (compatibility default fable; when optional Claude review is
#                          restored, choose routing explicitly under the workflow canon)
#   CLAUDE_REVIEW_EFFORT  (compatibility default high; it is not the current required-review
#                          default and does not make max routine)
#   CLAUDE_REVIEW_REPO    (default: current git toplevel; point at another checkout,
#                          e.g. the design corpus, symmetric to codex.sh CODEX_REPO)
#   CLAUDE_OUT_DIR        (default /tmp/codex-runs — same dir as codex.sh so both
#                          vendors' evidence lands together)
#   CLAUDE_BIN            (default: claude on PATH)
set -euo pipefail
umask 077
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

PROMPT="${1:-}"; LABEL="${2:-}"
if [ -z "$PROMPT" ] || [ -z "$LABEL" ]; then
  echo "usage: scripts/claude-review.sh <prompt-file> <label>" >&2
  exit 2
fi
[[ "$LABEL" =~ ^[A-Za-z0-9._-]{1,80}$ ]] || { echo "unsafe label" >&2; exit 2; }

CLAUDE_BIN="${CLAUDE_BIN:-claude}"
CLAUDE_OUT_DIR="${CLAUDE_OUT_DIR:-/tmp/codex-runs}"
CLAUDE_REVIEW_MODEL="${CLAUDE_REVIEW_MODEL:-fable}"
CLAUDE_REVIEW_EFFORT="${CLAUDE_REVIEW_EFFORT:-high}"
case "$CLAUDE_REVIEW_EFFORT" in low|medium|high|xhigh|max) ;; *) echo "unsupported Claude effort" >&2; exit 2 ;; esac
REPO="${CLAUDE_REVIEW_REPO:-$(git rev-parse --show-toplevel)}"
[ -d "$REPO" ] || { echo "repo not found: $REPO" >&2; exit 2; }

[ -s "$PROMPT" ] || { echo "prompt file missing or empty: $PROMPT" >&2; exit 2; }
[ -n "${REVIEW_MANIFEST:-}" ] || { echo "REVIEW_MANIFEST is required for review" >&2; exit 2; }
if [ -n "${REVIEW_MANIFEST:-}" ]; then
  REVIEW_MANIFEST="$(readlink -f "$REVIEW_MANIFEST")"
  export REVIEW_MANIFEST
  (cd "$REPO" && "$SCRIPT_DIR/review-manifest.sh" --verify "$REVIEW_MANIFEST")
  MANIFEST_SHA256="$(sha256sum -- "$REVIEW_MANIFEST" | cut -d' ' -f1)"
fi
# Absolutize before we cd into REPO to launch the reviewer.
PROMPT_ABS="$(cd "$(dirname "$PROMPT")" && pwd)/$(basename "$PROMPT")"
CONTEXT_PROMPT="$(mktemp)"
cp -- "$PROMPT_ABS" "$CONTEXT_PROMPT"
while IFS=$'\t' read -r _ path; do
  printf '\n\n===== ARTIFACT: %s =====\n' "$path" >> "$CONTEXT_PROMPT"
  cat -- "$REPO/$path" >> "$CONTEXT_PROMPT"
done < "$REVIEW_MANIFEST"
mkdir -p "$CLAUDE_OUT_DIR"
chmod 700 "$CLAUDE_OUT_DIR"
[ -O "$CLAUDE_OUT_DIR" ] || { echo "output directory is not owned by current user" >&2; exit 2; }
CLAUDE_OUT_DIR="$(cd "$CLAUDE_OUT_DIR" && pwd)"
OUT="$CLAUDE_OUT_DIR/claude-${LABEL}-review-$(date +%Y%m%d-%H%M%S)-$$.txt"
OUT_PARTIAL="$OUT.partial"
PROMPT_SHA256="$(sha256sum -- "$PROMPT_ABS" | cut -d' ' -f1)"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

{
  echo "→ claude review (static, read-only: dontAsk+tools+deny+safe-mode)"
  echo "  model  = ${CLAUDE_REVIEW_MODEL:-<default>}  effort = ${CLAUDE_REVIEW_EFFORT}"
  echo "  repo   = ${REPO}"
  echo "  prompt = ${PROMPT_ABS}"
  echo "  out    = ${OUT}"
} >&2

# Clean up the partial unless we successfully rename it into place.
trap 'rm -f "$OUT_PARTIAL" "$CONTEXT_PROMPT"' EXIT

# Launch from REPO root so relative Read/Grep/Glob resolve against the whole repo
# (not the caller's cwd — else a sibling crate is silently outside the search).
( cd "$REPO" && env -i HOME="${HOME:-}" PATH="$PATH" LANG="${LANG:-C.UTF-8}" "$CLAUDE_BIN" -p \
    --permission-mode dontAsk \
    --safe-mode \
    --setting-sources "" \
    --strict-mcp-config \
    --tools "" \
    --disallowedTools Bash Edit Write NotebookEdit Read Grep Glob WebFetch WebSearch Task \
    --disable-slash-commands \
    --no-session-persistence \
    --no-chrome \
    --effort "$CLAUDE_REVIEW_EFFORT" \
    --output-format text \
    --model "$CLAUDE_REVIEW_MODEL" ) < "$CONTEXT_PROMPT" > "$OUT_PARTIAL"

# Fail-closed: an empty output is a FAILED review, never a clean one.
[ -s "$OUT_PARTIAL" ] || { echo "x empty review output — treat as FAILED, not clean" >&2; exit 1; }
mv "$OUT_PARTIAL" "$OUT"
trap 'rm -f "$OUT_PARTIAL" "$OUT" "$OUT.receipt" "$OUT.receipt.partial" "$CONTEXT_PROMPT"' EXIT
[ "$(sha256sum -- "$PROMPT_ABS" | cut -d' ' -f1)" = "$PROMPT_SHA256" ] || { echo "prompt changed during review" >&2; exit 1; }
(cd "$REPO" && "$SCRIPT_DIR/review-manifest.sh" --verify "$REVIEW_MANIFEST")
[ "$(sha256sum -- "$REVIEW_MANIFEST" | cut -d' ' -f1)" = "$MANIFEST_SHA256" ] || { echo "manifest changed during review" >&2; exit 1; }
"$SCRIPT_DIR/review-receipt.sh" claude review "$CLAUDE_REVIEW_MODEL" \
  "$CLAUDE_REVIEW_EFFORT" "$PROMPT_ABS" "$OUT" "$STARTED_AT"
rm -f "$CONTEXT_PROMPT"
trap - EXIT

echo "✔ claude review done → ${OUT}" >&2
