#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
scratch=$(mktemp -d)
output="$scratch/site-install"
project="iotkit-site-bootstrap-test-$$"
port=$((20000 + $$ % 20000))
site_port=$((port + 1))
edge_pid=""
edge2_pid=""
compose_started=false
repo_test_parent=$(mktemp -d "$repo_root/.bootstrap-repo-test.XXXXXX")
repo_symlink_output="$repo_test_parent/symlink-output"

cleanup() {
  if [[ -n "$edge_pid" ]] && kill -0 "$edge_pid" 2>/dev/null; then
    kill -INT "$edge_pid" 2>/dev/null || true
    wait "$edge_pid" 2>/dev/null || true
  fi
  if [[ -n "$edge2_pid" ]] && kill -0 "$edge2_pid" 2>/dev/null; then
    kill -INT "$edge2_pid" 2>/dev/null || true
    wait "$edge2_pid" 2>/dev/null || true
  fi
  if [[ "$compose_started" == true ]]; then
    docker compose --env-file "$output/site.env" -p "$project" \
      -f "$repo_root/deploy/compose.site.yaml" down --volumes --remove-orphans >/dev/null 2>&1 || true
  fi
  rm -rf "$repo_test_parent"
  rm -rf "$scratch"
}
trap cleanup EXIT

openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj '/CN=IoTKit Test CA' \
  -keyout "$scratch/ca.key" -out "$scratch/ca.pem" >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes -subj '/CN=localhost' \
  -keyout "$scratch/server.key" -out "$scratch/server.csr" >/dev/null 2>&1
printf 'subjectAltName=DNS:localhost\nextendedKeyUsage=serverAuth\n' >"$scratch/server.ext"
openssl x509 -req -days 2 -in "$scratch/server.csr" -CA "$scratch/ca.pem" \
  -CAkey "$scratch/ca.key" -CAcreateserial -extfile "$scratch/server.ext" \
  -out "$scratch/server.pem" >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes -subj '/CN=not-localhost' \
  -keyout "$scratch/wrong-host.key" -out "$scratch/wrong-host.csr" >/dev/null 2>&1
printf 'subjectAltName=DNS:not-localhost\nextendedKeyUsage=serverAuth\n' \
  >"$scratch/wrong-host.ext"
openssl x509 -req -days 2 -in "$scratch/wrong-host.csr" -CA "$scratch/ca.pem" \
  -CAkey "$scratch/ca.key" -CAcreateserial -extfile "$scratch/wrong-host.ext" \
  -out "$scratch/wrong-host.pem" >/dev/null 2>&1
chmod 600 "$scratch/ca.key" "$scratch/server.key" "$scratch/wrong-host.key"

cargo build --manifest-path "$repo_root/Cargo.toml" -p iotkit-edge -p iotkit-edgectl
"$repo_root/target/debug/iotkit-edgectl" --db "$scratch/edge.db" init >/dev/null
"$repo_root/target/debug/iotkit-edgectl" --db "$scratch/edge.db" mqtt-binding \
  >"$scratch/binding.json"

expect_bootstrap_failure() {
  local name=$1 expected_output=$2
  shift 2
  if "$repo_root/scripts/bootstrap-site.sh" "$@" \
    >"$scratch/$name.stdout" 2>"$scratch/$name.stderr"; then
    echo "bootstrap unexpectedly accepted $name" >&2
    exit 1
  fi
  [[ ! -e "$expected_output" ]] || {
    echo "failed bootstrap left output behind: $expected_output" >&2
    exit 1
  }
}

jq '.password = "must-not-leak-test-secret"' "$scratch/binding.json" \
  >"$scratch/binding-with-secret.json"
expect_bootstrap_failure invalid-binding "$scratch/invalid-binding-output" \
  --binding "$scratch/binding-with-secret.json" \
  --output-dir "$scratch/invalid-binding-output" \
  --broker-host localhost --broker-bind 127.0.0.1 --broker-port "$port" \
  --tls-cert "$scratch/server.pem" --tls-key "$scratch/server.key" --tls-ca "$scratch/ca.pem"
