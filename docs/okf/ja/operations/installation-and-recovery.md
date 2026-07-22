---
type: Runbook
title: "IoTKit Edgeの導入と復旧"
description: "導入、日常確認、証明書、account、backup、restore、移行、rollbackの完全な手順です。"
language: ja
translation_key: operations.installation-and-recovery
status: stable
revision: 2
---

# IoTKit Edgeの導入と復旧

一つのIoTKit Edge deploymentに対するoperatorの入口です。Router、DNS、IP払出し、firewall、VPNの設定はIoTKitの範囲外です。

Release候補を現場へ持ち込む前に、既存DB・credentialを再利用せず、新しいreport directoryへhost統合gateを実行します。PostgreSQL clean install、Console、疑似Edge Node 2台、意味付け、external MQTT、restart/通信断、暗号化backup/restore、certificate rollback、両storage profileのcapacity smokeを通します。実BravePI、対象hardwareのcapacity測定、Windows+Caddy確認の代替ではありません。

```bash
scripts/test-edge-host-release-gate.sh /secure/report/iotkit-v1-YYYYMMDD
```

## 1. 導入

1. 全Edge Nodeへ異なる`edge_node_id`を与え、`mqtt-binding`を出力する。
2. IoTKit Edge hostを対象にするDNS名、full-chain server certificate、owner-only private key、root trust bundleを準備する。
3. 最初のEdge Nodeについて`scripts/bootstrap-edge.sh`を実行する。Bootstrapは起動前にIoTKit Edge source IDを割り当て、`iotkit-edge-output-<edge-id>`のwrite ACLをそのEdgeのIoTKit/YokaKit Observation/status namespaceだけに限定する。追加legacy topicだけ`--edge-publish-topic`を繰り返す。
4. `embedded`は`deploy/compose.edge.yaml`、`postgres`はさらに`deploy/compose.edge-postgres.yaml`を重ねて起動する。Profile metadataと起動profileが違えば停止する。
5. Owner-only password fileから`iotkit-edge account bootstrap`を実行し、最初の`system_admin`を作る。使用後fileを削除する。
6. 生成したEdge Node handoffを保護channelで転送する。Broker enrollmentはMQTT接続とexact topic権限だけで、raw custodyを許可しない。
7. Edge Nodeを起動し、Consoleの**機器管理 / 収集ノード**へ**未登録**として現れるまで待つ。期待する名称・診断identity・data generationを確認して**収集ノードを登録**する。Settings adminまたはsystem adminだけが実行可能。
8. **登録済み**を待ちcommissioning smokeを実行する。Durable acceptance後にsensor表示と意味を設定する。

新しいIoTKit Edgeへ旧DBやcredentialをcopyしません。登録はfresh publication streamに一度だけ行います。登録前に集めた値はIoTKit Edge custody外で、承認後にreplayしません。CaddyだけがLAN向けHTTPSを提供し、loopback HTTPへproxyします。HTTPS障害時もplaintext LANへfallbackしません。

## 2. 日常確認

- **状態:** IoTKit Edge、signal数、意味未設定、certificate残日数。
- **機器管理 / 収集ノード:** discovery、登録、最終descriptor通信、診断対象generation。**登録済み**は認可stateで、online保証ではない。
- **モニター:** current valueと最終受信。古いsignalはsensor、Adapter、Edge Node、Broker、IoTKit Edgeの順に確認。
- **受信履歴:** sensor・Edge Node・期間を一画面で絞り、aggregate graphとrecent rawを確認。同条件CSVは汎用Observation exportで業務帳票ではない。
- **出力:** Active purpose-bound route。Pending publicationはBroker PUBACKまで削除しない。
- **システム:** Filesystem、DB size、raw/semantic/outbox件数、最終backup、原因別診断。Console応答だけでEdge Node/Broker正常と判断しない。
- `postgres`はSQLからnamed volume空き容量を得られない。Host監視へ`docker compose ... exec postgres df -Pk /var/lib/postgresql/data`を追加し、使用率90%または空き2 GiBでwarning、512 MiBでcritical。
- **監査:** Display name、意味、出力、accountを誰が変更したか。
- `iotkit-edge-nodectl smoke status`: MQTT PUBACKではなくIoTKit Edge durable acceptance。
- `scripts/iotkit-broker-cert status --config DEPLOYMENT/broker-cert.env`: Certificate期限とbundle validation。

## 3. Certificate自動更新

`scripts/iotkit-broker-cert`はIoTKitのsensor意味やYokaKitから独立し、Broker host上のMosquitto/Caddy certificate bundleを管理します。

