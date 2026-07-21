#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

openssl req -x509 -newkey rsa:2048 -nodes -days 90 \
  -subj '/CN=IoTKit Certificate Test CA' \
  -keyout "$scratch/ca.key" -out "$scratch/ca.pem" >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes -subj '/CN=localhost' \
  -keyout "$scratch/server.key" -out "$scratch/server.csr" >/dev/null 2>&1
printf 'subjectAltName=DNS:localhost\nextendedKeyUsage=serverAuth\n' >"$scratch/server.ext"
openssl x509 -req -days 60 -in "$scratch/server.csr" \
  -CA "$scratch/ca.pem" -CAkey "$scratch/ca.key" -CAcreateserial \
  -extfile "$scratch/server.ext" -out "$scratch/server.pem" >/dev/null 2>&1
chmod 600 "$scratch/server.key"

touch "$scratch/edge.env" "$scratch/compose.yaml" "$scratch/password"
cat >"$scratch/cert.env" <<EOF
IOTKIT_CERT_DOMAIN=localhost
IOTKIT_CERT_FILE=$scratch/server.pem
IOTKIT_CERT_KEY_FILE=$scratch/server.key
IOTKIT_CERT_CA_FILE=$scratch/ca.pem
IOTKIT_CERT_EDGE_ENV=$scratch/edge.env
IOTKIT_CERT_COMPOSE_FILE=$scratch/compose.yaml
IOTKIT_CERT_BROKER_PORT=18883
IOTKIT_CERT_EDGE_ARCHIVE_PASSWORD_FILE=$scratch/password
EOF
chmod 600 "$scratch/cert.env"

status=$("$repo_root/scripts/iotkit-broker-cert" status --config "$scratch/cert.env")
jq -e '.domain == "localhost" and .state == "valid" and .days_remaining >= 58' \
  <<<"$status" >/dev/null

cp "$scratch/server.key" "$scratch/wrong.key"
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 \
  -out "$scratch/wrong.key" >/dev/null 2>&1
cp "$scratch/cert.env" "$scratch/wrong.env"
sed -i "s#IOTKIT_CERT_KEY_FILE=.*#IOTKIT_CERT_KEY_FILE=$scratch/wrong.key#" \
  "$scratch/wrong.env"
if "$repo_root/scripts/iotkit-broker-cert" status --config "$scratch/wrong.env" \
  >"$scratch/wrong.stdout" 2>"$scratch/wrong.stderr"; then
  echo "certificate status accepted a mismatched key" >&2
  exit 1
fi

if grep -R -Fq -- "$(cat "$scratch/server.key")" "$scratch"/*.stdout "$scratch"/*.stderr 2>/dev/null; then
  echo "certificate diagnostics leaked private key material" >&2
  exit 1
fi

echo "broker certificate validation/status test passed"