if grep -Fq 'must-not-leak-test-secret' "$scratch/invalid-binding.stdout" \
  || grep -Fq 'must-not-leak-test-secret' "$scratch/invalid-binding.stderr"; then
  echo "invalid binding secret leaked through bootstrap diagnostics" >&2
  exit 1
fi

cp "$scratch/server.key" "$scratch/unsafe-server.key"
chmod 644 "$scratch/unsafe-server.key"
expect_bootstrap_failure unsafe-key "$scratch/unsafe-key-output" \
  --binding "$scratch/binding.json" --output-dir "$scratch/unsafe-key-output" \
  --broker-host localhost --broker-bind 127.0.0.1 --broker-port "$port" \
  --tls-cert "$scratch/server.pem" --tls-key "$scratch/unsafe-server.key" --tls-ca "$scratch/ca.pem"

expect_bootstrap_failure unsafe-output-path "$scratch/unsafe#output" \
  --binding "$scratch/binding.json" --output-dir "$scratch/unsafe#output" \
  --broker-host localhost --broker-bind 127.0.0.1 --broker-port "$port" \
  --tls-cert "$scratch/server.pem" --tls-key "$scratch/server.key" --tls-ca "$scratch/ca.pem"

expect_bootstrap_failure wrong-bind "$scratch/wrong-bind-output" \
  --binding "$scratch/binding.json" --output-dir "$scratch/wrong-bind-output" \
  --broker-host localhost --broker-bind 127.0.0.2 --broker-port "$port" \
  --tls-cert "$scratch/server.pem" --tls-key "$scratch/server.key" --tls-ca "$scratch/ca.pem"

expect_bootstrap_failure wrong-certificate-host "$scratch/wrong-certificate-host-output" \
  --binding "$scratch/binding.json" --output-dir "$scratch/wrong-certificate-host-output" \
  --broker-host localhost --broker-bind 127.0.0.1 --broker-port "$port" \
  --tls-cert "$scratch/wrong-host.pem" --tls-key "$scratch/wrong-host.key" \
  --tls-ca "$scratch/ca.pem"

repo_output="$repo_test_parent/direct-output"
[[ ! -e "$repo_output" ]] || { echo "reserved test output already exists" >&2; exit 1; }
expect_bootstrap_failure repository-output "$repo_output" \
  --binding "$scratch/binding.json" --output-dir "$repo_output" \
  --broker-host localhost --broker-bind 127.0.0.1 --broker-port "$port" \
  --tls-cert "$scratch/server.pem" --tls-key "$scratch/server.key" --tls-ca "$scratch/ca.pem"

ln -s "$repo_root/scripts/bootstrap-site.sh" "$scratch/bootstrap-site-link"
[[ ! -e "$repo_symlink_output" ]] || { echo "reserved test output already exists" >&2; exit 1; }
if "$scratch/bootstrap-site-link" \
  --binding "$scratch/binding.json" --output-dir "$repo_symlink_output" \
  --broker-host localhost --broker-bind 127.0.0.1 --broker-port "$port" \
  --tls-cert "$scratch/server.pem" --tls-key "$scratch/server.key" --tls-ca "$scratch/ca.pem" \
  >"$scratch/repository-output-link.stdout" 2>"$scratch/repository-output-link.stderr"; then
  echo "bootstrap invoked through a symlink accepted repository output" >&2
  exit 1
fi
[[ ! -e "$repo_symlink_output" ]] || {
  echo "failed symlink bootstrap left repository output behind" >&2
  exit 1
}

"$repo_root/scripts/bootstrap-site.sh" \
  --binding "$scratch/binding.json" \
  --output-dir "$output" \
  --broker-host localhost \
  --broker-bind 127.0.0.1 \
  --broker-port "$port" \
  --site-https-port "$site_port" \
  --tls-cert "$scratch/server.pem" \
  --tls-key "$scratch/server.key" \
  --tls-ca "$scratch/ca.pem" \
  --site-publish-topic iotkit/v1/application/production-pulses >/dev/null