- `install`はchain、hostname、expiry、keyを検証し、3 fileをswitchし、Mosquitto reload、trust再読込のためIoTKit Edge restart、Caddy reload、新MQTT TLS/HTTPS probeを行う。
- Probe失敗時は3 fileを前versionへ戻しserviceをreloadする。
- `renew`は`lego`から設定ACME serverへ要求し、root trust bundleを維持し、同じvalidated install pathでfull chain/keyを入れる。HTTP-01はCaddy ACME webroot、DNS-01は`IOTKIT_CERT_LEGO_CHALLENGE=dns`、provider、credential environmentで選択する。
- 生成systemd service/timerを`/etc/systemd/system`へcopyし、owner-only `broker-cert.env`へACME email/serverを入れtimerをenableする。Daily randomized checkで通常更新は無人実行する。

初期DNS、ACME account、provider credentialはinstallation作業です。`IOTKIT_CERT_CA_FILE`はEdge Node/IoTKit Edgeが信頼するrootを含み、`lego`出力のintermediateで置換しません。Consoleは期限を表示しますがcertificateを発行・置換しません。

## 4. Account復旧

System administrator復旧はIoTKit Edge hostだけで実行します。`embedded`例:

```bash
docker compose --env-file "$install_root/edge.env" -f deploy/compose.edge.yaml \
  run --rm -v /owner-only/new-password:/run/iotkit/new-password:ro \
  edge account recover --storage-profile embedded --db /data/edge.db \
  --storage-metadata /run/iotkit/storage-profile.json --login-id admin \
  --password-file /run/iotkit/new-password
```

`postgres`はoverlayとconnection fileを明示します。

```bash
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml -f deploy/compose.edge-postgres.yaml \
  run --rm -v /owner-only/new-password:/run/iotkit/new-password:ro \
  edge account recover --storage-profile postgres \
  --postgres-config /run/iotkit/postgres.json \
  --storage-metadata /run/iotkit/storage-profile.json --login-id admin \
  --password-file /run/iotkit/new-password
```

Recoveryは既存sessionを失効します。Password、MQTT credential、private key、session tokenをargument、log、audit summary、Gitへ置きません。

## 5. 障害確認順序

1. Edge NodeとIoTKit Edge両方のDBを保全する。
2. Consoleとservice logを読み、最初にidentityを作り直さない。
3. DNS/routeとcertificateを確認する。
4. Mosquitto authとexact-topic ACLを確認する。
5. Edge Node `accepted-through`を確認する。未受理recordはEdge Nodeに残す。
6. IoTKit Edge output queueを確認する。同じObservation identityでretryする。
7. 復旧後、raw cursorとpending outputの収束前に保持dataを削除しない。

## 6. Edge Node登録の復旧

- **未登録:** Descriptorは見えているがrecord batchを保存・ackしない。
- **登録処理中:** Durable state。同じrequestをmatching result commitまでretryするため、各service restart後に再登録しない。
- **復旧確認待ち:** Descriptor、stored generation、activation resultがconflict。両DBを保全しidentity/restore履歴を調べ、row削除、別identity発行、state table直接編集をしない。
- Publication sequenceを一度でもallocateしたstreamへfresh activationを行わない。V1はstandalone outbox adopt、reactivation、Edge間transfer、identity reuseをしない。
- 登録はMQTT credentialをcreate/rotate/revokeせずBroker enrollmentを置換しない。
- 登録時にlocal reading boundaryを固定し、旧prefixをbounded background cleanupする。Normal processingから見えなくしますが、SQLite page、backup、mediaからのforensic eraseは保証しない。

## 7. 暗号化backup

IoTKit Edge DBはsensor historyのほかaccount/session hash、設定、audit、pending outboxを含みます。通常backupにplaintext DB copyを使いません。12文字以上のpassphraseをowner-only fileから渡します。

```bash
install_root="$HOME/.local/share/iotkit/edge-01"
backup_root="$HOME/.local/share/iotkit/backups/edge-01"
mkdir -p "$backup_root"
install -m 600 /dev/null "$install_root/secrets/backup-passphrase"
# Shell historyへ残さない方法でpassphraseを書き込む。
docker compose --env-file "$install_root/edge.env" -f deploy/compose.edge.yaml \
  run --rm -v "$backup_root:/backup" \
  -v "$install_root/secrets/backup-passphrase:/run/iotkit/backup-passphrase:ro" \
  edge backup create --db /data/edge.db \
  --output "/backup/edge-$(date +%Y%m%d-%H%M%S).iotkit-backup" \
  --passphrase-file /run/iotkit/backup-passphrase
```

`postgres`ではPostgreSQL toolを含むoverlay、profile、owner-only connection fileを渡します。成功JSONはformat、Edge ID、schema、raw件数、DB hashを含みます。ContainerはArgon2idとXChaCha20-Poly1305で暗号化・改ざん検知し、mode `0600`で新規作成し上書きしません。Console最終backup更新とoff-host copyを確認します。MQTT credential、certificate、private keyは含まれません。

