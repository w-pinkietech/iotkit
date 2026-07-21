# IoTKit Edge installation and recovery

This is the operator entry point for one IoTKit Edge deployment. IoTKit does not
configure routers, DNS, IP address allocation, firewalls, or VPNs.

## 1. Install

1. Give every Edge Node a different `edge_node_id` and export its `mqtt-binding`.
2. Prepare a DNS name, a full-chain server certificate, its private key, and a
   root trust bundle that covers the IoTKit Edge host. The key file must be owner-only.
3. Run `scripts/bootstrap-edge.sh` for the first Edge Node. Bootstrap assigns the
   IoTKit Edge source ID before startup and gives `iotkit-edge-output-<edge-id>` write access only to
   that IoTKit Edge's IoTKit/YokaKit observation and status namespace. Use repeated
   `--edge-publish-topic` only for additional exact legacy application topics.
4. Start `deploy/compose.edge.yaml`.
5. Create the first `system_admin` with `iotkit-edge account bootstrap` and an
   owner-only password file. Delete that file afterwards.
6. Transfer each generated Edge Node handoff through a protected channel. This
   Broker enrollment only gives the Edge Node its MQTT connection and exact-topic
   permissions; it does not authorize IoTKit Edge raw-data custody.
7. Start the Edge Node and wait for it to appear as **未登録** in **機器管理 / 収集ノード**. Confirm
   the expected Edge Node name or diagnostic identity and data generation, then use
   **収集ノードを登録**. Only a settings administrator or system administrator can do
   this.
8. Wait for **登録済み**, then run the commissioning smoke. Configure the sensor
   display and meaning only after the smoke is durably accepted.

Do not copy an old database or credential into a new IoTKit Edge. Registration is a
one-time operation for a fresh publication stream. Values collected before
registration remain outside IoTKit Edge custody and are not replayed after approval.

The generated Caddy endpoint serves HTTPS and proxies only to IoTKit Edge's loopback
HTTP listener. If HTTPS is broken, IoTKit does not expose a plaintext LAN
fallback.

## 2. Daily checks

- **状態**: IoTKit Edge, signal count, missing meaning, and certificate days remaining.
- **機器管理 / 収集ノード**: discovery, registration, the last descriptor communication, and
  the exact data generation used for diagnosis. **登録済み** is an authorization
  state; it does not mean the Edge Node is currently online.
- **モニター**: current value and last receipt. A stopped or old signal must be
  investigated at the sensor, adapter, Edge Node, broker, then IoTKit Edge—in that order.
- **受信履歴**: sensor、Edge Node、期間を同じ画面で絞り込み、集約graphと直近rawを
  確認する。同じ条件のCSVは汎用観測の持ち出しであり、業務帳票ではない。
- **出力**: active purpose-bound routes. Pending output is not deleted until
  broker PUBACK.
- **システム**: filesystem使用率、DB size、raw/意味付け/outbox件数、最終backup、
  原因別の診断を確認する。「Console応答中」はEdge NodeやBrokerの正常を意味しない。
- **監査**: who changed a display name, meaning, output, or account.
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

Only the IoTKit Edge host can recover a system administrator:

```bash
iotkit-edge account recover --db /path/edge.db --login-id admin \
  --password-file /owner-only/new-password
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

- **未登録** means IoTKit Edge has seen an Edge Node descriptor but will reject its record
  batches without acknowledging them.
- **登録処理中** is durable. Broker, IoTKit Edge, or Edge Node restart does not require a
  second registration; the same request is retried until the matching Edge Node
  result is committed.
- **復旧確認待ち** means the descriptor, stored generation, or activation result
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

IoTKit Edge DBにはsensor履歴だけでなく、account/session hash、設定、監査、未配送outboxも
含まれる。通常の運用backupとしてDB fileを平文コピーしない。backupの合言葉は
12文字以上とし、所有者だけが読めるfileから渡す。

稼働中のIoTKit Edgeから整合snapshotを作成できる。次はCompose導入時の例である。

```bash
install_root="$HOME/.local/share/iotkit/edge-01"
backup_root="$HOME/.local/share/iotkit/backups/edge-01"
mkdir -p "$backup_root"
install -m 600 /dev/null "$install_root/secrets/backup-passphrase"
# 対話可能なeditor等で合言葉を書き、shell履歴へ載せない。
docker compose --env-file "$install_root/edge.env" -f deploy/compose.edge.yaml \
  run --rm \
  -v "$backup_root:/backup" \
  -v "$install_root/secrets/backup-passphrase:/run/iotkit/backup-passphrase:ro" \
  edge backup create --db /data/edge.db \
  --output "/backup/edge-$(date +%Y%m%d-%H%M%S).iotkit-backup" \
  --passphrase-file /run/iotkit/backup-passphrase
