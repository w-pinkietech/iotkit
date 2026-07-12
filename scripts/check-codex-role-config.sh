#!/usr/bin/env bash
# Resolve project role layers through config/read and strict-parse each layer.
# This performs no model turn and is the repository-owned role-layer preflight.
set -euo pipefail
umask 077

REPO="${CODEX_REPO:-$(git rev-parse --show-toplevel)}"
REPO="$(realpath -e -- "$REPO")"
CODEX_BIN="${CODEX_BIN:-/home/kenta/.local/bin/codex}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() {
  echo "codex role config preflight: FAIL: $*" >&2
  exit 1
}

[ -d "$REPO" ] || fail "repository is not a directory: $REPO"
[ -f "$REPO/.codex/config.toml" ] || fail "project config is missing: $REPO/.codex/config.toml"
[ -x "$CODEX_BIN" ] || fail "Codex binary is missing or not executable: $CODEX_BIN"
command -v jq >/dev/null 2>&1 || fail "jq is required"
command -v timeout >/dev/null 2>&1 || fail "timeout is required"

toml_quoted_path() {
  jq -Rn --arg path "$1" '$path'
}

write_trusted_home_config() {
  local home="$1"
  local trusted_path="$2"
  printf '[projects.%s]\ntrust_level = "trusted"\n' "$(toml_quoted_path "$trusted_path")" \
    > "$home/config.toml"
}

run_app_server() {
  local home="$1"
  local cwd="$2"
  local stdout="$3"
  local stderr="$4"
  local request="$5"

  {
    printf '%s\n' '{"method":"initialize","id":1,"params":{"clientInfo":{"name":"iotkit-role-preflight","title":"iotkit-role-preflight","version":"1"}}}'
    printf '%s\n' '{"method":"initialized","params":{}}'
    if [ -n "$request" ]; then
      printf '%s\n' "$request"
    fi
    sleep 0.2
  } | (cd "$cwd" && CODEX_HOME="$home" timeout 10s "$CODEX_BIN" \
    app-server --strict-config --listen stdio://) > "$stdout" 2> "$stderr"
}

PROJECT_HOME="$TMP/project-home"
PROJECT_PROBE="$TMP/project-probe"
mkdir -p "$PROJECT_HOME" "$PROJECT_PROBE"
write_trusted_home_config "$PROJECT_HOME" "$REPO"
PROJECT_REQUEST="$(jq -cn --arg cwd "$REPO" \
  '{method:"config/read",id:2,params:{cwd:$cwd,includeLayers:true}}')"

if ! run_app_server "$PROJECT_HOME" "$PROJECT_PROBE" \
  "$TMP/project.stdout" "$TMP/project.stderr" "$PROJECT_REQUEST"; then
  fail "Codex config/read could not load the trusted project configuration"
fi

if ! jq -s -e -r '
  map(select(.id == 2)) as $responses
  | if ($responses | length) != 1 then
      error("config/read response missing")
    elif ($responses[0].error? != null) then
      error("config/read returned an error")
    elif (($responses[0].result.config.agents? | type) != "object") then
      error("config/read did not return agents")
    else
      ($responses[0].result.config.agents | to_entries
        | map(select((.value | type) == "object" and (.value | has("config_file"))))
      ) as $roles
      | (["implementer", "executor", "reviewer"] - ($roles | map(.key))) as $missing
      | if ($missing | length) > 0 then
          error("required role path is missing")
        elif any($roles[]; ((.value.config_file | type) != "string" or (.value.config_file | length) == 0)) then
          error("role config_file is not a non-empty string")
        else
          $roles[] | [.key, .value.config_file] | @tsv
        end
    end
' "$TMP/project.stdout" > "$TMP/roles.tsv"; then
  fail "Codex config/read did not resolve the required project role paths"
fi

mapfile -t ROLE_ROWS < "$TMP/roles.tsv"
[ "${#ROLE_ROWS[@]}" -gt 0 ] || fail "project config resolved no role layers"

validate_role_layer() {
  local role="$1"
  local configured_path="$2"
  local canonical_path
  ROLE_INDEX=$((ROLE_INDEX + 1))
  local role_home="$TMP/role-home-$ROLE_INDEX"
  local role_probe="$TMP/role-probe-$ROLE_INDEX"

  case "$configured_path" in
    /*) ;;
    *) fail "role $role resolved to a non-absolute path" ;;
  esac
  [ -f "$configured_path" ] || fail "role $role layer is missing: $configured_path"
  [ ! -L "$configured_path" ] || fail "role $role layer must not be a symlink: $configured_path"
  canonical_path="$(realpath -e -- "$configured_path")"
  case "$canonical_path" in
    "$REPO/.codex/"*) ;;
    *) fail "role $role layer is outside the project .codex directory" ;;
  esac

  mkdir -p "$role_home" "$role_probe"
  cp -- "$canonical_path" "$role_home/config.toml"

  # The role layer becomes CODEX_HOME/config.toml for the installed strict
  # parser. The probe cwd is outside the project so config/read is not used to
  # validate the layer and no model turn is issued.
  if ! run_app_server "$role_home" "$role_probe" \
    "$TMP/$role.stdout" "$TMP/$role.stderr" \
    ''; then
    fail "Codex strict parser rejected role $role layer: $configured_path"
  fi
}

ROLE_INDEX=0
for row in "${ROLE_ROWS[@]}"; do
  role="${row%%$'\t'*}"
  configured_path="${row#*$'\t'}"
  [ -n "$role" ] && [ "$configured_path" != "$row" ] || fail "malformed resolved role path"
  validate_role_layer "$role" "$configured_path"
done

printf 'codex role config preflight: OK (%s role layers)\n' "${#ROLE_ROWS[@]}"
