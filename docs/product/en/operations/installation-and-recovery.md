---
type: Runbook
title: "IoTKit Edge installation and recovery"
description: "Defines the complete installation, daily checks, certificate, account, backup, restore, migration, and rollback procedures."
language: en
translation_key: operations.installation-and-recovery
status: stable
revision: 8
---

# IoTKit Edge installation and recovery

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
- **Equipment / Collection Nodes**: discovery, registration, the last descriptor communication,
  and the exact data generation used for diagnosis. **Registered** is an authorization
  state; it does not mean the Edge Node is currently online.
- **Live**: show the current value, last receipt, and latest 15-minute trend for every
  registered signal. Numeric signals use a line and contacts use an ON/OFF step line;
  each card links to sensor detail. The browser refreshes at most 12 cards in the visible
  region every five seconds, and only while the document is visible. It identifies signals
  that have never received data and marks five minutes without a new value as **Check**,
  not as proof of a stopped device. Investigate Check at the sensor, adapter, Edge Node,
  broker, then IoTKit Edge—in that order.
- **Reception history**: filter sensor, Edge Node, and period on one screen, then inspect
  the bounded graph and recent raw rows that match the selected sensor. The graph's horizontal
  axis shows the actual reception timestamps in the display time zone, and its vertical axis shows
  the value range and sensor unit. CSV with the same filter exports generic observations and is not
  a business report.
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

## 7. Encrypted backup

### 7.1 Optional Edge Node encrypted backup

The Edge Node backup is a separate, local-root operation. It is not configured
and its timer is not enabled by default. It creates a custody-complete,
sanitized SQLite backup using the [Edge Node recovery contract](../contracts/edge-node-recovery-v1.md).
The snapshot sanitizer removes the deployment credential token from
`target_registry`; MQTT/TLS private material is outside this database and is
not placed in the artifact. Account, session, and device credential hashes may
remain as protected database state, so every artifact is encrypted and treated
as a secret. There is no legacy plaintext snapshot fallback.

Use an owner-only configuration and passphrase file. Do not put a passphrase in
an argument, shell history, log, or systemd unit. The destination must first
pass the capability probe: it must be an owner-only writable directory on a
stable, identified mount, with enough capacity and no-replace/read-back/parent-
sync behavior. A filesystem label or a mutable device name is not sufficient,
and the destination must be on a different filesystem from the live database.

`/run` is the staging tmpfs parent. `configure` opens that existing parent
without following its final path component and requires an euid-owned, non-
group/other-writable tmpfs directory (the usual `/run` mode `0755` is valid;
the world-writable `/dev/shm` root is not). It records the exact
`/run/iotkit-edge-node-backup` leaf and never creates a missing parent tree. At
`create` time the held parent descriptor is used to create only an absent exact
leaf with mode `0700`; an existing leaf must be the owner-only directory on the
same tmpfs and is checked for its link count and type. The service's
`RuntimeDirectory=iotkit-edge-node-backup` therefore supplies the accepted leaf;
do not pre-create or broaden an arbitrary `/run` tree, and do not put the
destination or a persistent database under `TMPDIR`.

```bash
sudo install -d -m 0700 /etc/iotkit
if ! sudo test -e /etc/iotkit/edge-node-backup-passphrase; then
  sudo install -m 600 /dev/null /etc/iotkit/edge-node-backup-passphrase
fi
sudo chmod 600 /etc/iotkit/edge-node-backup-passphrase
# Write the passphrase interactively without putting it in shell history.
sudo install -D -m 0644 deploy/systemd/iotkit-edge-node-backup.service \
  /etc/systemd/system/iotkit-edge-node-backup.service
sudo install -D -m 0644 deploy/systemd/iotkit-edge-node-backup.timer \
  /etc/systemd/system/iotkit-edge-node-backup.timer
sudo install -d -m 0755 /etc/systemd/system/iotkit-edge-node-backup.service.d
sudo iotkit-edge-nodectl backup configure \
  --config /etc/iotkit/edge-node-backup.json \
  --db /var/lib/iotkit/edge-node/edge.db \
  --destination /mnt/iotkit-backups/edge-node-01 \
  --staging-directory /run/iotkit-edge-node-backup \
  --passphrase-file /etc/iotkit/edge-node-backup-passphrase \
  --freshness-seconds 86400 --retention-count 7 \
  --systemd-drop-in \
  /etc/systemd/system/iotkit-edge-node-backup.service.d/destination.conf
sudo systemctl daemon-reload
```

