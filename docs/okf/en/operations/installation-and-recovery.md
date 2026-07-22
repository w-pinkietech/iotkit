---
type: Runbook
title: "IoTKit Edge installation and recovery"
description: "Defines the complete installation, daily checks, certificate, account, backup, restore, migration, and rollback procedures."
language: en
translation_key: operations.installation-and-recovery
status: stable
revision: 2
---

# IoTKit Edge installation and recovery

This is the operator entry point for one IoTKit Edge deployment. IoTKit does not
configure routers, DNS, IP address allocation, firewalls, or VPNs.

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
   that IoTKit Edge's IoTKit/YokaKit observation and status namespace. Use repeated
   `--edge-publish-topic` only for additional exact legacy application topics.
4. Start `embedded` with `deploy/compose.edge.yaml`. For `postgres`, add
   `deploy/compose.edge-postgres.yaml`. IoTKit Edge stops when profile metadata and
   the startup profile disagree.
5. Create the first `system_admin` with `iotkit-edge account bootstrap` and an
   owner-only password file. Delete that file afterwards.
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
- **Equipment / Collection Nodes**: discovery, registration, the last descriptor communication,
  and the exact data generation used for diagnosis. **Registered** is an authorization
  state; it does not mean the Edge Node is currently online.
- **Monitor**: current value and last receipt. A stopped or old signal must be
  investigated at the sensor, adapter, Edge Node, broker, then IoTKit Edge—in that order.
- **Reception history**: filter sensor, Edge Node, and period on one screen, then inspect
  the aggregate graph and recent raw rows. CSV with the same filter exports generic
  observations and is not a business report.
- **Output**: active purpose-bound routes. Pending output is not deleted until
  broker PUBACK.
- **System**: filesystem use, database size, raw/semantic/outbox counts, latest backup,
  and diagnosis by cause. A responsive Console does not prove that an Edge Node or Broker is healthy.
- For `postgres`, SQL cannot report free space in the named volume, so the Console does not
  claim capacity is healthy. Add `docker compose ... exec postgres df -Pk /var/lib/postgresql/data`
  to host monitoring. Warn at 90% use or 2 GiB free, and mark critical at 512 MiB free.
  Compare this with database growth shown by the Console.
- **Audit**: who changed a display name, meaning, output, or account.
- `iotkit-edge-nodectl smoke status`: durable IoTKit Edge acceptance, not merely MQTT PUBACK.
- `scripts/iotkit-broker-cert status --config DEPLOYMENT/broker-cert.env`: exact
  certificate expiry and bundle validation.

## 3. Certificate renewal

`scripts/iotkit-broker-cert` is independent of IoTKit sensor meaning and
YokaKit. It manages the Mosquitto/Caddy certificate bundle on the broker host.

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
2. Read the Console and service logs; do not recreate identity as a first step.
3. Check DNS/route and certificate status.
4. Check Mosquitto authentication and exact-topic ACL.
5. Check Edge Node `accepted-through`; an unaccepted record must remain in Edge Node
   storage.
6. Check IoTKit Edge's output queue. Retry uses the same observation identity.
7. After recovery, confirm raw cursor and pending output converge before
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

## 7. IoTKit Edge encrypted backup

The IoTKit Edge database contains not only sensor history but also account and session hashes,
configuration, audit, and pending outbox rows. Never use a plaintext database-file copy as the
normal operational backup. Supply a passphrase of at least 12 characters from an owner-only file.

A consistent snapshot can be created from a running IoTKit Edge. This is an `embedded` Compose example.

```bash
install_root="$HOME/.local/share/iotkit/edge-01"
backup_root="$HOME/.local/share/iotkit/backups/edge-01"
mkdir -p "$backup_root"
install -m 600 /dev/null "$install_root/secrets/backup-passphrase"
# Write the passphrase with an interactive editor without placing it in shell history.
docker compose --env-file "$install_root/edge.env" -f deploy/compose.edge.yaml \
  run --rm \
  -v "$backup_root:/backup" \
  -v "$install_root/secrets/backup-passphrase:/run/iotkit/backup-passphrase:ro" \
  edge backup create --db /data/edge.db \
  --output "/backup/edge-$(date +%Y%m%d-%H%M%S).iotkit-backup" \
  --passphrase-file /run/iotkit/backup-passphrase
```

