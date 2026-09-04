#!/usr/bin/env bash
# End-to-end journey (一気通貫テスト) for the redesigned IoTKit (#232, #233):
# trial-sample Input Adapter -> iotkit-edge-node -> Mosquitto -> independent
# consumer (mosquitto_sub). L1 is the minimal loop; L2 injects faults. Both run
# in this one script and every wait is a bounded condition wait.
#
#   L1  heartbeat online, Observation topic/payload shape, accumulated-count
#       equals the rising edges of the state pipeline, measurement follows the
#       deterministic trial-sample waveform, new series starts at 1/0.
#   L2  Broker stop/start (outbox converges, status first, sequence continues),
#       kill -9 and restart (Will, same series), tuning change without restart,
#       explicit reset starts a new series, deletion clears the retained value,
#       storage failure -> degraded -> online, SIGTERM -> graceful offline.
#
# Requirements: bash, python3, mosquitto + mosquitto_pub/mosquitto_sub on PATH,
# and either cargo (builds the two binaries) or IOTKIT_JOURNEY_BIN_DIR pointing
# at a directory with iotkit-edge-node and iotkit-edge-nodectl.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
checker="$repo_root/scripts/journey/check_messages.py"
scratch=$(mktemp -d)
port=$((20000 + RANDOM % 20000))
node_id="journey"
status_topic="iotkit/v1/edge-node/$node_id/status"
observation_topic() { printf 'iotkit/v1/edge-node/%s/observation/%s/%s' "$node_id" "$1" "$2"; }
measurement_topic=$(observation_topic sample-illuminance measurement)
state_topic=$(observation_topic sample-contact state)
count_topic=$(observation_topic sample-cycles accumulated-count)

broker_pid=""
node_pid=""
sub_pids=()
stage="starting"
failures=0

cleanup() {
  local status=$?
  if ((status != 0)); then
    echo "journey failed while: $stage" >&2
    [[ -f "$scratch/node.log" ]] && { echo "--- node.log (tail) ---" >&2; tail -n 40 "$scratch/node.log" >&2; }
    for log in "$scratch"/broker-*.log; do
      [[ -f "$log" ]] && { echo "--- $(basename "$log") (tail) ---" >&2; tail -n 20 "$log" >&2; }
    done
  fi
  [[ -n "$node_pid" ]] && kill -9 "$node_pid" 2>/dev/null || true
  for pid in "${sub_pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  [[ -n "$broker_pid" ]] && kill "$broker_pid" 2>/dev/null || true
  rm -rf "$scratch"
}
trap cleanup EXIT

for tool in python3 mosquitto mosquitto_pub mosquitto_sub; do
  command -v "$tool" >/dev/null || { echo "$tool is required (apt-get install mosquitto mosquitto-clients)" >&2; exit 1; }
done

# ── binaries ─────────────────────────────────────────────────────────────────
stage="building iotkit-edge-node and iotkit-edge-nodectl"
if [[ -n "${IOTKIT_JOURNEY_BIN_DIR:-}" ]]; then
  bin_dir=$IOTKIT_JOURNEY_BIN_DIR
else
  (cd "$repo_root" && cargo build -q -p iotkit-edge-node -p iotkit-edge-nodectl)
  bin_dir="$repo_root/target/debug"
fi
node_bin="$bin_dir/iotkit-edge-node"
nodectl_bin="$bin_dir/iotkit-edge-nodectl"
[[ -x "$node_bin" && -x "$nodectl_bin" ]] || { echo "binaries not found in $bin_dir" >&2; exit 1; }

# ── helpers ──────────────────────────────────────────────────────────────────
check() {
  local name=$1 expected=$2 actual=$3
  if [[ "$expected" == "$actual" ]]; then
    echo "ok   $name"
  else
    echo "FAIL $name" >&2
    echo "     expected: $expected" >&2
    echo "     actual:   $actual" >&2
    failures=$((failures + 1))
  fi
}

# wait_for <seconds> <description> <command...>: polls until the command
# succeeds. Fails the whole journey on timeout: a missing event is a defect.
wait_for() {
  local seconds=$1 what=$2
  shift 2
  local deadline=$((SECONDS + seconds))
  until "$@"; do
    if ((SECONDS >= deadline)); then
      echo "FAIL timed out after ${seconds}s waiting for: $what" >&2
      exit 1
    fi
    sleep 0.2
  done
  echo "ok   $what"
}

sql() { python3 -c 'import sqlite3,sys; c=sqlite3.connect(sys.argv[1], timeout=5); print(c.execute(sys.argv[2]).fetchone()[0])' "$scratch/iotkit.db" "$1"; }
sql_exec() { python3 -c 'import sqlite3,sys; c=sqlite3.connect(sys.argv[1], timeout=5); c.execute(sys.argv[2]); c.commit()' "$scratch/iotkit.db" "$1"; }
outbox_count() { sql "SELECT COUNT(*) FROM observation_outbox"; }
outbox_is() { [[ "$(outbox_count)" == "$1" ]]; }
outbox_at_least() { (( $(outbox_count) >= $1 )); }

