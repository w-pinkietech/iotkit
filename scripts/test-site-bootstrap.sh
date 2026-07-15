#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
scratch=$(mktemp -d)
output="$scratch/site-install"
project="iotkit-site-bootstrap-test-$$"
port=$((20000 + $$ % 20000))
edge_pid=""
compose_started=false
repo_test_parent=$(mktemp -d "$repo_root/.bootstrap-repo-test.XXXXXX")
repo_symlink_output="$repo_test_parent/symlink-output"

cleanup() {
  if [[ -n "$edge_pid" ]] && kill -0 "$edge_pid" 2>/dev/null; then
    kill -INT "$edge_pid" 2>/dev/null || true
    wait "$edge_pid" 2>/dev/null || true
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

edge_node_id=$(jq -er '.edge_node_id' "$scratch/binding.json")
grep -Fxq "user $edge_node_id" "$output/mosquitto/acl"
grep -Fxq "topic write iotkit/v1/edge-nodes/$edge_node_id/records" "$output/mosquitto/acl"
grep -Fxq 'topic write iotkit/v1/application/production-pulses' "$output/mosquitto/acl"
grep -Fq 'allow_anonymous false' "$output/mosquitto/mosquitto.conf"
grep -Fq 'listener 8883 0.0.0.0' "$output/mosquitto/mosquitto.conf"
grep -Fq "IOTKIT_BROKER_BIND=127.0.0.1" "$output/site.env"
grep -Fq "IOTKIT_BROKER_PORT=$port" "$output/site.env"
grep -Fq 'allow_insecure' "$output/edge-handoff/edge-mqtt.toml" && {
  echo "production Edge fragment enables insecure MQTT" >&2
  exit 1
}

site_password=$(<"$output/secrets/site-mqtt-password")
edge_password=$(<"$output/edge-handoff/mqtt-password")
for public_file in \
  "$output/site.env" "$output/mosquitto/mosquitto.conf" "$output/mosquitto/acl" \
  "$output/edge-handoff/edge-mqtt.toml"; do
  if grep -Fq "$site_password" "$public_file" || grep -Fq "$edge_password" "$public_file"; then
    echo "plaintext credential leaked into generated config: $public_file" >&2
    exit 1
  fi
done
if grep -Fq "$site_password" "$output/mosquitto/passwords" \
  || grep -Fq "$edge_password" "$output/mosquitto/passwords"; then
  echo "Mosquitto password database was not hashed" >&2
  exit 1
fi

docker compose --env-file "$output/site.env" -p "$project" \
  -f "$repo_root/deploy/compose.site.yaml" config >"$scratch/compose.rendered"
if grep -Fq "$site_password" "$scratch/compose.rendered" \
  || grep -Fq "$edge_password" "$scratch/compose.rendered"; then
  echo "plaintext credential leaked into rendered Compose config" >&2
  exit 1
fi

docker compose --env-file "$output/site.env" -p "$project" \
  -f "$repo_root/deploy/compose.site.yaml" up --build --detach
compose_started=true

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
smoke_epoch=$(jq -er '.ledger_epoch' <<<"$smoke_output")
smoke_pub_seq=$(jq -er '.pub_seq' <<<"$smoke_output")
smoke_test_id=$(jq -er '.test_id' <<<"$smoke_output")

delivered=false
for _ in $(seq 1 60); do
  status_output=$("$repo_root/target/debug/iotkit-edgectl" --db "$scratch/edge.db" smoke status \
    --ledger-epoch "$smoke_epoch" --pub-seq "$smoke_pub_seq" 2>/dev/null || true)
  query_output=$(docker compose --env-file "$output/site.env" -p "$project" \
    -f "$repo_root/deploy/compose.site.yaml" exec -T site \
    iotkit-site query --db /data/site.db --limit 10 2>/dev/null || true)
  if jq -e '.status == "delivered"' <<<"$status_output" >/dev/null 2>&1 \
    && grep -Fq "$smoke_test_id" <<<"$query_output"; then
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
echo "Production Site bootstrap TLS slice: OK"