site_env_before=$(sha256sum "$output/site.env")
if "$repo_root/scripts/bootstrap-site.sh" \
  --binding "$scratch/binding.json" \
  --output-dir "$output" \
  --broker-host localhost \
  --broker-bind 127.0.0.1 \
  --broker-port "$port" \
  --site-https-port "$site_port" \
  --tls-cert "$scratch/server.pem" \
  --tls-key "$scratch/server.key" \
  --tls-ca "$scratch/ca.pem" \
  >"$scratch/existing-output.stdout" 2>"$scratch/existing-output.stderr"; then
  echo "bootstrap unexpectedly replaced an existing output directory" >&2
  exit 1
fi
[[ "$(sha256sum "$output/site.env")" == "$site_env_before" ]] || {
  echo "failed bootstrap changed an existing output directory" >&2
  exit 1
}

for path in \
  "$output/site.env" \
  "$output/mosquitto/mosquitto.conf" \
  "$output/mosquitto/acl" \
  "$output/mosquitto/passwords" \
  "$output/secrets/site-mqtt-password" \
  "$output/secrets/output-mqtt-password" \
  "$output/tls/server.pem" \
  "$output/tls/server.key" \
  "$output/tls/ca.pem" \
  "$output/edge-handoff/mqtt-password" \
  "$output/edge-handoff/broker-ca.pem" \
  "$output/edge-handoff/edge-mqtt.toml"; do
  [[ -f "$path" ]] || { echo "missing generated file: $path" >&2; exit 1; }
  [[ "$(stat -c %a "$path")" == "600" ]] || {
    echo "unsafe generated file mode: $path" >&2
    exit 1
  }
done

"$repo_root/target/debug/iotkit-edgectl" --db "$scratch/edge2.db" init >/dev/null
"$repo_root/target/debug/iotkit-edgectl" --db "$scratch/edge2.db" mqtt-binding \
  >"$scratch/binding2.json"
"$repo_root/scripts/add-site-edge.sh" \
  --binding "$scratch/binding2.json" --site-dir "$output" >/dev/null
edge_node_id2=$(jq -er '.edge_node_id' "$scratch/binding2.json")
grep -Fxq "user $edge_node_id2" "$output/mosquitto/acl"
[[ -f "$output/edge-handoff/$edge_node_id2/mqtt-password" ]]
[[ -f "$output/edge-handoff/$edge_node_id2/edge-mqtt.toml" ]]

edge_node_id=$(jq -er '.edge_node_id' "$scratch/binding.json")
grep -Fxq "user $edge_node_id" "$output/mosquitto/acl"
grep -Fxq "topic write iotkit/v1/edge-nodes/$edge_node_id/records" "$output/mosquitto/acl"
grep -Fxq "topic write iotkit/v1/edge-nodes/$edge_node_id/descriptors" "$output/mosquitto/acl"
grep -Fxq "topic read iotkit/v1/edge-nodes/+/descriptors" "$output/mosquitto/acl"
grep -Fxq 'topic write iotkit/v1/application/production-pulses' "$output/mosquitto/acl"
grep -Fq 'allow_anonymous false' "$output/mosquitto/mosquitto.conf"
grep -Fq 'listener 8883 0.0.0.0' "$output/mosquitto/mosquitto.conf"
grep -Fq "IOTKIT_BROKER_BIND=127.0.0.1" "$output/site.env"
grep -Fq "IOTKIT_BROKER_PORT=$port" "$output/site.env"
grep -Fxq 'IOTKIT_MOSQUITTO_IMAGE=eclipse-mosquitto:2.0.22' "$output/site.env"
for setting in \
  'message_size_limit 1048576' \
  'max_packet_size 1114112' \
  'max_inflight_messages 20' \
  'max_queued_messages 1000' \
  'max_connections 128' \
  'memory_limit 268435456'; do
  grep -Fxq "$setting" "$output/mosquitto/mosquitto.conf" || {
    echo "missing Mosquitto limit: $setting" >&2
    exit 1
  }
done
grep -Fq 'allow_insecure' "$output/edge-handoff/edge-mqtt.toml" && {
  echo "production Edge fragment enables insecure MQTT" >&2
  exit 1
}
grep -Fxq 'trust_mode = "bundle_only"' "$output/edge-handoff/edge-mqtt.toml"

