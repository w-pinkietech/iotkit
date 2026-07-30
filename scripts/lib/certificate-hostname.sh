#!/usr/bin/env bash

certificate_covers_hostname() {
  local certificate=$1 hostname=$2 output
  if ! output=$(LC_ALL=C openssl x509 -in "$certificate" -noout \
    -checkhost "$hostname" 2>&1); then
    return 1
  fi
  [[ "$output" == "Hostname $hostname does match certificate" ]]
}