# Broker runs: each start writes its own log so ordering after a restart is
# checked against that run alone.
broker_run=0
broker_start() {
  broker_run=$((broker_run + 1))
  cat >"$scratch/mosquitto.conf" <<EOF
listener $port 127.0.0.1
allow_anonymous true
persistence false
log_type all
log_dest file $scratch/broker-$broker_run.log
EOF
  mosquitto -c "$scratch/mosquitto.conf" &
  broker_pid=$!
  wait_for 10 "broker $broker_run accepts connections" \
    mosquitto_pub -h 127.0.0.1 -p "$port" -t "journey/ready" -m ready -q 1
}
broker_stop() {
  kill "$broker_pid"
  wait "$broker_pid" 2>/dev/null || true
  broker_pid=""
}

# subscribe_ready <file>: an independent consumer capturing
# "topic<TAB>retain<TAB>payload" for every IoTKit message. The extra probe
# topic makes the subscription observable; the checks ignore it.
subscribe_ready() {
  local file=$1
  : >"$file"
  mosquitto_sub -h 127.0.0.1 -p "$port" -t "iotkit/v1/edge-node/$node_id/#" -t "journey/probe" -F '%t	%r	%p' >>"$file" 2>/dev/null &
  sub_pids+=("$!")
  local deadline=$((SECONDS + 5))
  until grep -q "^journey/probe" "$file"; do
    mosquitto_pub -h 127.0.0.1 -p "$port" -t "journey/probe" -m probe -q 1 >/dev/null 2>&1 || true
    if ((SECONDS >= deadline)); then
      echo "FAIL subscriber $(basename "$file") did not connect" >&2
      exit 1
    fi
    sleep 0.1
  done
}

# Capture helpers: <topic> <file> [...]
last_payload() { awk -F'\t' -v topic="$1" '$1 == topic { p = $3 } END { print p }' "$2"; }
live_count() { awk -F'\t' -v topic="$1" '$1 == topic && $2 == 0' "$2" | wc -l | tr -d ' '; }
live_at_least() { (( $(live_count "$1" "$2") >= $3 )); }
last_payload_matches() { last_payload "$1" "$2" | grep -Eq -- "$3"; }
any_payload_matches() { awk -F'\t' -v topic="$1" '$1 == topic { print $3 }' "$2" | grep -Eq -- "$3"; }
json_field() { python3 -c 'import json,sys; v=json.load(sys.stdin)[sys.argv[1]]; print(json.dumps(v))' "$1"; }
status_value_is() { last_payload_matches "$status_topic" "$1" "\"value\":\"$2\""; }
count_value_at_least() { local v; v=$(last_payload "$count_topic" "$1" | json_field value 2>/dev/null || echo -1); (( v >= $2 )); }
all_pipelines_live() { live_at_least "$state_topic" "$1" 1 && live_at_least "$count_topic" "$1" 1; }
# Splits a capture into the retained values a late subscriber received and the
# live messages that followed.
split_retained() { awk -F'\t' '$2 == 1' "$1" >"$1.retained"; awk -F'\t' '$2 == 0' "$1" >"$1.live"; }

node_start() {
  IOTKIT_ENABLE_TRIAL_SAMPLE=1 RUST_LOG=info "$node_bin" --config "$scratch/node.toml" >>"$scratch/node.log" 2>&1 &
  node_pid=$!
}
node_alive() { kill -0 "$node_pid" 2>/dev/null; }
node_exited() { ! kill -0 "$node_pid" 2>/dev/null; }

# ── configuration ────────────────────────────────────────────────────────────
echo journey >"$scratch/mqtt-password"
cat >"$scratch/node.toml" <<EOF
[edge_node]
id = "$node_id"
db_path = "$scratch/iotkit.db"
health_json_path = "$scratch/health.json"

[api]
enabled = false

[output.mqtt]
enabled = true
host = "127.0.0.1"
port = $port
password_file = "$scratch/mqtt-password"
allow_insecure = true

[status]
heartbeat_interval = "5s"

[pipelines]
export_path = "$scratch/pipelines.toml"

[adapters.instances.trial_sample]
type = "trial-sample"
config_schema_version = 1
source = "trial:sample"
poll_interval_ms = 250
EOF

cat >"$scratch/pipelines-import.toml" <<'EOF'
[[pipeline]]
id = "sample-illuminance"
kind = "measurement"
unit = "lx"