The configure command publishes the owner-only configuration and the exact
drop-in as one guarded pair. Review the generated mount point; the drop-in is
only:

```ini
[Unit]
RequiresMountsFor=/absolute/captured/mount/point
```

The timer remains disabled until an operator explicitly opts in:

```bash
sudo systemctl enable --now iotkit-edge-node-backup.timer
sudo systemctl status iotkit-edge-node-backup.timer
```

Before enabling it, use the manual non-secret surfaces and check the artifact
off host:

```bash
sudo iotkit-edge-nodectl backup create --config /etc/iotkit/edge-node-backup.json
sudo iotkit-edge-nodectl backup inspect --input /mnt/iotkit-backups/edge-node-01/SELECTED.iotkit-node-backup \
  --passphrase-file /etc/iotkit/edge-node-backup-passphrase
sudo iotkit-edge-nodectl backup status --config /etc/iotkit/edge-node-backup.json
```

Escrow the passphrase through the deployment's approved encrypted, owner-only
procedure and retain an off-host copy of each encrypted artifact. A lost
passphrase makes the artifact intentionally unrecoverable. A create failure is
not a durable backup and does not authorize deleting or replacing the live DB.

### 7.2 IoTKit Edge encrypted backup

The IoTKit Edge database contains not only sensor history but also account and session hashes,
configuration, audit, and pending outbox rows. Never use a plaintext database-file copy as the
normal operational backup. Supply a passphrase of at least 12 characters from an owner-only file.

A consistent snapshot can be created from a running IoTKit Edge. This is an `embedded` Compose example.

```bash
install_root="$HOME/.local/share/iotkit/edge-01"
backup_root="$HOME/.local/share/iotkit/backups/edge-01"
mkdir -p "$backup_root"
if [ ! -e "$install_root/secrets/backup-passphrase" ]; then
  install -m 600 /dev/null "$install_root/secrets/backup-passphrase"
fi
chmod 600 "$install_root/secrets/backup-passphrase"
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

## 8. Restore

### 8.1 Return an Edge Node encrypted-backup candidate to production

Stop and physically isolate the old host, then fence its Broker credential first.
`fence-edge-node.sh` advances the bundled Mosquitto password generation and
restarts the Broker to sever existing sessions. It emits the new password once,
alongside a non-secret receipt, in a new owner-only directory.

```bash
set -euo pipefail
umask 077
CASE="edge-node-${EDGE_NODE_ID}-$(date +%Y%m%d%H%M%S)"
EDGE_CONTROL_SOCKET="/data/recovery-control.sock"
INSPECT_STAGING="/run/iotkit-edge-node-recovery-inspect-$CASE"
: "${IOTKIT_REPO_ROOT:?set the checkout containing deploy/compose.edge.yaml}"
: "${NODE_RUNTIME_USER:?set the service account used by the Edge Node}"
: "${NODE_RUNTIME_GROUP:?set the service group used by the Edge Node}"
getent passwd "$NODE_RUNTIME_USER" >/dev/null
getent group "$NODE_RUNTIME_GROUP" >/dev/null
NODE_RUNTIME_UID=$(id -u "$NODE_RUNTIME_USER")
if [[ "$NODE_RUNTIME_UID" != "$(id -u)" ]]; then
  echo "run this owner-bound recovery block as NODE_RUNTIME_USER" >&2
  exit 1
fi
install_root=$(realpath "$install_root")
if [[ "$(stat -c %u "$install_root")" != "$(id -u)" ]]; then
  echo "run the deployment-file steps as the owner of install_root" >&2
  exit 1
