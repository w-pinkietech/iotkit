#!/usr/bin/env bash
# Consumer-side conformance for the MQTT Output Adapter v1 contract.
#
# Publishes every fixture under testdata/observation/v1 to a throwaway
# Mosquitto exactly as IoTKit would (topic, QoS 1, retain) and checks what a
# subscriber receives: the same topic, the same payload bytes, and the retain
# semantics the contract promises (a new subscriber gets the latest value; a
# zero-length retained payload clears it).
#
# This is the receiving half of the end-to-end journey. #232 child issue 4
# replaces mosquitto_pub with the IoTKit process.
#
# Broker: a local `mosquitto` binary when present, otherwise the pinned
# eclipse-mosquitto image through docker. Clients: mosquitto_pub/mosquitto_sub
# from the same source.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
fixture_dir="$repo_root/testdata/observation/v1"
scratch=$(mktemp -d)
port=$((20000 + RANDOM % 20000))
container=""
broker_pid=""
stage="starting"
failures=0

cleanup() {
  local status=$?
  if ((status != 0)); then
    echo "observation consumer check failed while: $stage" >&2
    [[ -f "$scratch/mosquitto.log" ]] && sed -n '1,40p' "$scratch/mosquitto.log" >&2
  fi
  [[ -n "$broker_pid" ]] && kill "$broker_pid" 2>/dev/null || true
  [[ -n "$container" ]] && docker rm -f "$container" >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT

command -v python3 >/dev/null || { echo "python3 is required" >&2; exit 1; }

if command -v mosquitto >/dev/null && command -v mosquitto_pub >/dev/null && command -v mosquitto_sub >/dev/null; then
  mode=local
  cat >"$scratch/mosquitto.conf" <<EOF
listener $port 127.0.0.1
allow_anonymous true
persistence false
log_dest file $scratch/mosquitto.log
EOF
  mosquitto -c "$scratch/mosquitto.conf" &
  broker_pid=$!
  pub() { mosquitto_pub -h 127.0.0.1 -p "$port" "$@"; }
  sub() { mosquitto_sub -h 127.0.0.1 -p "$port" "$@"; }
else
  command -v docker >/dev/null || {
    echo "neither mosquitto (with mosquitto_pub/mosquitto_sub) nor docker is available" >&2
    exit 1
  }
  mode=docker
  # shellcheck disable=SC1091
  source "$repo_root/deploy/mosquitto-image.env"
  cat >"$scratch/mosquitto.conf" <<EOF
listener 1883 0.0.0.0
allow_anonymous true
persistence false
log_dest stdout
EOF
  container="iotkit-observation-consumer-$$"
  docker run -d --rm --name "$container" \
    -v "$scratch/mosquitto.conf:/mosquitto/config/mosquitto.conf:ro" \
    "$IOTKIT_MOSQUITTO_IMAGE" >/dev/null
  pub() { docker exec "$container" mosquitto_pub -h 127.0.0.1 -p 1883 "$@"; }
  sub() { docker exec "$container" mosquitto_sub -h 127.0.0.1 -p 1883 "$@"; }
fi

stage="waiting for the broker"
for _ in $(seq 1 50); do
  if pub -t "iotkit-test/ready" -m ready -q 1 >/dev/null 2>&1; then
    break
  fi
  sleep 0.2
done
pub -t "iotkit-test/ready" -m ready -q 1 >/dev/null

# Reads one field of a fixture file.
field() { python3 -c 'import json,sys; v=json.load(open(sys.argv[1]))[sys.argv[2]]; print(v if not isinstance(v,bool) else str(v).lower())' "$1" "$2"; }
# Lowercase hex of the exact payload bytes, as mosquitto_sub -F %x prints them.
payload_hex() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["payload"].encode("utf-8").hex())' "$1"; }

# Publishes a fixture the way IoTKit would: QoS 1, retain, exact bytes.
publish_fixture() {
  local file=$1 topic payload
  topic=$(field "$file" topic)
  payload=$(field "$file" payload)
  if [[ -z "$payload" ]]; then
    pub -t "$topic" -q 1 -r -n
  else
    pub -t "$topic" -q 1 -r -m "$payload"
  fi
}

# Subscribes after publication and returns "topic<TAB>retain<TAB>hexpayload"
# of the first message, or "none" when nothing arrives within the timeout.
# %x prints the payload as hex, so a zero-length payload compares as "".
receive_retained() {
  local topic=$1 out
  out=$(sub -t "$topic" -C 1 -W 3 -F '%t	%r	%x' 2>/dev/null || true)
  [[ -n "$out" ]] && printf '%s' "$out" || printf 'none'
}

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

stage="checking retained delivery of every non-empty fixture"
for file in "$fixture_dir"/*.json; do
  name=$(basename "$file" .json)
  [[ "$name" == *.schema ]] && continue
  payload=$(field "$file" payload)
  [[ -z "$payload" ]] && continue
  topic=$(field "$file" topic)
  publish_fixture "$file"
  # A consumer that subscribes after the publication gets the retained latest
  # value, flagged as retained, with the exact bytes IoTKit sent.
  check "$name: late subscriber receives retained bytes" \
    "$(printf '%s\t1\t%s' "$topic" "$(payload_hex "$file")")" \
    "$(receive_retained "$topic")"
done

stage="checking that a new series replaces the retained accumulated-count"
count_file="$fixture_dir/observation-accumulated-count.json"
new_series_file="$fixture_dir/observation-accumulated-count-new-series.json"
count_topic=$(field "$count_file" topic)
publish_fixture "$count_file"
publish_fixture "$new_series_file"
check "new series: retained value is sequence 1 / value 0 of the new series" \
  "$(printf '%s\t1\t%s' "$count_topic" "$(payload_hex "$new_series_file")")" \
  "$(receive_retained "$count_topic")"

stage="checking that pipeline deletion clears the retained value"
deleted_file="$fixture_dir/observation-deleted.json"
deleted_topic=$(field "$deleted_file" topic)
publish_fixture "$count_file"
# A live subscriber sees the zero-length payload itself.
sub -t "$deleted_topic" -C 2 -W 5 -F '%t	%l' >"$scratch/live.txt" 2>/dev/null &
live_pid=$!
sleep 0.5
publish_fixture "$deleted_file"
wait "$live_pid" || true
check "deletion: live subscriber receives a zero-length payload" \
  "$(printf '%s\t%s\n%s\t0' "$deleted_topic" "$(printf '%s' "$(field "$count_file" payload)" | wc -c | tr -d ' ')" "$deleted_topic")" \
  "$(cat "$scratch/live.txt")"
# A consumer that subscribes afterwards gets nothing: the Broker holds no value.
check "deletion: late subscriber receives no retained value" "none" "$(receive_retained "$deleted_topic")"

stage="checking the Will payload as the Broker would publish it"
will_file="$fixture_dir/status-offline-will.json"
status_topic=$(field "$will_file" topic)
publish_fixture "$fixture_dir/status-online.json"
# Simulate the Broker acting on the Will: same topic, retain, null uptime_ms and unix_epoch_ms.
publish_fixture "$will_file"
check "will: retained status is offline with null times" \
  "$(printf '%s\t1\t%s' "$status_topic" "$(payload_hex "$will_file")")" \
  "$(receive_retained "$status_topic")"

if ((failures > 0)); then
  echo "observation consumer check ($mode): $failures failure(s)" >&2
  exit 1
fi
echo "observation consumer check ($mode): OK"
