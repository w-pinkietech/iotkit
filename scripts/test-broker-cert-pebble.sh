#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
scratch=$(mktemp -d "$repo_root/.pebble-test.XXXXXX")
project="iotkit-pebble-test-$$"
lego_container="iotkit-lego-bin-$$"
lego_image="docker.io/goacme/lego:v4.35.2@sha256:ae124a405844759b201b31efbd7a0ba302dbd16e86f2fb177c4b6db8bdc782c8"
base_port=$((30000 + $$ % 10000))
octet=$((20 + $$ % 180))
export IOTKIT_PEBBLE_PORT=$base_port
export IOTKIT_PEBBLE_MGMT_PORT=$((base_port + 1))
export IOTKIT_PEBBLE_DNS_PORT=$((base_port + 2))
export IOTKIT_PEBBLE_CA_PORT=$((base_port + 3))
export IOTKIT_PEBBLE_SUBNET="10.31.$octet.0/24"
export IOTKIT_PEBBLE_IP="10.31.$octet.2"
export IOTKIT_PEBBLE_CHALL_IP="10.31.$octet.3"

cleanup() {
  docker rm -f "$lego_container" >/dev/null 2>&1 || true
  docker compose -p "$project" -f "$repo_root/deploy/compose.pebble.yaml" \
    down --volumes --remove-orphans >/dev/null 2>&1 || true
  chmod -R u+w "$scratch" 2>/dev/null || true
  rm -rf "$scratch"
}
trap cleanup EXIT

docker compose -p "$project" -f "$repo_root/deploy/compose.pebble.yaml" \
  up --detach
for _ in $(seq 1 60); do
  if curl -ksSf "https://127.0.0.1:$IOTKIT_PEBBLE_PORT/dir" >/dev/null; then
    break
  fi
  sleep 1
done
curl -ksSf "https://127.0.0.1:$IOTKIT_PEBBLE_PORT/dir" >/dev/null
docker compose -p "$project" -f "$repo_root/deploy/compose.pebble.yaml" \
  cp pebble:/test/certs/pebble.minica.pem "$scratch/pebble.minica.pem"
curl -ksSf "https://127.0.0.1:$IOTKIT_PEBBLE_CA_PORT/roots/0" \
  >"$scratch/pebble-root.pem"

mkdir -p "$scratch/lego-data"
docker create --name "$lego_container" "$lego_image" --version >/dev/null
docker cp "$lego_container:/lego" "$scratch/lego-bin"
docker rm "$lego_container" >/dev/null
chmod 700 "$scratch/lego-bin"
"$scratch/lego-bin" --version | grep -Fq 'version 4.35.2'
cat >"$scratch/dns-hook.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
action=$1
host=$2
value=$3
case "$action" in
  present)
    curl -fsS -X POST -H 'Content-Type: application/json' \
      --data "{\"host\":\"$host\",\"value\":\"$value\"}" \
      "$CHALLTESTSRV_URL/set-txt" >/dev/null
    ;;
  cleanup)
    curl -fsS -X POST -H 'Content-Type: application/json' \
      --data "{\"host\":\"$host\"}" \
      "$CHALLTESTSRV_URL/clear-txt" >/dev/null
    ;;
  *) exit 2 ;;
esac
EOF
chmod +x "$scratch/dns-hook.sh"

export LEGO_CA_CERTIFICATES="$scratch/pebble.minica.pem"
export EXEC_PATH="$scratch/dns-hook.sh"
export EXEC_POLLING_INTERVAL=1
export EXEC_PROPAGATION_TIMEOUT=30
export CHALLTESTSRV_URL="http://127.0.0.1:$IOTKIT_PEBBLE_MGMT_PORT"
"$scratch/lego-bin" --path "$scratch/lego-data" \
  --server "https://localhost:$IOTKIT_PEBBLE_PORT/dir" \
  --email test@iotkit.invalid --domains localhost --accept-tos \
  --dns exec --dns.resolvers "127.0.0.1:$IOTKIT_PEBBLE_DNS_PORT" \
  --dns.propagation-wait 1s run

cert="$scratch/lego-data/certificates/localhost.crt"
key="$scratch/lego-data/certificates/localhost.key"
issuer="$scratch/lego-data/certificates/localhost.issuer.crt"
test -s "$cert" -a -s "$key" -a -s "$issuer"
chmod 600 "$key"
touch "$scratch/edge.env" "$scratch/compose.yaml" "$scratch/password"
cat >"$scratch/cert.env" <<EOF
IOTKIT_CERT_DOMAIN=localhost
IOTKIT_CERT_FILE=$cert
IOTKIT_CERT_KEY_FILE=$key
IOTKIT_CERT_CA_FILE=$scratch/pebble-root.pem
IOTKIT_CERT_EDGE_ENV=$scratch/edge.env
IOTKIT_CERT_COMPOSE_FILE=$scratch/compose.yaml
IOTKIT_CERT_BROKER_PORT=18883
IOTKIT_CERT_EDGE_ARCHIVE_PASSWORD_FILE=$scratch/password
EOF
chmod 600 "$scratch/cert.env"
"$repo_root/scripts/iotkit-broker-cert" status --config "$scratch/cert.env" \
  | jq -e '.domain == "localhost" and .state == "valid"' >/dev/null

serial_before=$(openssl x509 -in "$cert" -noout -serial)
"$scratch/lego-bin" --path "$scratch/lego-data" \
  --server "https://localhost:$IOTKIT_PEBBLE_PORT/dir" \
  --email test@iotkit.invalid --domains localhost \
  --dns exec --dns.resolvers "127.0.0.1:$IOTKIT_PEBBLE_DNS_PORT" \
  --dns.propagation-wait 1s \
  renew --days 100 --ari-disable --no-random-sleep
serial_after=$(openssl x509 -in "$cert" -noout -serial)
[[ "$serial_before" != "$serial_after" ]] || {
  echo "Pebble renewal did not replace the certificate" >&2
  exit 1
}

echo "Pebble ACME issuance and renewal test passed"