fi
install -d -m 700 "$install_root/recovery"
LIVE_PARENT=$(realpath "$(dirname -- "$LIVE_DB")")
for owner_bound_path in "$SELECTED" "$PASSPHRASE" "$LIVE_PARENT"; do
  if [[ "$(stat -c %u "$owner_bound_path")" != "$NODE_RUNTIME_UID" ]] ||
    (( (8#$(stat -c %a "$owner_bound_path") & 8#077) != 0 )); then
    echo "backup evidence and live DB parent must be owner-only NODE_RUNTIME_USER paths" >&2
    exit 1
  fi
done
sudo install -d -o "$(id -u)" -g "$(id -g)" -m 700 "$INSPECT_STAGING"
edge_cli() {
  sudo docker compose --env-file "$install_root/edge.env" \
    -f "$IOTKIT_REPO_ROOT/deploy/compose.edge.yaml" \
    exec --user 0 -T edge iotkit-edge "$@"
}
"$IOTKIT_REPO_ROOT/scripts/upgrade-edge-node-recovery-acl.sh" \
  --edge-dir "$install_root" --edge-node-id "$EDGE_NODE_ID"
sudo docker compose --env-file "$install_root/edge.env" \
  -f "$IOTKIT_REPO_ROOT/deploy/compose.edge.yaml" \
  up --detach --no-deps --force-recreate edge
for _ in $(seq 1 30); do
  sudo test -S "$install_root/data/edge/recovery-control.sock" && break
  sleep 1
done
sudo test -S "$install_root/data/edge/recovery-control.sock"
"$IOTKIT_REPO_ROOT/scripts/fence-edge-node.sh" \
  --edge-dir "$install_root" --edge-node-id "$EDGE_NODE_ID" \
  --output-directory "$install_root/recovery/$CASE"
iotkit-edge-nodectl backup inspect --input "$SELECTED" \
  --passphrase-file "$PASSPHRASE" \
  --staging-directory "$INSPECT_STAGING" \
  | tee "$install_root/recovery/$CASE/backup-inspection.json" >/dev/null
rmdir "$INSPECT_STAGING"
chmod 600 "$install_root/recovery/$CASE/backup-inspection.json"
edge_cli recovery prepare --control-socket "$EDGE_CONTROL_SOCKET" \
  --backup-inspection "/recovery/$CASE/backup-inspection.json" \
  --broker-fence-receipt "/recovery/$CASE/broker-fence-receipt.json" \
  --handoff-output "/recovery/$CASE/recovery-handoff.json"
RECOVERY_ID=$(sudo jq -r .recovery_id \
  "$install_root/recovery/$CASE/recovery-handoff.json")
```

`prepare` checks the active old epoch and durable accepted-through on IoTKit
Edge against the backup boundary and Broker generation, then stores the case
and new epoch. Do not edit the handoff. Restore to an absent candidate path.
Restore receipt v2 includes the candidate instance and the Node-side
`device_auth_generation`. The candidate database must be inside a new
dedicated parent; do not repurpose a shared data directory. The restore
operation is owner-bound to the live database and candidate parent. Root
provisions the dedicated tmpfs leaf, candidate parent, handoff, and passphrase,
then transfers them to the actual Node service account **before** restore.
Restore and activation both run as that account.

```bash
CANDIDATE_PARENT=$(dirname -- "$CANDIDATE_DB")
RESTORE_STAGING="/run/iotkit-edge-node-recovery-restore-$CASE"
if sudo test -e "$CANDIDATE_PARENT"; then
  echo "candidate parent must be a new dedicated directory" >&2
  exit 1
fi
sudo install -d -o "$NODE_RUNTIME_USER" -g "$NODE_RUNTIME_GROUP" -m 700 \
  "$CANDIDATE_PARENT" "$RESTORE_STAGING"
sudo install -o "$NODE_RUNTIME_USER" -g "$NODE_RUNTIME_GROUP" -m 600 \
  "$install_root/recovery/$CASE/recovery-handoff.json" \
  "$RESTORE_STAGING/recovery-handoff.json"
sudo install -o "$NODE_RUNTIME_USER" -g "$NODE_RUNTIME_GROUP" -m 600 \
  "$PASSPHRASE" "$RESTORE_STAGING/passphrase"
sudo -u "$NODE_RUNTIME_USER" -g "$NODE_RUNTIME_GROUP" \
  iotkit-edge-nodectl backup restore --input "$SELECTED" \
  --candidate-db "$CANDIDATE_DB" --live-db "$LIVE_DB" \
  --staging-directory "$RESTORE_STAGING" \
  --passphrase-file "$RESTORE_STAGING/passphrase" \
  --recovery-handoff "$RESTORE_STAGING/recovery-handoff.json" \
  | sudo tee "$install_root/recovery/$CASE/restore-receipt.json" >/dev/null
sudo chmod 600 "$install_root/recovery/$CASE/restore-receipt.json"
sudo rm -f "$RESTORE_STAGING/passphrase" "$RESTORE_STAGING/recovery-handoff.json"
sudo rmdir "$RESTORE_STAGING"
edge_cli recovery authorize --control-socket "$EDGE_CONTROL_SOCKET" \
  --restore-receipt "/recovery/$CASE/restore-receipt.json"
sudo -u "$NODE_RUNTIME_USER" -g "$NODE_RUNTIME_GROUP" \
  sh -c 'test -r "$1" && test -w "$2"; probe="$2/.iotkit-write-probe.$$"; (umask 077; : >"$probe") && rm -f "$probe"' \
  sh "$CANDIDATE_DB" "$CANDIDATE_PARENT"
sudo install -o "$NODE_RUNTIME_USER" -g "$NODE_RUNTIME_GROUP" -m 600 \
  "$install_root/recovery/$CASE/mqtt-password" \
  /etc/iotkit/mqtt-password
sudo -u "$NODE_RUNTIME_USER" -g "$NODE_RUNTIME_GROUP" \
  iotkit-edge-nodectl backup activate --candidate-db "$CANDIDATE_DB" \
  --broker-host "$BROKER_HOST" --broker-port "$BROKER_PORT" \
  --password-file /etc/iotkit/mqtt-password --ca-file /etc/iotkit/broker-ca.pem
```

The candidate cannot collect, publish, or bind HTTP ingest before receiving
the matching request. One SQLite transaction converges rows accepted by the
Edge, renumbers remaining publications into the new epoch, and puts
`epoch_start` at sequence 1. The normal runtime remains fenced until IoTKit
Edge durably commits the matching result and the Node stores its completion.
Process, Broker, Edge, and candidate restarts reuse the same
request/result/completion/completion-ACK exchange. Edge retains and retries the
completion until the Node durably stores it and publishes the matching ACK. A
different candidate, artifact, epoch, generation, or cursor enters
`recovery_hold`.

`backup activate` reporting `recovered` proves only that the candidate stored
completion; it is not yet production-ready. Poll the running Edge through its
owner-only control socket. If `completion_acknowledged` remains false or the
call timed out, rerun the same activate command on the same candidate and
report again. Do not start the normal runtime until the durable Edge report is
`state=completed` and `completion_acknowledged=true`.

```bash
while :; do
  edge_cli recovery report --control-socket "$EDGE_CONTROL_SOCKET" \
    --recovery-id "$RECOVERY_ID" \
    | sudo tee "$install_root/recovery/$CASE/final-report.json" >/dev/null
  sudo jq -e \
    '.state == "completed" and .completion_acknowledged == true' \
    "$install_root/recovery/$CASE/final-report.json" >/dev/null && break
  sudo -u "$NODE_RUNTIME_USER" -g "$NODE_RUNTIME_GROUP" \
    iotkit-edge-nodectl backup activate --candidate-db "$CANDIDATE_DB" \
    --broker-host "$BROKER_HOST" --broker-port "$BROKER_PORT" \
    --password-file /etc/iotkit/mqtt-password \
    --ca-file /etc/iotkit/broker-ca.pem
  sleep 5
done

# Restore deliberately removed the old admin credential and every operator/session token.
# Re-establish local ownership interactively; never pass the new passphrase through argv,
# an environment variable, a log, or the incident report.
sudo -u "$NODE_RUNTIME_USER" -g "$NODE_RUNTIME_GROUP" \
  iotkit-edge-nodectl --db "$CANDIDATE_DB" passphrase reset

# Move the existing backup policy to the recovered DB before the timer can run again.
NODE_BACKUP_CONFIG=${NODE_BACKUP_CONFIG:-/etc/iotkit/edge-node-backup.json}
NODE_BACKUP_DROP_IN=${NODE_BACKUP_DROP_IN:-/etc/systemd/system/iotkit-edge-node-backup.service.d/destination.conf}
BACKUP_DESTINATION=$(sudo jq -er .destination "$NODE_BACKUP_CONFIG")
BACKUP_STAGING=$(sudo jq -er .staging_directory "$NODE_BACKUP_CONFIG")
BACKUP_PASSPHRASE=$(sudo jq -er .passphrase_file "$NODE_BACKUP_CONFIG")
BACKUP_FRESHNESS=$(sudo jq -er .freshness_seconds "$NODE_BACKUP_CONFIG")
BACKUP_RETENTION=$(sudo jq -er .retention_count "$NODE_BACKUP_CONFIG")
sudo iotkit-edge-nodectl backup configure \
  --config "$NODE_BACKUP_CONFIG" --db "$CANDIDATE_DB" \
  --destination "$BACKUP_DESTINATION" \
  --staging-directory "$BACKUP_STAGING" \
  --passphrase-file "$BACKUP_PASSPHRASE" \
  --freshness-seconds "$BACKUP_FRESHNESS" \
  --retention-count "$BACKUP_RETENTION" \
  --systemd-drop-in "$NODE_BACKUP_DROP_IN" --replace-existing
sudo systemctl daemon-reload
POST_RECOVERY_CREATED=$(sudo iotkit-edge-nodectl backup create \
  --config "$NODE_BACKUP_CONFIG")
POST_RECOVERY_BACKUP_ID=$(jq -er .backup_id <<<"$POST_RECOVERY_CREATED")
POST_RECOVERY_ARTIFACT="$BACKUP_DESTINATION/$POST_RECOVERY_BACKUP_ID.iotkit-node-backup"
sudo iotkit-edge-nodectl backup inspect \
  --input "$POST_RECOVERY_ARTIFACT" --passphrase-file "$BACKUP_PASSPHRASE" \
  | tee "$install_root/recovery/$CASE/post-recovery-backup-inspection.json" >/dev/null
sudo iotkit-edge-nodectl backup status --config "$NODE_BACKUP_CONFIG" \
  | tee "$install_root/recovery/$CASE/post-recovery-backup-status.json" >/dev/null
sudo jq -e --arg backup_id "$POST_RECOVERY_BACKUP_ID" \
  '.status == "authenticated" and .backup_id == $backup_id' \
  "$install_root/recovery/$CASE/post-recovery-backup-inspection.json" >/dev/null
sudo jq -e --arg backup_id "$POST_RECOVERY_BACKUP_ID" \
  '.status == "healthy" and .backup_id == $backup_id' \
  "$install_root/recovery/$CASE/post-recovery-backup-status.json" >/dev/null
# Retain the encrypted artifact through the approved off-host custody procedure,
# then point this variable at that retained copy (or the artifact itself when the
# configured destination is already an approved off-host mount).
: "${POST_RECOVERY_OFF_HOST_ARTIFACT:?set the retained off-host artifact path}"
sudo test -s "$POST_RECOVERY_OFF_HOST_ARTIFACT"
sudo iotkit-edge-nodectl backup inspect \
  --input "$POST_RECOVERY_OFF_HOST_ARTIFACT" \
  --passphrase-file "$BACKUP_PASSPHRASE" \
  | tee "$install_root/recovery/$CASE/post-recovery-off-host-inspection.json" >/dev/null
sudo jq -e --arg backup_id "$POST_RECOVERY_BACKUP_ID" \
  '.status == "authenticated" and .backup_id == $backup_id' \
  "$install_root/recovery/$CASE/post-recovery-off-host-inspection.json" >/dev/null
jq -n --arg backup_id "$POST_RECOVERY_BACKUP_ID" \
  '{backup_id: $backup_id, authenticated: true, healthy: true,
    off_host_copy_verified: true}' \
  | tee "$install_root/recovery/$CASE/post-recovery-backup-evidence.json" >/dev/null
chmod 600 "$install_root/recovery/$CASE"/post-recovery-backup-*.json

sudo systemctl stop iotkit-edge-node.service
sudo install -d -m 755 /etc/systemd/system/iotkit-edge-node.service.d
printf '[Service]\nEnvironment="IOTKIT_DB_PATH=%s"\n' "$CANDIDATE_DB" \
  | sudo tee /etc/systemd/system/iotkit-edge-node.service.d/50-recovered-database.conf \
    >/dev/null
sudo chmod 644 \
  /etc/systemd/system/iotkit-edge-node.service.d/50-recovered-database.conf
sudo systemctl daemon-reload
sudo systemctl start iotkit-edge-node.service

while :; do
  edge_cli recovery report --control-socket "$EDGE_CONTROL_SOCKET" \
    --recovery-id "$RECOVERY_ID" \
    | sudo tee "$install_root/recovery/$CASE/final-report.json" >/dev/null
  sudo jq -e \
    '.state == "completed" and .completion_acknowledged == true and .cursor_converged == true' \
    "$install_root/recovery/$CASE/final-report.json" >/dev/null && break
  sleep 5
done
sudo chmod 600 "$install_root/recovery/$CASE/final-report.json"
```

The matching completion and completion ACK do not manufacture an admin
credential. The interactive local passphrase reset is a separate required
authority step: it establishes new ownership and revokes any remaining
operator/session authority before normal startup. If authenticated HTTP ingest
is used, reapply its desired listener, TLS generation, and device authority
through the normal typed operations after the reset. Restore cleared the
applied listener generation, so HTTP ingest remains closed until that explicit
reapplication succeeds. A recovered Node must also produce a fresh encrypted
backup, authenticate it, observe healthy backup status, and retain the artifact
off host before the incident is closed. `--replace-existing` is required here:
without it the timer still names the old database. Failure to complete this
evidence is still a recovery failure, not a reason to bypass the ownership
fence.

This backup reconfiguration and evidence block applies because this procedure
returns an encrypted-backup candidate to production. A site may choose not to
configure scheduled backups at all; backup configuration remains optional.
Such a site cannot use this encrypted-backup recovery procedure and instead
follows the separately accepted no-backup replacement loss boundary.

The final report preserves the Node `backup_created_at` and Edge/Broker
`broker_fenced_at` observations but reports `recovery_window_ms=null` because
those independent clocks do not establish a duration. It also includes the
snapshot boundary, replay count, expected and currently accepted new-epoch
cursor, and Edge-only post-backup range. `cursor_converged=true` proves replay reached
IoTKit Edge durable raw custody. `remaining_gap_review_required` remains true
because a lost old host cannot prove whether it allocated an additional local
tail after the authenticated snapshot; record that explicit loss boundary in
the incident review.

The repository does not define a universal Edge Node systemd unit. The unit
name and `NODE_RUNTIME_USER`/`NODE_RUNTIME_GROUP` above must match the deployed
supervisor. Verify the service runs as that account and can write the candidate
database before retiring evidence. Never re-enable the old credential or old
database; retain them as incident evidence and retire the old host.

No-backup replacement restores neither readings nor dedup claims. A legacy
snapshot, plaintext DB copy, SQL edit, or invented handoff is never a fallback.

### 8.2 IoTKit Edge restore

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
Without an encrypted backup and a later permitted handoff, a replacement does not restore readings
or dedup claims. An encrypted-backup candidate is still fenced until the separately contracted
permit and credential-generation checks complete.

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
