# IoTKit Site installation and recovery

This is the operator entry point for one factory. IoTKit does not configure the
factory router, DNS, IP address allocation, firewall, or VPN.

## 1. Install

1. Give every Edge a different `edge_node_id` and export its `mqtt-binding`.
2. Prepare a DNS name, a full-chain server certificate, its private key, and a
   root trust bundle that covers the Site host. The key file must be owner-only.
3. Run `scripts/bootstrap-site.sh` for the first Edge. Bootstrap assigns the
   Site source ID before startup and gives `site-output` write access only to
   that Site's IoTKit/YokaKit observation and status namespace. Use repeated
   `--site-publish-topic` only for additional exact legacy application topics.
4. Start `deploy/compose.site.yaml`.
5. Create the first `system_admin` with `iotkit-site account bootstrap` and an
   owner-only password file. Delete that file afterwards.
6. Transfer each generated Edge handoff through a protected channel. This
   Broker enrollment only gives the Edge its MQTT connection and exact-topic
   permissions; it does not authorize Site raw-data custody.
7. Start the Edge and wait for it to appear as **未登録** in **Edge管理**. Confirm
   the expected Edge name or diagnostic identity and data generation, then use
   **Edgeを登録**. Only a settings administrator or system administrator can do
   this.
8. Wait for **登録済み**, then run the commissioning smoke. Configure the sensor
   display and meaning only after the smoke is durably accepted.

Do not copy an old database or credential into a new Site. Registration is a
one-time operation for a fresh publication stream. Values collected before
registration remain outside Site custody and are not replayed after approval.

The generated Caddy endpoint serves HTTPS and proxies only to Site's loopback
HTTP listener. If HTTPS is broken, IoTKit does not expose a plaintext LAN
fallback.

## 2. Daily checks

- **状態**: Site, signal count, missing meaning, and certificate days remaining.
- **Edge管理**: discovery, registration, the last descriptor communication, and
  the exact data generation used for diagnosis. **登録済み** is an authorization
  state; it does not mean the Edge is currently online.
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
- `renew` asks the configured ACME server through `lego`, keeps the configured
  root trust bundle, then uses the same validated install path for the new
  full-chain certificate and key. HTTP-01 uses Caddy's ACME webroot. DNS-01 is
  selected by setting `IOTKIT_CERT_LEGO_CHALLENGE=dns`,
  `IOTKIT_CERT_LEGO_DNS_PROVIDER`, and the provider's credential environment.
- Copy the generated systemd service/timer into `/etc/systemd/system`, add the
  ACME email/server settings to owner-only `broker-cert.env`, then enable the
  timer. The timer checks daily with randomized delay; normal renewal is
  unattended.

The initial DNS, ACME account choice, and provider credential remain installation
work. `IOTKIT_CERT_CA_FILE` must contain the root certificates trusted by Edge
and Site; an intermediate certificate emitted by `lego` is not a replacement
for that trust bundle. The Console shows expiry but does not issue or replace
certificates.

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

## 6. Edge registration recovery

- **未登録** means Site has seen an Edge descriptor but will reject its record
  batches without acknowledging them.
- **登録処理中** is durable. Broker, Site, or Edge restart does not require a
  second registration; the same request is retried until the matching Edge
  result is committed.
- **復旧確認待ち** means the descriptor, stored generation, or activation result
  conflicted. Preserve both databases and investigate identity or restore
  history. Do not delete rows, issue a second Edge identity, or edit the state
  table to make the warning disappear.
- A fresh activation is rejected when the Edge publication stream has ever
  allocated an outbox sequence. IoTKit v1 does not adopt an existing standalone
  outbox, reactivate an Edge, transfer it between Sites, or reuse an identity.
- Registration does not create, rotate, or revoke MQTT credentials and does not
  replace Broker enrollment. Credential recovery remains a separate deployment
  operation.
- Registration freezes a local reading boundary and removes the old prefix in
  bounded background work. This makes the rows unavailable to normal IoTKit
  processing, but it is not a promise of forensic physical erasure from SQLite
  pages, backups, or storage media.