For `postgres`, always include the overlay containing PostgreSQL tools and pass the profile
and owner-only connection file.

```bash
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml -f deploy/compose.edge-postgres.yaml \
  run --rm \
  -v "$backup_root:/backup" \
  -v "$install_root/secrets/backup-passphrase:/run/iotkit/backup-passphrase:ro" \
  edge backup create --storage-profile postgres \
  --postgres-config /run/iotkit/postgres.json \
  --storage-metadata /run/iotkit/storage-profile.json \
  --output "/backup/edge-$(date +%Y%m%d-%H%M%S).iotkit-backup" \
  --passphrase-file /run/iotkit/backup-passphrase
```

On success, JSON reports a manifest with format, IoTKit Edge ID, schema, raw count, and database hash.
The container uses Argon2id and XChaCha20-Poly1305 for encryption and tamper detection, is newly
created with mode `0600`, and never overwrites an existing name. Confirm that the Console's last
backup time changed and copy the encrypted container to separate media. MQTT credentials,
certificates, and private keys are not included and must be recovered from deployment configuration.

Compose places the pre-encryption snapshot in dedicated tmpfs, not the backup directory. For host
CLI use, set `TMPDIR` to an owner-only, non-backed-up area erased on restart. The backup CLI does not
supply scheduling. Run it from the OS or existing operations system, copy encrypted containers off
host, alert on failures, and perform regular restore drills. Without this, the RPO after host failure
for records accepted since the last off-host backup is not guaranteed.

## 8. IoTKit Edge restore

Always restore to a new database path. Never overwrite the live database directly.

```bash
docker compose --env-file "$install_root/edge.env" -f deploy/compose.edge.yaml stop edge
docker compose --env-file "$install_root/edge.env" -f deploy/compose.edge.yaml \
  run --rm \
  -v "$backup_root:/backup:ro" \
  -v "$install_root/secrets/backup-passphrase:/run/iotkit/backup-passphrase:ro" \
  edge backup restore --input /backup/SELECTED.iotkit-backup \
  --db /data/edge.restore-candidate.db \
  --passphrase-file /run/iotkit/backup-passphrase
docker compose --env-file "$install_root/edge.env" -f deploy/compose.edge.yaml \
  run --rm edge diagnose --db /data/edge.restore-candidate.db
```

Restore verifies encryption, manifest, database hash, `quick_check`, IoTKit Edge ID, and cursors,
revokes all browser sessions, and records restore history transactionally. After validation, move
the old `edge.db` and its `-wal`/`-shm` into one holding directory, rename the candidate to `edge.db`,
and start IoTKit Edge. Keep the old database until convergence is confirmed.

A `postgres` backup can be restored only to a new database with no existing tables. Stop IoTKit
Edge, create a temporary database such as `iotkit_restore`, and prepare an owner-only temporary
`postgres.json` pointing to it. Change only the database name; use the normal credential, host, and
port, and set mode `0600`.

```bash
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml -f deploy/compose.edge-postgres.yaml stop edge
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml -f deploy/compose.edge-postgres.yaml \
  exec postgres createdb --username iotkit iotkit_restore
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml -f deploy/compose.edge-postgres.yaml \
  run --rm -v "$backup_root:/backup:ro" \
  -v "$install_root/secrets/postgres-restore.json:/run/iotkit/postgres-restore.json:ro" \
  -v "$install_root/secrets/backup-passphrase:/run/iotkit/backup-passphrase:ro" \
  edge backup restore --storage-profile postgres \
  --postgres-config /run/iotkit/postgres-restore.json \
  --storage-metadata /run/iotkit/storage-profile.json \
  --input /backup/SELECTED.iotkit-backup \
  --passphrase-file /run/iotkit/backup-passphrase
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml -f deploy/compose.edge-postgres.yaml \
  run --rm \
  -v "$install_root/secrets/postgres-restore.json:/run/iotkit/postgres-restore.json:ro" \
  edge diagnose --storage-profile postgres \
  --postgres-config /run/iotkit/postgres-restore.json \
  --storage-metadata /run/iotkit/storage-profile.json
```

After checking the manifest, IoTKit Edge ID, schema, cursors, and pending outbox, have two people
confirm the target name and encrypted backup. Keep IoTKit Edge stopped while moving the current
database aside and switching the restored database to the normal name.

