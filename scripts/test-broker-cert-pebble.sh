#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
scratch=$(mktemp -d)
project="iotkit-pebble-test-$$"
base_port=$((30000 + $$ % 10000))
octet=$((20 + $$ % 180))
export IOTKIT_PEBBLE_PORT=$base_port
export IOTKIT_PEBBLE_MGMT_PORT=$((base_port + 1))
export IOTKIT_PEBBLE_DNS_PORT=$((base_port + 2))
export IOTKIT_PEBBLE_SUBNET="10.31.$octet.0/24"
export IOTKIT_PEBBLE_IP="10.31.$octet.2"
export IOTKIT_PEBBLE_CHALL_IP="10.31.$octet.3"

cleanup() {
  docker compose -p "$project" -f "$repo_root/deploy/compose.pebble.yaml" \
    down --volumes --remove-orphans >/dev/null 2>&1 || true
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

mkdir -p "$scratch/bin" "$scratch/lego"
GOBIN="$scratch/bin" GOMODCACHE="${GOMODCACHE:-$scratch/go-mod}" \
  go install github.com/go-acme/lego/v4/cmd/lego@v4.35.2
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
"$scratch/bin/lego" --path "$scratch/lego" \
  --server "https://localhost:$IOTKIT_PEBBLE_PORT/dir" \
  --email test@iotkit.invalid --domains localhost --accept-tos \
  --dns exec --dns.resolvers "127.0.0.1:$IOTKIT_PEBBLE_DNS_PORT" run

cert="$scratch/lego/certificates/localhost.crt"
key="$scratch/lego/certificates/localhost.key"
issuer="$scratch/lego/certificates/localhost.issuer.crt"
test -s "$cert" -a -s "$key" -a -s "$issuer"
chmod 600 "$key"
touch "$scratch/site.env" "$scratch/compose.yaml" "$scratch/password"
cat >"$scratch/cert.env" <<EOF
IOTKIT_CERT_DOMAIN=localhost
IOTKIT_CERT_FILE=$cert
IOTKIT_CERT_KEY_FILE=$key
IOTKIT_CERT_CA_FILE=$issuer
IOTKIT_CERT_SITE_ENV=$scratch/site.env
IOTKIT_CERT_COMPOSE_FILE=$scratch/compose.yaml
IOTKIT_CERT_BROKER_PORT=18883
IOTKIT_CERT_SITE_PASSWORD_FILE=$scratch/password
EOF
chmod 600 "$scratch/cert.env"
"$repo_root/scripts/iotkit-broker-cert" status --config "$scratch/cert.env" \
  | jq -e '.domain == "localhost" and .state == "valid"' >/dev/null

serial_before=$(openssl x509 -in "$cert" -noout -serial)
"$scratch/bin/lego" --path "$scratch/lego" \
  --server "https://localhost:$IOTKIT_PEBBLE_PORT/dir" \
  --email test@iotkit.invalid --domains localhost \
  --dns exec --dns.resolvers "127.0.0.1:$IOTKIT_PEBBLE_DNS_PORT" \
  renew --days 100
serial_after=$(openssl x509 -in "$cert" -noout -serial)
[[ "$serial_before" != "$serial_after" ]] || {
  echo "Pebble renewal did not replace the certificate" >&2
  exit 1
}

echo "Pebble ACME issuance and renewal test passed"
