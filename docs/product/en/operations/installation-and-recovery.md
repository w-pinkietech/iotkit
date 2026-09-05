---
type: Runbook
title: "IoTKit Edge installation and recovery"
description: "Defines the complete installation, daily checks, certificate, account, backup, restore, migration, and rollback procedures."
language: en
translation_key: operations.installation-and-recovery
status: stable
revision: 30
---

# IoTKit Edge installation and recovery

> **Transitional note (#232 child issue 5).** The central `iotkit-edge`, `scripts/bootstrap-edge.sh`, and `deploy/compose.edge*.yaml` described here were deleted in #251. Encrypted backup and fenced restore were deleted in #253; recovery is now the three-file copy in Backup, Restore, and Device replacement below. The full rewrite of the install and daily-check chapters is the last PR of #250 (5e). Until then the [trial profile](trial-profile.md) is the only runnable installation procedure.

This is the operator entry point for one IoTKit Edge deployment. IoTKit does not
configure routers, DNS, IP address allocation, firewalls, or VPNs.

The Rust IoTKit Edge starts from its own fresh schema. Databases and encrypted
backup artifacts created by the former Go implementation are not accepted,
converted, or restored. Export any required business data before cutover and
perform a clean installation.

Before taking a release candidate to a site, run the host integration gate into a new
report directory without reusing any database or credential. It covers a clean PostgreSQL
installation, Console operations, two simulated Edge Nodes, semantic configuration,
external MQTT, restarts and outages, encrypted backup and restore, certificate rollback,
and the capacity regression smoke for both storage profiles. It does not replace a real
BravePI test, capacity measurements on target hardware, or Windows and Caddy verification.

```bash
scripts/test-edge-host-release-gate.sh /secure/report/iotkit-v1-YYYYMMDD
```

## 1. Install

1. Give every Edge Node a different `edge_node_id` and export its `mqtt-binding`.
2. Prepare a DNS name, a full-chain server certificate, its private key, and a
   root trust bundle that covers the IoTKit Edge host. The key file must be owner-only.
3. Run `scripts/bootstrap-edge.sh` for the first Edge Node. Bootstrap assigns the
   IoTKit Edge source ID before startup and gives `iotkit-edge-output-<edge-id>` write access only to
   that IoTKit Edge's IoTKit/Pinikiet observation and status namespace. Use repeated
   `--edge-publish-topic` only for additional exact legacy application topics.
4. Start `embedded` with `deploy/compose.edge.yaml`. For `postgres`, add
   `deploy/compose.edge-postgres.yaml`. IoTKit Edge stops when profile metadata and
   the startup profile disagree.
5. Create the first `system_admin` with `iotkit-edge account bootstrap` and an
   owner-only password file. Delete that file afterwards.
   After the first login, the status page shows **Setup for first use** to settings
   administrators and system administrators. It derives progress from existing durable state and
   points to only the first incomplete action: register a collection node, name and locate the
   device, confirm sensor type and unit, then define how to use the value. Viewers receive no
   mutation guidance, and the panel disappears when all four items are complete. External output
   is an optional next step because not every deployment needs it.
6. Transfer each generated Edge Node handoff through a protected channel. This
   Broker enrollment only gives the Edge Node its MQTT connection and exact-topic
   permissions; it does not authorize IoTKit Edge raw-data custody.
7. Start the Edge Node and wait for it to appear as **Unregistered** under
   **Equipment / Collection Nodes**. Confirm
   the expected Edge Node name or diagnostic identity and data generation, then use
   **Register collection node**. Only a settings administrator or system administrator can do
   this.
8. Wait for **Registered**, then run the commissioning smoke. Configure the sensor
   display and meaning only after the smoke is durably accepted.

Do not copy an old database or credential into a new IoTKit Edge. Registration is a
one-time operation for a fresh publication stream. Values collected before
registration remain outside IoTKit Edge custody and are not replayed after approval.

The generated Caddy endpoint serves HTTPS and proxies only to IoTKit Edge's loopback
HTTP listener. If HTTPS is broken, IoTKit does not expose a plaintext LAN
fallback.

## 2. Daily checks

- **Status**: IoTKit Edge, signal count, missing meaning, and certificate days remaining.
- **Status / causal diagnosis**: read the ordered Sensor input → Input Adapter → Edge Node
  collector → internal Broker path → raw custody → semantic projection → external output evidence.
  Start with its single earliest critical or warning action. Each stage shows its last successful
  fact when known (otherwise **not yet confirmed**), bounded affected scope, cautious cause, and
  next check. It is recalculated from current durable and process facts: a later matching success
  clears an active state automatically, with no manual incident dismissal. An `unknown` state means
  there is not enough current evidence, not that the stage is healthy. An Edge Node heartbeat is fresh for 90
  seconds, warning from 90 through 299 seconds, and critical at or after 300 seconds. A retained-only
  heartbeat is historical detail, and an old raw value is only an advisory when upstream evidence
  is healthy: neither proves a sensor is stopped.
- **Equipment / Collection Nodes**: discovery, registration, the last descriptor communication,
  and the exact data generation used for diagnosis. **Registered** is an authorization
  state; it does not mean the Edge Node is currently online.
- **Live**: show one card for every saved active `cumulative_counter` measurement rule. Each card uses the latest persisted
  processed value after calibration and rule evaluation, with its receipt time, independently of
  its chart. The chart contains only results received after the operator opened the page.
  Multiple active cumulative rules for one signal become
  separate cards; numeric, boolean, and alarm rule cards, plus ruleless signals, are omitted, and one dashboard-level setup message
  appears only when no active cumulative rules exist. The cumulative chart grows from page open across the whole page
  session, uses each bucket's exact terminal value, and is shown as a staircase. The browser keeps the result bounded to
  at most 1,000 buckets, increasing the bucket width after the session exceeds that range rather
  than rolling the time window. Until a post-open processed value arrives,
  the chart stays empty and says that it is waiting, even when a prior processed current value is
  available. Each card links to sensor detail. The browser
  refreshes at most 12 cards in the visible region every five seconds, and only while the document
  is visible. After a successful fetch, the elapsed last-receipt time and chart window stay anchored
  to the IoTKit Edge time at page open and advance by the browser's monotonic elapsed time, even if a
  later fetch temporarily fails. It identifies rules that
  have never produced data and marks five minutes without a new result as **Check**, not as proof
  of a stopped device. Use **Reception history** for raw and past data. Investigate Check at the
  sensor, adapter, Edge Node, broker, IoTKit Edge, then semantic projection—in that order.
  Live and sensor-detail **real-signal** chart axes use semantic observed/event time when it is
  available; the latest receipt/current freshness remains the IoTKit Edge raw receipt time.
  The real-signal preview uses the same recent 60-second, one-second chart buckets while
  evaluating its bounded input history so boolean and cumulative results retain their state.
  For cumulative rules, the result card shows the persisted current total. The real-signal preview
  labels the hypothetical last-60-second delta. Numeric, boolean, alarm, and draft upper charts
  remain recent-60-second charts, while a selected persisted cumulative rule gives the upper
  received/settings-result chart and lower persisted cumulative staircase the same page-open
  display-start to current-time axis. The upper chart retains overlapping recent responses in the browser and
  compacts them to at most 1,000 representative points across the whole display period. An existing
  rule also shows a separate persisted cumulative staircase after that selected saved rule becomes
  active. It records saved-current changes from display start, extends an unchanged value to the
  monotonic current page time, and keeps at most 1,000 displayed points. It does not discard session
  changes merely because they leave a rolling 60-second history request; a draft says accumulation
  starts after save. Each stair samples the persisted current state in persistence order, not an
  observed-time bucket or bucket average. A successful session with no captured saved point is
  shown as no saved change since display started, while a failed history request is shown as
  unavailable.
- **Reception history**: filter sensor, Edge Node, and period on one screen, then inspect
  the bounded graph and recent raw rows that match the selected sensor. The raw graph's horizontal
  axis shows IoTKit Edge receipt timestamps in the display time zone, and its vertical axis shows
  the value range and sensor unit. CSV with the same filter exports generic observations and is not
  a business report.
- **Output**: active purpose-bound routes. Pending output is not deleted until
  broker PUBACK.
- **System**: filesystem use, database size, raw/semantic/pending-projection/outbox counts, latest backup,
  and diagnosis by cause. A responsive Console does not prove that an Edge Node or Broker is healthy.
- For `postgres`, SQL cannot report free space in the named volume, so the Console does not
  claim capacity is healthy. Add `docker compose ... exec postgres df -Pk /var/lib/postgresql/data`
  to host monitoring. Warn at 90% use or 2 GiB free, and mark critical at 512 MiB free.
  Compare this with database growth shown by the Console.
- **Audit**: who changed a display name, meaning, output, or account.
- `iotkit-edge-nodectl smoke status`: durable IoTKit Edge acceptance, not merely MQTT PUBACK.
- `scripts/iotkit-broker-cert status --config DEPLOYMENT/broker-cert.env`: exact
  certificate expiry and bundle validation.

Absolute Console timestamps use the display time zone selected when IoTKit Edge starts. For a Compose deployment, set an IANA time zone such as `IOTKIT_DISPLAY_TIME_ZONE=Asia/Tokyo` in the owner-only `edge.env`; a direct launch may instead pass `iotkit-edge serve --display-time-zone Asia/Tokyo ...`. The default is `UTC`, and an invalid value is a startup error. Raw storage, API, and CSV timestamps remain Unix milliseconds; this setting changes display only.

## 3. Certificate renewal

`scripts/iotkit-broker-cert` is independent of IoTKit sensor meaning and
Pinikiet. It manages the Mosquitto/Caddy certificate bundle on the broker host.

- `install` validates the chain, hostname, expiry, and key; switches the three
  files; reloads Mosquitto; restarts IoTKit Edge so trust changes are read; reloads
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
work. `IOTKIT_CERT_CA_FILE` must contain the root certificates trusted by Edge Node
and IoTKit Edge; an intermediate certificate emitted by `lego` is not a replacement
for that trust bundle. The Console shows expiry but does not issue or replace
certificates.

## 4. Account recovery

Only the IoTKit Edge host can recover a system administrator. Example for an `embedded`
Compose installation:

```bash
docker compose --env-file "$install_root/edge.env" -f deploy/compose.edge.yaml \
  run --rm -v /owner-only/new-password:/run/iotkit/new-password:ro \
  edge account recover --storage-profile embedded --db /data/edge.db \
  --storage-metadata /run/iotkit/storage-profile.json --login-id admin \
  --password-file /run/iotkit/new-password
```

For `postgres`, explicitly provide the overlay and PostgreSQL connection file.

```bash
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml -f deploy/compose.edge-postgres.yaml \
  run --rm -v /owner-only/new-password:/run/iotkit/new-password:ro \
  edge account recover --storage-profile postgres \
  --postgres-config /run/iotkit/postgres.json \
  --storage-metadata /run/iotkit/storage-profile.json --login-id admin \
  --password-file /run/iotkit/new-password
```

Recovery revokes existing sessions. Passwords, MQTT credentials, private keys,
and session tokens must never be placed in arguments, logs, audit summaries, or
Git.

## 5. Failure order

1. Preserve both Edge Node and IoTKit Edge databases.
2. If the Console is available, read its causal status section and start with the first actionable
   critical or warning cause. Do not turn `unknown`, registration, a descriptor, or historic raw
   data into an online assertion. If a fatal supervised IoTKit Edge task exited, the Console is
   intentionally unavailable; use the host service manager and service logs instead.
3. Check DNS/route and certificate status.
4. Check Mosquitto authentication and exact-topic ACL, including the Edge Node write and IoTKit
   Edge read permissions for `iotkit/v1/edge-nodes/{edge_node_id}/status`.
5. Check Edge Node `accepted-through`; an unaccepted record must remain in Edge Node
   storage.
6. Check IoTKit Edge's semantic-projection queue and output queue. Retry uses the same observation identity.
7. After recovery, confirm raw cursor, pending semantic projection, and pending output converge before
   deleting any retained data.

## 6. Edge Node registration recovery

- **Unregistered** means IoTKit Edge has seen an Edge Node descriptor but will reject its record
  batches without acknowledging them.
- **Registration in progress** is durable. Broker, IoTKit Edge, or Edge Node restart does not require a
  second registration; the same request is retried until the matching Edge Node
  result is committed.
- **Recovery review required** means the descriptor, stored generation, or activation result
  conflicted. Preserve both databases and investigate identity or restore
  history. Do not delete rows, issue a second Edge Node identity, or edit the state
  table to make the warning disappear.
- A fresh activation is rejected when the Edge Node publication stream has ever
  allocated an outbox sequence. IoTKit v1 does not adopt an existing standalone
  outbox, reactivate an Edge Node, transfer it between IoTKit Edge deployments, or reuse an identity.
- Registration does not create, rotate, or revoke MQTT credentials and does not
  replace Broker enrollment. Credential recovery remains a separate deployment
  operation.
- Registration freezes a local reading boundary and removes the old prefix in
  bounded background work. This makes the rows unavailable to normal IoTKit
  processing, but it is not a promise of forensic physical erasure from SQLite
  pages, backups, or storage media.

## 7. Backup

The device state is these three files.

- TOML (`edge-node-id`, `[output.mqtt]`, `[status]`, Input Adapter instances)
- The SQLite file (pipeline definitions, evaluator state, accumulated value or state, series / sequence, and the pre-PUBACK outbox)
- `pipelines.toml` (the pipeline-definition backup written from the database; it is not read at startup)

Copy these three files from a stopped device and keep them. Encrypted backup, the snapshot CLI, and fenced restore are not provided.

## 8. Restore

Copy the three files onto the stopped device or onto replacement hardware, keeping the same relative layout. Do not recreate the SQLite file; use the copy. To reload pipeline definitions from a file, use `nodectl pipeline import`. After import every pipeline starts a new series.

NTP synchronization is required. A consumer can detect clock skew from the difference between a heartbeat `timestamp` and the receive time.

## 9. Device replacement

To replace the hardware, stop the old device, copy the three files onto the new device, and start it. Continuity of accumulated values and series holds only for what the copied SQLite contains. If the copy is unavailable, new series begin. Encrypted backup and reactivation through recovery authority are not used.

## 10. Offline migration from SQLite to PostgreSQL

Stop IoTKit Edge during migration and let the Broker and Edge Nodes retain unacknowledged data.
Never dual-write SQLite and PostgreSQL or automatically fall back after failure. The destination is
an empty database with no IoTKit tables. A running IoTKit Edge holds the same SQLite deployment lock,
so migration cannot begin if shutdown was forgotten. Migration creates a protected consistent
snapshot before copying every table. Store PostgreSQL connection data in a mode-`0600` JSON file;
never pass a DSN or password on the command line.

Offline profile migration accepts a current schema-v12 SQLite source only. For a v9, v10, or v11
source, first start the current IoTKit Edge against that SQLite database and wait for its transactional
v12 upgrade to complete, then stop it again before migration. That upgrade preserves the derived raw
series key, adds the latest Edge Node operational-status row, and builds current-epoch raw-receipt and
active rule/route diagnostic indexes. It neither backfills rows, copies retained history, nor creates a
heartbeat history, but the index builds read retained raw, observation, and outbox history; leave time
and temporary database/WAL capacity in proportion to that history. The offline copy preserves and
verifies the stored raw-series value, the v11 raw-preview index, the v12 latest-status table, and the
v12 diagnostic indexes; it does not derive different target values.

```json
{"dsn":"postgres://iotkit:REDACTED@postgres:5432/iotkit?sslmode=require"}
```

```bash
install -m 600 /dev/null /run/iotkit/postgres.json
# Write the JSON with an interactive editor without placing secrets in shell history.
iotkit-edge storage migrate \
  --from-sqlite /data/edge.db \
  --to-postgres-config /run/iotkit/postgres.json \
  --report /data/sqlite-to-postgres-report.json
```

A successful report includes profile, IoTKit Edge ID, schema version, every table count, cursor
vector, a content digest of every row, and `completed: true`. It is newly created with mode `0600`.
Keep the original SQLite database, start with the PostgreSQL profile, and verify Console history,
pending outbox, and Edge Node cursor reconvergence. On mismatch or partial failure, do not use the
PostgreSQL side; recreate an empty database and run migration again.

## 11. Manual IoTKit Edge update and rollback

1. Copy the TOML, SQLite file, and `pipelines.toml` from the stopped device and keep them.
2. Record the current Git commit, Compose configuration, and IoTKit Edge image ID. Do not put
   credentials or private keys in Git.
3. Fetch the new version and build the IoTKit Edge image. Keep the Broker running and stop only
   IoTKit Edge. Edge Nodes retain unacknowledged records.
4. For schema v12, retain the three files copied before the update and leave enough free space and time for the
   derived raw-series-key backfill, its raw-preview index, the latest-status table, current-epoch
   raw-receipt and active rule/route diagnostic indexes, and SQLite WAL growth. Startup backfills valid
   canonical measurement envelopes where needed; the status table has no heartbeat-history backfill and
   the diagnostic indexes add no copied history rows. Their builds read retained raw, observation, and
   outbox history. The migration either commits schema v12 completely or rolls back.
5. Start the new IoTKit Edge. Schema migrations run transactionally at startup.
6. Verify HTTPS login, `/api/v1/system/diagnostics`, cursor reconvergence, pending semantic projection,
   pending outbox, history graphs, and CSV. Let the queue drain before treating restart recovery as
   complete. After the retention period, remove the old image and pre-update database hold.
7. If startup, migration, or health verification fails, stop IoTKit Edge. Do not open a migrated
   database with the old binary. Return to the old commit/image and restore the three files copied
   before the update using the same procedure as section 8. Do not recreate Broker or
   Edge Node identities or credentials.

This is a manual update, not automatic update. Returning only the image after database migration is
not rollback; restore the matching pre-update database as well.
