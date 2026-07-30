#!/bin/sh
set -eu

if [ "${1:-}" = "serve" ] && [ "${IOTKIT_STORAGE_PROFILE:-}" = "postgres" ]; then
  : "${IOTKIT_POSTGRES_CONFIG:?postgres storage requires IOTKIT_POSTGRES_CONFIG}"
  set -- "$@" "--postgres-config=$IOTKIT_POSTGRES_CONFIG"
fi

exec /usr/local/bin/iotkit-edge "$@"