site_password=$(<"$output/secrets/site-mqtt-password")
output_password=$(<"$output/secrets/output-mqtt-password")
edge_password=$(<"$output/edge-handoff/mqtt-password")
for public_file in \
  "$output/site.env" "$output/mosquitto/mosquitto.conf" "$output/mosquitto/acl" \
  "$output/edge-handoff/edge-mqtt.toml"; do
  if grep -Fq "$site_password" "$public_file" || grep -Fq "$output_password" "$public_file" \
    || grep -Fq "$edge_password" "$public_file"; then
    echo "plaintext credential leaked into generated config: $public_file" >&2
    exit 1
  fi
done
if grep -Fq "$site_password" "$output/mosquitto/passwords" \
  || grep -Fq "$output_password" "$output/mosquitto/passwords" \
  || grep -Fq "$edge_password" "$output/mosquitto/passwords"; then
  echo "Mosquitto password database was not hashed" >&2
  exit 1
fi

docker compose --env-file "$output/site.env" -p "$project" \
  -f "$repo_root/deploy/compose.site.yaml" config >"$scratch/compose.rendered"
grep -Fq 'image: eclipse-mosquitto:2.0.22' "$scratch/compose.rendered"
grep -Fq 'no-new-privileges:true' "$scratch/compose.rendered"
grep -Fq 'pids_limit: 128' "$scratch/compose.rendered"
grep -Eq 'mem_limit: ("?268435456"?|256m)' "$scratch/compose.rendered"
grep -A2 -F 'cap_drop:' "$scratch/compose.rendered" | grep -Fq 'ALL'
if grep -Fq "$site_password" "$scratch/compose.rendered" \
  || grep -Fq "$output_password" "$scratch/compose.rendered" \
  || grep -Fq "$edge_password" "$scratch/compose.rendered"; then
  echo "plaintext credential leaked into rendered Compose config" >&2
  exit 1
fi

docker compose --env-file "$output/site.env" -p "$project" \
  -f "$repo_root/deploy/compose.site.yaml" up --build --detach
compose_started=true

openssl rand -base64 24 >"$scratch/admin-password"
chmod 600 "$scratch/admin-password"
docker compose --env-file "$output/site.env" -p "$project" \
  -f "$repo_root/deploy/compose.site.yaml" run --rm \
  -v "$scratch/admin-password:/run/iotkit/admin-password:ro" \
  site account bootstrap --db /data/site.db --login-id admin \
  --display-name '試験管理者' --password-file /run/iotkit/admin-password >/dev/null
admin_password=$(<"$scratch/admin-password")
login_payload=$(jq -nc --arg password "$admin_password" \
  '{login_id:"admin", password:$password}')
for _ in $(seq 1 60); do
  login_code=$(curl -sS --cacert "$scratch/ca.pem" \
    -c "$scratch/cookies" -o "$scratch/login-response.json" -w '%{http_code}' \
    -H "Origin: https://localhost:$site_port" \
    -H 'Content-Type: application/json' --data "$login_payload" \
    "https://localhost:$site_port/api/v1/session" || true)
  [[ "$login_code" == 201 ]] && break
  sleep 1
done
[[ "${login_code:-}" == 201 ]] || {
  echo "Caddy HTTPS Site login failed: ${login_code:-no response}" >&2
  exit 1
}
curl -sS --cacert "$scratch/ca.pem" -b "$scratch/cookies" \
  "https://localhost:$site_port/status" |
  grep -Fq '現場の状態'
if curl -sS "http://localhost:$site_port/status" 2>/dev/null |
  grep -Fq '現場の状態'; then
  echo "Site Console was served as plaintext HTTP" >&2
  exit 1
fi

cat >"$scratch/edge.toml" <<EOF
[edge]
db_path = "$scratch/edge.db"
health_json_path = "$scratch/health.json"

[adapters.bravepi]
enabled = false

[adapters.rpi_local]
enabled = false

[api]
enabled = false
EOF
sed \
  -e "s|/etc/iotkit/mqtt-password|$output/edge-handoff/mqtt-password|" \
  -e "s|/etc/iotkit/broker-ca.pem|$output/edge-handoff/broker-ca.pem|" \
  "$output/edge-handoff/edge-mqtt.toml" >>"$scratch/edge.toml"

"$repo_root/target/debug/iotkit-edge" --config "$scratch/edge.toml" \
  >"$scratch/edge.log" 2>&1 &
edge_pid=$!

