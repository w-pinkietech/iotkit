#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
scratch=$(mktemp -d)
export XDG_DATA_HOME="$scratch/state"
state="$XDG_DATA_HOME/iotkit/trial"
config="$scratch/iotkit.toml"
password_file="$scratch/admin-password"
launcher_log="$scratch/launcher.log"
cookies="$scratch/cookies"
login_response="$scratch/login.json"
port=$((21000 + $$ % 10000))
broker_port=$((port + 1))
stage="initializing the trial journey"

cleanup() {
  status=$?
  if ((status != 0)); then
    echo "trial journey failed while: $stage" >&2
    if [[ -f "$scratch/login.html" ]]; then
      echo "== trial login response ==" >&2
      sed -n '1,80p' "$scratch/login.html" >&2
    fi
    if [[ -f "$state/trial.env" ]]; then
      echo "== trial Compose state ==" >&2
      docker compose --env-file "$state/trial.env" \
        --file "$repo_root/deploy/compose.trial.yaml" ps --all >&2 || true
      echo "== trial service logs ==" >&2
      docker compose --env-file "$state/trial.env" \
        --file "$repo_root/deploy/compose.trial.yaml" \
        logs --no-color --tail 200 >&2 || true
    fi
  fi
  if [[ -f "$state/trial-state.json" ]]; then
    python3 "$repo_root/scripts/iotkit_trial.py" --config "$config" down >/dev/null 2>&1 || true
    python3 "$repo_root/scripts/iotkit_trial.py" --config "$config" \
      reset --confirm-trial-data-loss >/dev/null 2>&1 || true
  fi
  rm -rf "$scratch"
  return "$status"
}
trap cleanup EXIT

cat >"$config" <<EOF
config_version = 1
profile = "trial"

[trial]
console_port = $port
broker_port = $broker_port
sample_interval_ms = 250
EOF
openssl rand -base64 24 >"$password_file"
chmod 600 "$password_file"

stage="validating the trial configuration"
python3 "$repo_root/scripts/iotkit_trial.py" --config "$config" validate
stage="building and starting the trial services"
python3 "$repo_root/scripts/iotkit_trial.py" --config "$config" \
  up --admin-password-file "$password_file" >"$launcher_log"
password=$(<"$password_file")
stage="checking that the launcher did not expose the administrator password"
if grep -Fq "$password" "$launcher_log"; then
  echo "trial launcher exposed the administrator password" >&2
  exit 1
fi

origin="http://127.0.0.1:$port"
stage="waiting for the trial Console login page"
trial_login_ready=false
for _ in $(seq 1 60); do
  if curl --noproxy '*' -fsS "$origin/login" >"$scratch/login.html" &&
    grep -Fq "お試し環境" "$scratch/login.html"; then
    trial_login_ready=true
    break
  fi
  sleep 1
done
[[ "$trial_login_ready" == true ]]

stage="logging in to the trial Console"
login_payload=$(jq -nc --arg password "$password" \
  '{login_id:"admin", password:$password}')
login_code=$(curl --noproxy '*' -sS -c "$cookies" -o "$login_response" -w '%{http_code}' \
  -H "Origin: $origin" -H 'Content-Type: application/json' \
  --data "$login_payload" "$origin/api/v1/session")
[[ "$login_code" == 201 ]]
csrf_token=$(jq -er '.csrf_token' "$login_response")

stage="waiting for Edge Node discovery"
discovered=false
for _ in $(seq 1 60); do
  curl --noproxy '*' -fsS -b "$cookies" \
    "$origin/api/v1/edge-nodes" >"$scratch/edge-nodes.json"
  if jq -e '.items | length == 1 and .[0].state == "needs-setup"' \
    "$scratch/edge-nodes.json" >/dev/null; then
    discovered=true
    break
  fi
  sleep 1
done
[[ "$discovered" == true ]]

edge_node_ref=$(jq -er '.items[0].edge_node_ref' "$scratch/edge-nodes.json")
revision=$(jq -er '.items[0].revision' "$scratch/edge-nodes.json")
stage="activating the discovered Edge Node"
activation_code=$(curl --noproxy '*' -sS -b "$cookies" \
  -o "$scratch/activation.json" -w '%{http_code}' \
  -X POST -H "Origin: $origin" -H "X-CSRF-Token: $csrf_token" \
  -H "If-Match: \"$revision\"" \
  "$origin/api/v1/edge-nodes/$edge_node_ref/activation")
[[ "$activation_code" == 202 ]]

stage="waiting for changing sample history"
received=false
for _ in $(seq 1 90); do
  now=$(date +%s%3N)
  from=$((now - 120000))
  curl --noproxy '*' -fsS -b "$cookies" \
    "$origin/api/v1/history?from=$from&to=$now&limit=20" >"$scratch/history.json"
  if jq -e '.items | length >= 2' "$scratch/history.json" >/dev/null 2>&1; then
    received=true
    break
  fi
  sleep 1
done
[[ "$received" == true ]]
jq -e '[.items[].values[0]] | unique | length >= 2' "$scratch/history.json" >/dev/null
grep -Fq "お試し環境" \
  <(curl --noproxy '*' -fsS -b "$cookies" "$origin/status")

compose=(
  docker compose
  --env-file "$state/trial.env"
  --file "$repo_root/deploy/compose.trial.yaml"
)
stage="enqueuing a custody smoke record"
smoke=$("${compose[@]}" exec -T edge-node \
  iotkit-edge-nodectl --db /data/node.db smoke enqueue)
ledger_epoch=$(jq -er '.ledger_epoch' <<<"$smoke")
pub_seq=$(jq -er '.pub_seq' <<<"$smoke")
stage="waiting for durable sample delivery"
delivered=false
for _ in $(seq 1 60); do
  smoke_status=$("${compose[@]}" exec -T edge-node \
    iotkit-edge-nodectl --db /data/node.db smoke status \
    --ledger-epoch "$ledger_epoch" --pub-seq "$pub_seq")
  if jq -e '.status == "delivered" and .accepted_through >= .pub_seq' \
    <<<"$smoke_status" >/dev/null; then
    delivered=true
    break
  fi
  sleep 1
done
[[ "$delivered" == true ]]

stage="stopping and resetting the trial environment"
python3 "$repo_root/scripts/iotkit_trial.py" --config "$config" down
if curl --noproxy '*' -fsS --max-time 2 "$origin/login" >/dev/null 2>&1; then
  echo "trial Console remained reachable after down" >&2
  exit 1
fi
python3 "$repo_root/scripts/iotkit_trial.py" --config "$config" \
  reset --confirm-trial-data-loss
[[ ! -e "$XDG_DATA_HOME/iotkit/trial" ]]

echo "IoTKit trial first-run journey passed"
