# IoTKit Site installation and recovery

This is the operator entry point for one factory. IoTKit does not configure the
factory router, DNS, IP address allocation, firewall, or VPN.

## 1. Install

1. Give every Edge a different `edge_node_id` and export its `mqtt-binding`.
2. Prepare a DNS name and a certificate/key/issuer bundle that covers the Site
   host. The key file must be owner-only.
3. Run `scripts/bootstrap-site.sh` for the first Edge. Add exact YokaKit
   observation and status topics with repeated `--site-publish-topic`.
4. Start `deploy/compose.site.yaml`.
5. Create the first `system_admin` with `iotkit-site account bootstrap` and an
   owner-only password file. Delete that file afterwards.
6. Transfer each generated Edge handoff through a protected channel and run the
   commissioning smoke. Do not copy an old database or credential into a new Site.

The generated Caddy endpoint serves HTTPS and proxies only to Site's loopback
HTTP listener. If HTTPS is broken, IoTKit does not expose a plaintext LAN
fallback.

## 2. Daily checks

- **状態**: Site, signal count, missing meaning, and certificate days remaining.
- **モニター**: current value and last receipt. A stopped or old signal must be
  investigated at the sensor, adapter, Edge, broker, then Site—in that order.
- **出力**: active purpose-bound routes. Pending output is not deleted until
  broker PUBACK.
- **監査**: who changed a display name, meaning, output, or account.
- `iotkit-edgectl smoke status`: durable Site acceptance, not merely MQTT PUBACK.
- `scripts/iotkit-broker-cert status --config SITE/broker-cert.env`: exact
  certificate expiry and bundle validation.

## 3. Certificate renewal

`scripts/iotkit-broker-cert` is independent of IoTKit sensor meaning and
YokaKit. It manages the Mosquitto/Caddy certificate bundle on the broker host.

- `install` validates the chain, hostname, expiry, and key; switches the three
  files; reloads Mosquitto; restarts Site so trust changes are read; reloads
  Caddy; then probes new MQTT TLS and HTTPS connections.
- A failed probe restores the previous three files and reloads the services.
- `renew` asks the configured ACME server through `lego`, then uses the same
  validated install path. HTTP-01 uses Caddy's ACME webroot. DNS-01 is selected
  by setting `IOTKIT_CERT_LEGO_CHALLENGE=dns`,
  `IOTKIT_CERT_LEGO_DNS_PROVIDER`, and the provider's credential environment.
- Copy the generated systemd service/timer into `/etc/systemd/system`, add the
  ACME email/server settings to owner-only `broker-cert.env`, then enable the
  timer. The timer checks daily with randomized delay; normal renewal is
  unattended.

The initial DNS, ACME account choice, and provider credential remain installation
work. The Console shows expiry but does not issue or replace certificates.

## 4. Account recovery

Only the Site host can recover a system administrator:

```bash
iotkit-site account recover --db /path/site.db --login-id admin \
  --password-file /owner-only/new-password
```

Recovery revokes existing sessions. Passwords, MQTT credentials, private keys,
and session tokens must never be placed in arguments, logs, audit summaries, or
Git.

## 5. Failure order

1. Preserve both Edge and Site databases.
2. Read the Console and service logs; do not recreate identity as a first step.
3. Check DNS/route and certificate status.
4. Check Mosquitto authentication and exact-topic ACL.
5. Check Edge `accepted-through`; an unaccepted record must remain in Edge
   storage.
6. Check Site's output queue. Retry uses the same observation identity.
7. After recovery, confirm raw cursor and pending output converge before
   deleting any retained data.
