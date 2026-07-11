#!/usr/bin/env bash
# Build a deterministic, mode-aware manifest for working-tree review artifacts.
set -euo pipefail

if [ "${1:-}" = "--verify" ]; then
  MANIFEST="${2:-}"
  [ -s "$MANIFEST" ] || { echo "manifest missing or empty: $MANIFEST" >&2; exit 2; }
  [ "$(tail -c 1 "$MANIFEST" | od -An -t u1 | tr -d ' ')" = 10 ] || { echo "manifest must end with newline" >&2; exit 1; }
  REPO_ROOT="$(git rev-parse --show-toplevel)"
  count=0
  while IFS=$'\t' read -r descriptor path; do
    count=$((count + 1))
    read -r expected_mode type expected_hash <<<"$descriptor"
    [[ "$descriptor" =~ ^100(644|755)\ blob\ [0-9a-f]{40}$ ]] || { echo "malformed manifest record" >&2; exit 1; }
    case "$path" in ""|/*|..|../*|*/../*|*/..) echo "manifest path escapes repository: $path" >&2; exit 1 ;; esac
    [ ! -L "$path" ] || { echo "manifest symlink rejected: $path" >&2; exit 1; }
    [ "$type" = blob ] && [ -f "$path" ] || { echo "manifest path/type mismatch: $path" >&2; exit 1; }
    canonical="$(realpath -e -- "$path")"
    case "$canonical" in "$REPO_ROOT"/*) ;; *) echo "manifest path outside repository: $path" >&2; exit 1 ;; esac
    if [ -x "$path" ]; then actual_mode=100755; else actual_mode=100644; fi
    actual_hash="$(git hash-object -- "$path")"
    [ "$actual_mode" = "$expected_mode" ] && [ "$actual_hash" = "$expected_hash" ] || {
      echo "manifest content/mode mismatch: $path" >&2
      exit 1
    }
  done < "$MANIFEST"
  [ "$count" -gt 0 ] || { echo "empty manifest" >&2; exit 1; }
  echo "manifest verified: $MANIFEST" >&2
  exit 0
fi

OUT="${1:-}"
shift || true
[ -n "$OUT" ] && [ "$#" -gt 0 ] || {
  echo "usage: scripts/review-manifest.sh <output> <path>..." >&2
  exit 2
}

TMP="$OUT.partial"
trap 'rm -f "$TMP"' EXIT
: > "$TMP"
for path in "$@"; do
  [ -f "$path" ] || { echo "artifact is not a file: $path" >&2; exit 2; }
  [ ! -L "$path" ] || { echo "artifact symlink rejected: $path" >&2; exit 2; }
  case "$path" in /*|..|../*|*/../*|*/..) echo "artifact path escapes repository: $path" >&2; exit 2 ;; esac
  if [ -x "$path" ]; then mode=100755; else mode=100644; fi
  printf '%s blob %s\t%s\n' "$mode" "$(git hash-object -- "$path")" "$path" >> "$TMP"
done
LC_ALL=C sort -o "$TMP" "$TMP"
mv "$TMP" "$OUT"
trap - EXIT
printf '%s  %s\n' "$(sha256sum -- "$OUT" | cut -d' ' -f1)" "$OUT"