[pipeline.input]
adapter = "trial_sample"
measurement_key = "illuminance_lux"

[[pipeline]]
id = "sample-contact"
kind = "state"

[pipeline.input]
adapter = "trial_sample"
measurement_key = "contact_state"

[pipeline.detector]
mode = "high-active"
rise_threshold = 0.5
fall_threshold = 0.5

[[pipeline]]
id = "sample-cycles"
kind = "accumulated-count"
trigger = "on-transition"

[pipeline.input]
adapter = "trial_sample"
measurement_key = "contact_state"

[pipeline.detector]
mode = "high-active"
rise_threshold = 0.5
fall_threshold = 0.5
EOF

# ═════════════════════════════════════════════════════════════════════════════
echo "== L1: minimal loop =="
stage="L1: starting the broker and the consumer"
broker_start
subscribe_ready "$scratch/l1.txt"

stage="L1: starting IoTKit"
node_start
wait_for 20 "IoTKit connects and publishes the first heartbeat (online)" status_value_is "$scratch/l1.txt" online

stage="L1: importing the pipelines while IoTKit runs"
"$nodectl_bin" --db "$scratch/iotkit.db" pipeline import --replace-all --export-path "$scratch/pipelines.toml" "$scratch/pipelines-import.toml" >"$scratch/import.json"
# Rising edges arrive every 2 s (4 polls high, 4 low at 250 ms).
wait_for 30 "accumulated-count reaches 3" count_value_at_least "$scratch/l1.txt" 3

stage="L1: checking the consumer capture"
python3 "$checker" --node "$node_id" l1 "$scratch/l1.txt" \
  --measurement sample-illuminance --state sample-contact --count sample-cycles --min-count 3 \
  || failures=$((failures + 1))
wait_for 5 "outbox converges to PUBACK-acknowledged (empty) while connected" outbox_is 0

# ═════════════════════════════════════════════════════════════════════════════
echo "== L2: fault injection =="
stage="L2: Broker outage"
broker_stop
wait_for 15 "publications accumulate in the outbox while the Broker is down" outbox_at_least 8
outage_backlog=$(outbox_count)
wait_for 10 "IoTKit keeps running through the outage" node_alive
broker_start
wait_for 20 "outbox converges after the Broker returns" outbox_is 0
# The Broker's own log is the deterministic record of what IoTKit sent after
# reconnecting: the status comes first, then the backlog.
first_after_reconnect=$(grep -m1 "Received PUBLISH from iotkit-edge-node-$node_id" "$scratch/broker-2.log" || true)
check "status is published before the outbox after reconnecting" \
  "yes" "$([[ "$first_after_reconnect" == *"'$status_topic'"* ]] && echo yes || echo "no: $first_after_reconnect")"
# A late subscriber gets the retained latest values, and the live stream
# continues each of them by exactly one: nothing from the backlog was lost.
subscribe_ready "$scratch/l2-outage.txt"
wait_for 10 "live observations continue after the outage" live_at_least "$measurement_topic" "$scratch/l2-outage.txt" 4
wait_for 15 "state and accumulated-count publish after the outage" all_pipelines_live "$scratch/l2-outage.txt"
split_retained "$scratch/l2-outage.txt"
python3 "$checker" --node "$node_id" retained "$scratch/l2-outage.txt.retained" \
  --expect-topic "$status_topic" --expect-topic "$measurement_topic" \
  --expect-topic "$state_topic" --expect-topic "$count_topic" || failures=$((failures + 1))
python3 "$checker" --node "$node_id" continues "$scratch/l2-outage.txt.retained" "$scratch/l2-outage.txt.live" || failures=$((failures + 1))
python3 "$checker" --node "$node_id" shape "$scratch/l2-outage.txt" || failures=$((failures + 1))

stage="L2: kill -9 and restart"
subscribe_ready "$scratch/l2-kill.txt"
wait_for 10 "observations flow before the kill" live_at_least "$measurement_topic" "$scratch/l2-kill.txt" 2
kill -9 "$node_pid"
wait "$node_pid" 2>/dev/null || true
node_pid=""
wait_for 10 "the Will arrives: offline with null times" \
  last_payload_matches "$status_topic" "$scratch/l2-kill.txt" '^\{"uptime_ms":null,"unix_epoch_ms":null,"value":"offline"\}$'
subscribe_ready "$scratch/l2-restart.txt"
node_start
wait_for 20 "IoTKit is back online after the restart" status_value_is "$scratch/l2-restart.txt" online
wait_for 10 "observations flow after the restart" live_at_least "$measurement_topic" "$scratch/l2-restart.txt" 4
wait_for 15 "state and accumulated-count publish after the restart" all_pipelines_live "$scratch/l2-restart.txt"
# Retained values in the new capture are the last state before the kill; the
# live ones must continue them. The contract allows one duplicate: the
# publication that was in flight when the process died.
split_retained "$scratch/l2-restart.txt"
python3 "$checker" --node "$node_id" continues --allow-duplicate \
  "$scratch/l2-restart.txt.retained" "$scratch/l2-restart.txt.live" || failures=$((failures + 1))