Pre-encryption snapshotは専用tmpfsへ置きます。Host CLIもowner-only、backup対象外、restart時消去領域を`TMPDIR`にします。CLIはscheduleしないためOS/運用基盤から定期実行し、off-host copy、失敗通知、restore drillを用意します。

## 8. Restore

必ず新DB pathへrestoreし、live DBを直接上書きしません。

```bash
docker compose --env-file "$install_root/edge.env" -f deploy/compose.edge.yaml stop edge
docker compose --env-file "$install_root/edge.env" -f deploy/compose.edge.yaml \
  run --rm -v "$backup_root:/backup:ro" \
  -v "$install_root/secrets/backup-passphrase:/run/iotkit/backup-passphrase:ro" \
  edge backup restore --input /backup/SELECTED.iotkit-backup \
  --db /data/edge.restore-candidate.db \
  --passphrase-file /run/iotkit/backup-passphrase
docker compose --env-file "$install_root/edge.env" -f deploy/compose.edge.yaml \
  run --rm edge diagnose --db /data/edge.restore-candidate.db
```

Encryption、manifest、DB hash、`quick_check`、Edge ID、cursorを検証し、全browser sessionを失効してrestore historyをtransaction記録します。検証後、旧`edge.db`と`-wal`/`-shm`を同じ退避directoryへ移しcandidateをrenameします。収束まで旧DBを削除しません。

`postgres`はtableのない新DBへだけrestoreします。Edge停止中に`iotkit_restore`等を作り、通常credential/host/portでdatabase名だけ変えたmode `0600`の一時connection fileを使います。Manifest、Edge ID、schema、cursor、pending outboxを確認し、二人確認後に旧DBを退避、新DBを通常名へrenameします。Startup、cursor、outbox確認まで旧DBを残し、通常停止で`docker compose down --volumes`を使いません。

古いbackupより先からEdge Nodeが再開した場合、ackせず`recovery_hold`にします。他backup/旧DBで回収不能と判断した最後だけ、次のようにlossを監査して永久retryを止めます。欠損修復ではなく、SQLでcursor/stateを変更しません。

```bash
iotkit-edge backup accept-archive-loss --storage-profile embedded --db /path/edge.db \
  --edge-node-id EDGE --ledger-epoch EPOCH \
  --confirm-edge-id EDGE_ID --reason '元DB故障、他の検証済みbackupなし'
```

PostgreSQLでは同じcommandへ`--storage-profile postgres --postgres-config FILE --storage-metadata FILE`を渡します。

## 9. Device retireとhardware交換

Device正本ledgerはEdge Nodeにあります。Console row編集を交換扱いにしません。終了は`iotkit-edge-nodectl device retire`、個体識別hardware交換は`device replace`を使います。Candidate profileと既存seriesを照合し、`system_id`を維持してhardwareだけを交換します。Forced/unchecked executionを通常手順にしません。

## 10. SQLiteからPostgreSQLへのoffline移行

IoTKit Edgeを停止し、未ack dataはBroker/Edge Nodeに保持させます。Dual-write、自動fallbackをせず、IoTKit tableのない空DBへ移行します。Running Edgeがdeployment lockを持つため停止忘れでは開始しません。Protected consistent snapshotから全tableをcopyします。Connection情報はmode `0600` JSONへ置き、DSN/passwordをargvへ渡しません。

```json
{"dsn":"postgres://iotkit:REDACTED@postgres:5432/iotkit?sslmode=require"}
```

```bash
install -m 600 /dev/null /run/iotkit/postgres.json
# Secretをshell historyへ残さない方法でJSONを書く。
iotkit-edge storage migrate \
  --from-sqlite /data/edge.db \
  --to-postgres-config /run/iotkit/postgres.json \
  --report /data/sqlite-to-postgres-report.json
```

成功reportはprofile、Edge ID、schema、全table件数、cursor vector、全row digest、`completed: true`を含み、mode `0600`で新規作成します。旧SQLiteを残し、PostgreSQLでConsole history、pending outbox、cursor再収束を確認します。不一致・途中失敗時は移行先を使わず、空DBを再作成して再実行します。

## 11. Manual updateとrollback

1. 暗号化backupを作りConsole表示を確認する。
2. Git commit、Compose設定、image IDを記録し、credential/keyをGitへ入れない。
3. 新versionを取得・buildし、Brokerを動かしたままEdgeだけ停止する。
4. 新Edgeを起動する。Schema migrationはstartup transaction。
5. HTTPS login、diagnostics、cursor再収束、pending outbox、history graph、CSVを確認する。
6. 失敗時はEdgeを停止する。旧binaryでmigration済みDBを開かず、旧commit/imageへ戻し、更新前backupを**新candidate DB**へrestoreして§8と同じswapを行う。Identity/credentialを作り直さない。

これはmanual updateです。Migration後にimageだけ戻すのはrollbackではなく、対応する更新前DBも戻します。