```

成功時はformat、IoTKit Edge ID、schema、raw件数、DB hashを含むmanifestをJSONで返す。
containerはArgon2idとXChaCha20-Poly1305で暗号化・改ざん検知され、mode `0600`で
新規作成される。同名fileは上書きしない。Consoleの最終backup時刻が更新されたこと、
別mediaにも暗号化containerを複製できたことを確認する。MQTT credential、certificate、
private keyはbackupに含まれないため、導入設定側で別に復旧する。

## 8. IoTKit Edge restore

復元先は必ず新しいDB pathにする。稼働DBへ直接上書きしない。

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

restoreは暗号・manifest・DB hash・`quick_check`・IoTKit Edge ID・cursorを照合し、全browser
sessionを失効して復元履歴をtransactionで記録する。検証後、host上で元の`edge.db`と
その`-wal`/`-shm`を一つの退避directoryへ移し、candidateを`edge.db`へrenameしてから
IoTKit Edgeを起動する。元DBは収束確認まで削除しない。

古いbackupのcursorより先からEdge Nodeが再開した場合、IoTKit EdgeはackせずEdge Nodeを
`recovery_hold`にする。`iotkit-edge diagnose`とConsoleは失われる可能性のあるcursor
範囲を表示する。別backupや元DBから回収できないと判断した場合に限り、IoTKit Edge IDと理由を
明示して次を実行する。

```bash
iotkit-edge backup accept-archive-loss --db /path/edge.db \
  --edge-node-id EDGE --ledger-epoch EPOCH \
  --confirm-edge-id SITE_ID --reason '元DB故障、他の検証済みbackupなし'
```

これは欠損を修復する操作ではない。`archive_lost`を監査し、永久retryを止めるための
最終判断である。SQLでcursorやEdge Node stateを直接変更してはならない。

## 9. Device retirement and hardware replacement

deviceの正本台帳はEdge Nodeにあり、IoTKit Consoleの表示行を編集して交換扱いにはしない。
使用終了は`iotkit-edge-nodectl device retire`、個体識別型deviceの交換は
`iotkit-edge-nodectl device replace`を使う。replaceは候補の観測profileと既存seriesを照合し、
`system_id`を維持してhardwareだけを交換する。強制指定と確認なし実行を通常手順にしない。
Edge Node descriptorがIoTKit Edgeへ届くと、retired状態と交換後の継続seriesがConsoleへ反映される。

## 10. Manual IoTKit Edge update and rollback

1. 上記の暗号化backupを作り、Consoleの最終backup表示を確認する。
2. 現在のGit commit、Compose設定、IoTKit Edge image IDを記録する。credentialや秘密鍵はGitへ
   入れない。
3. 新versionを取得してIoTKit Edge imageをbuildする。Brokerは動かしたままIoTKit Edgeだけを停止する。
   停止中もEdge Nodeは未ack recordを保持する。
4. 新IoTKit Edgeを起動する。schema migrationは起動時にtransactionで実行される。
5. HTTPS login、`/api/v1/system/diagnostics`、Edge Node cursorの再収束、未配送outbox、履歴graph、
   CSVを確認する。問題がなければ旧imageと更新前DBの退避を保持期間後に片付ける。
6. 起動・migration・health確認に失敗した場合はIoTKit Edgeを停止する。旧binaryでmigration済みDBを
   開こうとせず、旧commit/imageへ戻し、更新前backupを**新しいcandidate DB**へ復元して
   §8と同じswapを行う。Broker/Edge Node identityやcredentialを作り直さない。

この手順はmanual updateであり、自動更新ではない。DB migration後にimageだけを戻す操作は
rollbackではない。更新前backupからDBも対で戻す。