```bash
old_database="iotkit_before_restore_$(date +%Y%m%d%H%M%S)"
compose=(docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml -f deploy/compose.edge-postgres.yaml)
"${compose[@]}" exec postgres psql --username iotkit --dbname postgres \
  --set ON_ERROR_STOP=1 --command \
  "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname IN ('iotkit','iotkit_restore') AND pid <> pg_backend_pid();"
"${compose[@]}" exec postgres psql --username iotkit --dbname postgres \
  --set ON_ERROR_STOP=1 --command "ALTER DATABASE iotkit RENAME TO \"$old_database\";"
"${compose[@]}" exec postgres psql --username iotkit --dbname postgres \
  --set ON_ERROR_STOP=1 --command 'ALTER DATABASE iotkit_restore RENAME TO iotkit;'
"${compose[@]}" up --detach edge
"${compose[@]}" run --rm edge diagnose --storage-profile postgres \
  --postgres-config /run/iotkit/postgres.json \
  --storage-metadata /run/iotkit/storage-profile.json
```

Keep the old database until startup, cursor reconvergence, and pending outbox are verified. If the
switch fails, stop the Edge, rename the new `iotkit` database to a separate failed name, restore
`$old_database` to `iotkit`, and restart. The managed database lives in the `postgres-data` Compose
named volume; preserving only the installation directory is insufficient. Never use
`docker compose down --volumes` for a normal stop.

If an Edge Node resumes beyond the cursor in an old backup, IoTKit Edge does not acknowledge it and
places it in `recovery_hold`. `iotkit-edge diagnose` and the Console show the possibly lost cursor
range. Only after deciding that no other backup or original database can recover it, run the next
operation with the IoTKit Edge ID and reason.

```bash
iotkit-edge backup accept-archive-loss --storage-profile embedded --db /path/edge.db \
  --edge-node-id EDGE --ledger-epoch EPOCH \
  --confirm-edge-id EDGE_ID --reason 'original database failed; no other verified backup'
```

For PostgreSQL, pass `--storage-profile postgres --postgres-config FILE --storage-metadata FILE`
to the same command. This operation does not repair missing data. It is the final decision that
audits `archive_lost` and stops permanent retry. Never alter cursors or Edge Node state with SQL.

## 9. Device retirement and hardware replacement

The authoritative device ledger is on the Edge Node; editing a display row in the Console does not
replace a device. Use `iotkit-edge-nodectl device retire` when use ends and
`iotkit-edge-nodectl device replace` for an identity-bearing hardware replacement. Replacement
compares the candidate observation profile with existing series, preserves `system_id`, and changes
only hardware. Forced or unconfirmed execution is not a normal procedure. The Console reflects the
retired state and continuing series after the Edge Node descriptor reaches IoTKit Edge.

## 10. Offline migration from SQLite to PostgreSQL

Stop IoTKit Edge during migration and let the Broker and Edge Nodes retain unacknowledged data.
Never dual-write SQLite and PostgreSQL or automatically fall back after failure. The destination is
an empty database with no IoTKit tables. A running IoTKit Edge holds the same SQLite deployment lock,
so migration cannot begin if shutdown was forgotten. Migration creates a protected consistent
snapshot before copying every table. Store PostgreSQL connection data in a mode-`0600` JSON file;
never pass a DSN or password on the command line.

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

1. Create the encrypted backup above and confirm the latest-backup display in the Console.
2. Record the current Git commit, Compose configuration, and IoTKit Edge image ID. Do not put
   credentials or private keys in Git.
3. Fetch the new version and build the IoTKit Edge image. Keep the Broker running and stop only
   IoTKit Edge. Edge Nodes retain unacknowledged records.
4. Start the new IoTKit Edge. Schema migrations run transactionally at startup.
5. Verify HTTPS login, `/api/v1/system/diagnostics`, cursor reconvergence, pending outbox, history
   graphs, and CSV. After the retention period, remove the old image and pre-update database hold.
6. If startup, migration, or health verification fails, stop IoTKit Edge. Do not open a migrated
   database with the old binary. Return to the old commit/image, restore the pre-update backup into
   a **new candidate database**, and perform the same swap as section 8. Do not recreate Broker or
   Edge Node identities or credentials.

This is a manual update, not automatic update. Returning only the image after database migration is
not rollback; restore the matching pre-update database as well.
