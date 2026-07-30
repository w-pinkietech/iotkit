#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
helper="$repo_root/scripts/lib/certificate-hostname.sh"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

mkdir "$scratch/bin"
cat >"$scratch/bin/openssl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "${IOTKIT_TEST_CHECKHOST_OUTPUT:-}"
exit "${IOTKIT_TEST_CHECKHOST_STATUS:-0}"
EOF
chmod 700 "$scratch/bin/openssl"

# shellcheck disable=SC1090
source "$helper"

hostname=broker.example.invalid
certificate=unused-certificate-path

IOTKIT_TEST_CHECKHOST_OUTPUT="Hostname $hostname does match certificate" \
  IOTKIT_TEST_CHECKHOST_STATUS=0 \
  PATH="$scratch/bin:$PATH" \
  certificate_covers_hostname "$certificate" "$hostname"

if IOTKIT_TEST_CHECKHOST_OUTPUT="Hostname $hostname does NOT match certificate" \
  IOTKIT_TEST_CHECKHOST_STATUS=0 \
  PATH="$scratch/bin:$PATH" \
  certificate_covers_hostname "$certificate" "$hostname"; then
  echo "hostname mismatch output was accepted with a zero exit status" >&2
  exit 1
fi

if IOTKIT_TEST_CHECKHOST_OUTPUT="Hostname $hostname does match certificate" \
  IOTKIT_TEST_CHECKHOST_STATUS=1 \
  PATH="$scratch/bin:$PATH" \
  certificate_covers_hostname "$certificate" "$hostname"; then
  echo "hostname match output was accepted with a failing exit status" >&2
  exit 1
fi

if IOTKIT_TEST_CHECKHOST_OUTPUT="" \
  IOTKIT_TEST_CHECKHOST_STATUS=0 \
  PATH="$scratch/bin:$PATH" \
  certificate_covers_hostname "$certificate" "$hostname"; then
  echo "empty hostname validation output was accepted" >&2
  exit 1
fi

mkdir "$scratch/cn-only"
openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
  -subj "/CN=$hostname" \
  -keyout "$scratch/cn-only/key.pem" \
  -out "$scratch/cn-only/certificate.pem" >/dev/null 2>&1
certificate_covers_hostname "$scratch/cn-only/certificate.pem" "$hostname"
if certificate_covers_hostname \
  "$scratch/cn-only/certificate.pem" "wrong.example.invalid"; then
  echo "CN-only certificate accepted the wrong hostname" >&2
  exit 1
fi

grep -Fq 'source "$repo_root/scripts/lib/certificate-hostname.sh"' \
  "$repo_root/scripts/bootstrap-edge.sh"
grep -Fq 'certificate_covers_hostname "$tls_cert" "$broker_host"' \
  "$repo_root/scripts/bootstrap-edge.sh"
grep -Fq 'source "$script_dir/lib/certificate-hostname.sh"' \
  "$repo_root/scripts/iotkit-broker-cert"
grep -Fq 'certificate_covers_hostname "$input_cert" "$IOTKIT_CERT_DOMAIN"' \
  "$repo_root/scripts/iotkit-broker-cert"

echo "certificate hostname validation test passed"