stage="L2: tuning change without restart"
subscribe_ready "$scratch/l2-ops.txt"
sed 's/rise_threshold = 0.5/rise_threshold = 0.7/' "$scratch/pipelines-import.toml" >"$scratch/pipelines-tuned.toml"
"$nodectl_bin" --db "$scratch/iotkit.db" pipeline update --export-path "$scratch/pipelines.toml" "$scratch/pipelines-tuned.toml" >"$scratch/update.json"
check "pipeline update reports no new series for a tuning change" \
  "0" "$(grep -c '"new_series": {' "$scratch/update.json" || true)"
check "pipelines.toml backup carries the new threshold" \
  "yes" "$(grep -q 'rise_threshold = 0.7' "$scratch/pipelines.toml" && echo yes || echo no)"
wait_for 10 "IoTKit keeps running through the change" node_alive
wait_for 10 "accumulated-count keeps publishing after the change" live_at_least "$count_topic" "$scratch/l2-ops.txt" 1
split_retained "$scratch/l2-ops.txt"
python3 "$checker" --node "$node_id" continues "$scratch/l2-ops.txt.retained" "$scratch/l2-ops.txt.live" || failures=$((failures + 1))

stage="L2: explicit reset starts a new series"
series_before=$(last_payload "$count_topic" "$scratch/l2-ops.txt" | json_field series_id)
"$nodectl_bin" --db "$scratch/iotkit.db" pipeline reset --export-path "$scratch/pipelines.toml" sample-cycles >"$scratch/reset.json"
wait_for 10 "the new accumulated-count series publishes sequence 1, value 0" \
  any_payload_matches "$count_topic" "$scratch/l2-ops.txt" '"sequence":1,.*"value":0\}$'
series_after=$(last_payload "$count_topic" "$scratch/l2-ops.txt" | json_field series_id)
check "reset changes the series id" "yes" "$([[ "$series_before" != "$series_after" ]] && echo yes || echo no)"

stage="L2: deletion clears the retained value"
"$nodectl_bin" --db "$scratch/iotkit.db" pipeline delete --export-path "$scratch/pipelines.toml" sample-illuminance >"$scratch/delete.json"
wait_for 10 "the live consumer receives the zero-length payload" \
  any_payload_matches "$measurement_topic" "$scratch/l2-ops.txt" '^$'
late=$(mosquitto_sub -h 127.0.0.1 -p "$port" -t "$measurement_topic" -C 1 -W 2 -F '%t' 2>/dev/null || true)
check "a late subscriber receives no retained value for the deleted pipeline" "" "$late"

stage="L2: storage failure -> degraded -> online"
sql_exec "CREATE TRIGGER journey_storage_fault BEFORE INSERT ON readings BEGIN SELECT RAISE(FAIL, 'injected storage failure'); END;"
wait_for 15 "status switches to degraded with a storage-write-failed fault" \
  last_payload_matches "$status_topic" "$scratch/l2-ops.txt" '"value":"degraded","faults":\[\{"kind":"storage-write-failed","since_uptime_ms":[0-9]+,"since_unix_epoch_ms":(null|[0-9]+),"count":[0-9]+'
sql_exec "DROP TRIGGER journey_storage_fault"
wait_for 30 "status returns to online with faults [] once a write succeeds" \
  last_payload_matches "$status_topic" "$scratch/l2-ops.txt" '"value":"online","faults":\[\]\}$'

stage="L2: graceful shutdown"
kill -TERM "$node_pid"
wait_for 15 "IoTKit exits on SIGTERM" node_exited
node_status=0
wait "$node_pid" || node_status=$?
node_pid=""
check "IoTKit exits with status 0 on a requested shutdown" "0" "$node_status"
wait_for 5 "the graceful offline arrives with the shutdown time and faults" \
  last_payload_matches "$status_topic" "$scratch/l2-ops.txt" '^\{"uptime_ms":[0-9]+,"unix_epoch_ms":(null|[0-9]+),"value":"offline","faults":\[\]\}$'
check "no Will after a graceful shutdown" "0" "$(grep -c '"uptime_ms":null' "$scratch/l2-ops.txt" || true)"
python3 "$checker" --node "$node_id" shape "$scratch/l2-ops.txt" || failures=$((failures + 1))

stage="summarizing"
if ((failures > 0)); then
  echo "journey: $failures failure(s)" >&2
  exit 1
fi
echo "journey: OK (L1 minimal loop, L2 fault injection)"