cat >"$scratch/edge2.toml" <<EOF
[edge]
db_path = "$scratch/edge2.db"
health_json_path = "$scratch/health2.json"

[adapters.bravepi]
enabled = false

[adapters.rpi_local]
enabled = false

[api]
enabled = false
EOF
sed \
  -e "s|/etc/iotkit/mqtt-password|$output/edge-handoff/$edge_node_id2/mqtt-password|" \
  -e "s|/etc/iotkit/broker-ca.pem|$output/edge-handoff/$edge_node_id2/broker-ca.pem|" \
  "$output/edge-handoff/$edge_node_id2/edge-mqtt.toml" >>"$scratch/edge2.toml"
"$repo_root/target/debug/iotkit-edge" --config "$scratch/edge2.toml" \
  >"$scratch/edge2.log" 2>&1 &
edge2_pid=$!

smoke_output=""
for _ in $(seq 1 60); do
  if smoke_output=$("$repo_root/target/debug/iotkit-edgectl" \
    --db "$scratch/edge.db" smoke enqueue 2>/dev/null); then
    break
  fi
  sleep 1
done
[[ -n "$smoke_output" ]] || {
  docker compose --env-file "$output/site.env" -p "$project" \
    -f "$repo_root/deploy/compose.site.yaml" logs broker site
  sed -n '1,200p' "$scratch/edge.log"
  echo "TLS commissioning smoke could not be enqueued" >&2
  exit 1
}
smoke_output2=""
for _ in $(seq 1 60); do
  if smoke_output2=$("$repo_root/target/debug/iotkit-edgectl" \
    --db "$scratch/edge2.db" smoke enqueue 2>/dev/null); then
    break
  fi
  sleep 1
done
[[ -n "$smoke_output2" ]] || {
  sed -n '1,200p' "$scratch/edge2.log"
  echo "second Edge TLS commissioning smoke could not be enqueued" >&2
  exit 1
}
smoke_epoch=$(jq -er '.ledger_epoch' <<<"$smoke_output")
smoke_pub_seq=$(jq -er '.pub_seq' <<<"$smoke_output")
smoke_test_id=$(jq -er '.test_id' <<<"$smoke_output")
smoke_epoch2=$(jq -er '.ledger_epoch' <<<"$smoke_output2")
smoke_pub_seq2=$(jq -er '.pub_seq' <<<"$smoke_output2")
smoke_test_id2=$(jq -er '.test_id' <<<"$smoke_output2")

delivered=false
for _ in $(seq 1 60); do
  status_output=$("$repo_root/target/debug/iotkit-edgectl" --db "$scratch/edge.db" smoke status \
    --ledger-epoch "$smoke_epoch" --pub-seq "$smoke_pub_seq" 2>/dev/null || true)
  status_output2=$("$repo_root/target/debug/iotkit-edgectl" --db "$scratch/edge2.db" smoke status \
    --ledger-epoch "$smoke_epoch2" --pub-seq "$smoke_pub_seq2" 2>/dev/null || true)
  query_output=$(docker compose --env-file "$output/site.env" -p "$project" \
    -f "$repo_root/deploy/compose.site.yaml" exec -T site \
    iotkit-site query --db /data/site.db --limit 10 2>/dev/null || true)
  if jq -e '.status == "delivered"' <<<"$status_output" >/dev/null 2>&1 \
    && jq -e '.status == "delivered"' <<<"$status_output2" >/dev/null 2>&1 \
    && grep -Fq "$smoke_test_id" <<<"$query_output" \
    && grep -Fq "$smoke_test_id2" <<<"$query_output"; then
    delivered=true
    break
  fi
  sleep 1
done
[[ "$delivered" == true ]] || {
  docker compose --env-file "$output/site.env" -p "$project" \
    -f "$repo_root/deploy/compose.site.yaml" logs broker site
  sed -n '1,200p' "$scratch/edge.log"
  echo "TLS commissioning smoke did not reach Site custody" >&2
  exit 1
}

kill -INT "$edge_pid"
wait "$edge_pid"
edge_pid=""
kill -INT "$edge2_pid"
wait "$edge2_pid"
edge2_pid=""
echo "Production Site bootstrap TLS slice: OK"
