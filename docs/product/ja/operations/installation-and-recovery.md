---
type: Runbook
title: "IoTKit Edgeの導入と復旧"
description: "導入、日常確認、証明書、account、backup、restore、移行、rollbackの完全な手順です。"
language: ja
translation_key: operations.installation-and-recovery
status: stable
revision: 19
---

# IoTKit Edgeの導入と復旧

一つのIoTKit Edge deploymentに対するoperatorの入口です。Router、DNS、IP払出し、firewall、VPNの設定はIoTKitの範囲外です。

Rust製IoTKit Edgeは固有のfresh schemaから開始します。以前のGo実装が作成したDBと
暗号化backup artifactは受理、変換、restoreしません。必要な業務dataはcutover前に
exportし、clean installを行います。

Release候補を現場へ持ち込む前に、既存DB・credentialを再利用せず、新しいreport directoryへhost統合gateを実行します。PostgreSQL clean install、Console、疑似Edge Node 2台、意味付け、external MQTT、restart/通信断、暗号化backup/restore、certificate rollback、両storage profileのcapacity smokeを通します。実BravePI、対象hardwareのcapacity測定、Windows+Caddy確認の代替ではありません。

```bash
scripts/test-edge-host-release-gate.sh /secure/report/iotkit-v1-YYYYMMDD
```

## 1. 導入

1. 全Edge Nodeへ異なる`edge_node_id`を与え、`mqtt-binding`を出力する。
2. IoTKit Edge hostを対象にするDNS名、full-chain server certificate、owner-only private key、root trust bundleを準備する。
3. 最初のEdge Nodeについて`scripts/bootstrap-edge.sh`を実行する。Bootstrapは起動前にIoTKit Edge source IDを割り当て、`iotkit-edge-output-<edge-id>`のwrite ACLをそのEdgeのIoTKit/Pinikiet Observation/status namespaceだけに限定する。追加legacy topicだけ`--edge-publish-topic`を繰り返す。
4. `embedded`は`deploy/compose.edge.yaml`、`postgres`はさらに`deploy/compose.edge-postgres.yaml`を重ねて起動する。Profile metadataと起動profileが違えば停止する。
5. Owner-only password fileから`iotkit-edge account bootstrap`を実行し、最初の`system_admin`を作る。使用後fileを削除する。
   初回ログイン後、設定管理者とsystem adminの概要画面には**利用開始までの設定**が表示される。これは
   収集ノード登録、デバイス名・設置場所、センサー種類・単位、値の使い方の順に、既存の耐久状態から
   未完了の最初の1操作だけを案内する。閲覧担当者には変更導線を表示せず、4項目が完了すると案内は消える。
   外部出力はdeploymentによって不要なため完了条件に含めず、必要な場合の次工程として案内する。
6. 生成したEdge Node handoffを保護channelで転送する。Broker enrollmentはMQTT接続とexact topic権限だけで、raw custodyを許可しない。
7. Edge Nodeを起動し、Consoleの**機器管理 / 収集ノード**へ**未登録**として現れるまで待つ。期待する名称・診断identity・data generationを確認して**収集ノードを登録**する。Settings adminまたはsystem adminだけが実行可能。
8. **登録済み**を待ちcommissioning smokeを実行する。Durable acceptance後にsensor表示と意味を設定する。

新しいIoTKit Edgeへ旧DBやcredentialをcopyしません。登録はfresh publication streamに一度だけ行います。登録前に集めた値はIoTKit Edge custody外で、承認後にreplayしません。CaddyだけがLAN向けHTTPSを提供し、loopback HTTPへproxyします。HTTPS障害時もplaintext LANへfallbackしません。

## 2. 日常確認

- **状態:** IoTKit Edge、signal数、意味未設定、certificate残日数。
- **機器管理 / 収集ノード:** discovery、登録、最終descriptor通信、診断対象generation。**登録済み**は認可stateで、online保証ではない。
- **ライブ:** 有効な計測ruleごとにcardを表示し、calibrationとruleを適用した最新の処理済みcurrent valueと最終受信を、graphとは独立して示す。Graphは画面を開いてからの結果だけを示す。同じsignalに複数の有効ruleがあれば別cardになり、ruleがなければ設定案内を示す。数値は1秒bucketの折れ線を直近60秒、boolean/alarmは同じ1秒bucketから導出した最新10状態変化を表示する。Graphは最大でも直近60秒に限定する。開始後の処理済み値がまだなければ、過去のcurrent valueが表示できる場合でもgraphを空にして待機中と示し、cardからsensor詳細へ進める。Browserは画面がvisibleな間だけ5秒ごとに表示領域内の最大12件を更新する。一度取得した最終受信の経過時間とgraphの時間窓は、IoTKit Edgeの画面開始時刻を基準にBrowserの単調な経過時間で進めるため、一時的に再取得へ失敗しても進み続ける。未受信は明示し、5分以上新着がないruleは停止と断定せず**要確認**にする。Rawと過去dataは**受信履歴**で確認する。要確認時はsensor、Adapter、Edge Node、Broker、IoTKit Edge、semantic projectionの順に確認する。ライブとSensor詳細の**実信号プレビュー**のgraph横軸は有効なsemantic observed/event time、最終受信とcurrentの鮮度はIoTKit Edgeのraw receipt timeを使う。実信号プレビューは同じ直近60秒・1秒bucket graphで、bounded input history全体を評価してbooleanと累積のstateを維持する。累積ruleのresult cardには保存済みcurrent totalを示し、実信号プレビューには仮計算の直近60秒deltaを明記し、このgraph自体は直近60秒のままにする。保存済みruleは別のstaircase graphで、選択した保存済みruleの表示開始後の累積を示す。永続化済みcurrentの変化を追加し、表示開始後の最新最大60点だけを保持する。61点目から最古点を外すため、rolling 60秒の履歴requestから古い変化が外れても、最大60点の範囲では画面中のsession変化を消さない。新規draftは保存後に累積開始と示す。Staircaseは1秒bucketの平均ではなく、保存順の保存済みcurrent stateを示す。正常に保存済みの点を取得できないsessionは表示開始後の保存済み変化なしと示し、履歴取得失敗は取得できないと示す。
- **受信履歴:** sensor・Edge Node・期間を一画面で絞り、選択中sensorと一致するbounded graphとrecent rawを確認。Raw graphの横軸はIoTKit Edge receipt日時を表示time zoneで示し、縦軸は値の範囲とsensor単位を示す。同条件CSVは汎用Observation exportで業務帳票ではない。
- **出力:** Active purpose-bound route。Pending publicationはBroker PUBACKまで削除しない。
- **システム:** Filesystem、DB size、raw/semantic/outbox件数、最終backup、原因別診断。Console応答だけでEdge Node/Broker正常と判断しない。
- `postgres`はSQLからnamed volume空き容量を得られない。Host監視へ`docker compose ... exec postgres df -Pk /var/lib/postgresql/data`を追加し、使用率90%または空き2 GiBでwarning、512 MiBでcritical。
- **監査:** Display name、意味、出力、accountを誰が変更したか。
- `iotkit-edge-nodectl smoke status`: MQTT PUBACKではなくIoTKit Edge durable acceptance。
- `scripts/iotkit-broker-cert status --config DEPLOYMENT/broker-cert.env`: Certificate期限とbundle validation。

Consoleの絶対日時はIoTKit Edge起動時の表示time zoneを使う。Compose deploymentはowner-only `edge.env`の`IOTKIT_DISPLAY_TIME_ZONE=Asia/Tokyo`などIANA time zoneで設定し、直接起動は`iotkit-edge serve --display-time-zone Asia/Tokyo ...`でも指定できる。省略時は`UTC`で、不正な値はstartup errorにする。Raw storage、API、CSVのtimestampはUnix msのままで、この設定は表示だけを変える。

## 3. Certificate自動更新

`scripts/iotkit-broker-cert`はIoTKitのsensor意味やPinikietから独立し、Broker host上のMosquitto/Caddy certificate bundleを管理します。

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

## 7. 暗号化バックアップ

### 7.1 Optional Edge Node暗号化backup

Edge Node backupはlocal-rootで行う別operationです。既定ではconfigurationせず、
timerもenableしません。これは[Edge Node復旧契約](../contracts/edge-node-recovery-v1.md)
に従うcustody-completeなsanitized SQLite backupを作ります。Snapshot sanitizerは
`target_registry`のdeployment credential tokenを空にします。MQTT/TLS private
materialはこのDBの外にありartifactへ入れません。Account、session、device
credential hashはprotected DB stateとして残り得るため、artifactは暗号化しsecret
として扱います。Legacy plaintext snapshot fallbackもありません。

Owner-only configurationとpassphrase fileを使います。Passphraseをargument、shell
history、log、systemd unitへ置きません。Destinationはcapability probeを先に通し、
owner-only writable directory、stableに識別できるmount、必要capacity、no-replace・
read-back・parent-syncを備えなければなりません。Filesystem labelやmutable device
nameだけでは不十分で、live DBとは異なるfilesystemでなければなりません。

`/run`はstaging tmpfs parentです。`configure`はfinal path componentをfollowせず既存
parentをopenし、euid所有でgroup/other writableでないtmpfs directory（通常の`/run`
mode `0755`は可、world-writableな`/dev/shm` rootは不可）であることを検証します。
正確な`/run/iotkit-edge-node-backup` leafを記録し、missing parent treeは作りません。
`create`はheld parent descriptorからabsentなexact leafだけをmode `0700`で作ります。
既存leafは同じtmpfs上のowner-only directoryで、link countとtypeを検証して受け入れます。
従ってServiceの`RuntimeDirectory=iotkit-edge-node-backup`が受け入れ可能なleafを供給します。
任意の`/run` treeを先に作ったり拡張したりせず、destinationやpersistent databaseを
`TMPDIR`へ置きません。

```bash
sudo install -d -m 0700 /etc/iotkit
if ! sudo test -e /etc/iotkit/edge-node-backup-passphrase; then
  sudo install -m 600 /dev/null /etc/iotkit/edge-node-backup-passphrase
fi
sudo chmod 600 /etc/iotkit/edge-node-backup-passphrase
# Shell historyへ残さない方法でpassphraseを対話的に書き込む。
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

Configure commandはowner-only configurationとexact drop-inを一つのguarded pairとして
publishします。生成されたmount pointを確認します。Drop-inは次だけです。

```ini
[Unit]
RequiresMountsFor=/absolute/captured/mount/point
```

Timerはoperatorが明示的にopt-inするまでdisabledです。

```bash
sudo systemctl enable --now iotkit-edge-node-backup.timer
sudo systemctl status iotkit-edge-node-backup.timer
```

Enable前にmanualのnon-secret surfaceを使い、artifactをoff-hostで確認します。

```bash
sudo iotkit-edge-nodectl backup create --config /etc/iotkit/edge-node-backup.json
sudo iotkit-edge-nodectl backup inspect --input /mnt/iotkit-backups/edge-node-01/SELECTED.iotkit-node-backup \
  --passphrase-file /etc/iotkit/edge-node-backup-passphrase
sudo iotkit-edge-nodectl backup status --config /etc/iotkit/edge-node-backup.json
```

Passphraseはdeploymentのapproved encrypted owner-only procedureでescrowし、各暗号化
artifactのoff-host copyを保持します。失ったpassphraseではartifactを復元できません。
Create失敗はdurable backupではなく、live DBの削除・置換を許可しません。

### 7.2 IoTKit Edge暗号化backup

IoTKit Edge DBはsensor historyのほかaccount/session hash、設定、audit、pending outboxを含みます。通常backupにplaintext DB copyを使いません。12文字以上のpassphraseをowner-only fileから渡します。


```bash
install_root="$HOME/.local/share/iotkit/edge-01"
backup_root="$HOME/.local/share/iotkit/backups/edge-01"
mkdir -p "$backup_root"
if [ ! -e "$install_root/secrets/backup-passphrase" ]; then
  install -m 600 /dev/null "$install_root/secrets/backup-passphrase"
fi
chmod 600 "$install_root/secrets/backup-passphrase"
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

### 8.1 Edge Nodeを暗号化backupから本番復帰する

旧機を停止して物理的に隔離した後も、まずBroker credentialを失効させます。`fence-edge-node.sh`
は同梱Mosquitto passwordを世代更新し、Brokerをrestartして既存sessionを切断します。
新passwordと非secret receiptはowner-onlyの新directoryへ一度だけ出力されます。

```bash
set -euo pipefail
umask 077
CASE="edge-node-${EDGE_NODE_ID}-$(date +%Y%m%d%H%M%S)"
EDGE_CONTROL_SOCKET="/data/recovery-control.sock"
INSPECT_STAGING="/run/iotkit-edge-node-recovery-inspect-$CASE"
: "${IOTKIT_REPO_ROOT:?deploy/compose.edge.yamlを含むcheckoutを設定する}"
: "${NODE_RUNTIME_USER:?Edge Nodeのservice accountを設定する}"
: "${NODE_RUNTIME_GROUP:?Edge Nodeのservice groupを設定する}"
getent passwd "$NODE_RUNTIME_USER" >/dev/null
getent group "$NODE_RUNTIME_GROUP" >/dev/null
NODE_RUNTIME_UID=$(id -u "$NODE_RUNTIME_USER")
if [[ "$NODE_RUNTIME_UID" != "$(id -u)" ]]; then
  echo "owner-bound recovery blockはNODE_RUNTIME_USERとして実行してください" >&2
  exit 1
fi
install_root=$(realpath "$install_root")
if [[ "$(stat -c %u "$install_root")" != "$(id -u)" ]]; then
  echo "install_rootのownerとしてdeployment file操作を実行してください" >&2
  exit 1
fi
install -d -m 700 "$install_root/recovery"
LIVE_PARENT=$(realpath "$(dirname -- "$LIVE_DB")")
for owner_bound_path in "$SELECTED" "$PASSPHRASE" "$LIVE_PARENT"; do
  if [[ "$(stat -c %u "$owner_bound_path")" != "$NODE_RUNTIME_UID" ]] ||
    (( (8#$(stat -c %a "$owner_bound_path") & 8#077) != 0 )); then
    echo "backup evidenceとlive DB parentはowner-only NODE_RUNTIME_USER pathが必要です" >&2
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

`prepare`はEdge上のactive old epochとdurable accepted-throughをbackup境界およびBroker
generationと照合し、caseと新epochを保存します。Handoffを手編集せず、candidateはabsent
pathへrestoreします。Restore receipt v2はcandidate instanceとNode側
`device_auth_generation`を含みます。Candidate DBは新規の専用parent directory内へ置き、
共有data directoryを転用しません。Restore操作はlive DBとcandidate parentのownerに
boundされます。Rootは専用tmpfs leaf、candidate parent、handoff、passphraseを準備し、
restoreの**前**に実際のNode service accountへ移管します。Restoreとactivationはともに
そのaccountで実行します。

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

Candidateはmatching requestを受けるまでcollect、publish、HTTP ingestを開始しません。
一つのSQLite transactionでEdge accepted-through以下を収束させ、残るpublicationを新epochへ
連続再採番し、`epoch_start`をseq 1へ置きます。Edgeがmatching resultをdurable commitして
completionを返し、Nodeがそのcompletionを保存するまで通常runtimeはfencedです。Process、
Broker、Edge、candidateのrestartでは同じrequest/result/completion/completion ACKを
再利用します。EdgeはNodeがcompletionをdurably保存してmatching ACKをpublishするまで
completionを保持してretryします。異なるcandidate、artifact、epoch、generation、cursorは
`recovery_hold`です。

`backup activate`の`recovered`はcandidateがcompletionを保存した証拠だけで、まだ
production-readyではありません。Owner-only control socketを通じて稼働中Edgeをpollします。
`completion_acknowledged`がfalseまたはtimeoutなら、同じcandidateで同じactivateを再実行して
reportを取り直します。Durable Edge reportが`state=completed`かつ
`completion_acknowledged=true`になるまで通常runtimeを開始しません。

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

# Restoreは旧admin credentialと全operator/session tokenを意図的に除去しています。
# 新passphraseをargv、環境変数、log、incident reportへ渡さず、対話的にlocal ownershipを
# 再確立します。
sudo -u "$NODE_RUNTIME_USER" -g "$NODE_RUNTIME_GROUP" \
  iotkit-edge-nodectl --db "$CANDIDATE_DB" passphrase reset

# Timerを再開する前に既存backup policyのDBをrecovered DBへ切り替えます。
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
# 暗号化artifactを承認済みoff-host custody手順で保持した後、そのcopyを指定します。
# 設定済みdestination自体が承認済みoff-host mountならartifact自身を指定できます。
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

Matching completionとcompletion ACKはadmin credentialを作りません。対話的なlocal
passphrase resetは別の必須authority stepです。Normal startupの前に新しいownershipを
確立し、残っているoperator/session authorityを失効させます。Authenticated HTTP ingestを
使う場合は、reset後に通常のtyped operationを通じてdesired listener、TLS generation、
device authorityを再適用します。Restoreはapplied listener generationをclearするため、
明示的な再適用が成功するまでHTTP ingestは閉じたままです。Incidentをcloseする前に、
recovered Nodeから新しい暗号化backupを作成できることも確認します。失敗時にownership
fenceを迂回してはいけません。Incidentをcloseする前にfresh encrypted backupを作成し、
authenticateし、backup statusがhealthyであることを確認し、artifactをoff-hostに保持します。
ここでは`--replace-existing`が必須です。省略するとtimerは旧DBを指し続けます。このevidenceを
完了できない状態もrecovery failureです。

このbackup再設定とevidence blockは、暗号化backup candidateをproductionへ戻す本手順だから
適用します。Siteはscheduled backupを設定しない選択もでき、backup設定自体は引き続き任意です。
そのsiteは本encrypted-backup recovery手順を利用できず、別途acceptしたno-backup replacementの
loss boundaryに従います。

Final reportはNode clockの`backup_created_at`とEdge/Broker clockの
`broker_fenced_at`を個別の観測として残しますが、独立clockからdurationを導けないため
`recovery_window_ms=null`です。Snapshot boundary、replay数、new epochの
expected/current accepted cursor、Edgeだけにあるbackup後rangeも含みます。
`cursor_converged=true`はreplayがIoTKit Edge durable raw custodyへ到達した証拠です。
`remaining_gap_review_required`はtrueのままです。失われた旧機ではauthenticated snapshot後に
追加local tailをallocateしたか証明できないため、この明示的loss boundaryをincident reviewへ
記録します。

Repositoryは汎用Edge Node systemd unitを定義していません。上記unit名と
`NODE_RUNTIME_USER`/`NODE_RUNTIME_GROUP`は実deploymentのsupervisorに合わせ、serviceが
そのaccountで起動しcandidate DBへ書き込めることを確認してからevidenceを廃止します。
旧credentialと旧DBは再利用せずincident evidenceとして保持し、旧機を廃止します。

Backupなしhardware replacementはreadingもdedup claimもrestoreしません。Legacy snapshotや
plaintext DB copy、SQL編集、自作handoffはfallbackではありません。

### 8.2 IoTKit Edge restore

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

Device正本ledgerはEdge Nodeにあります。Console row編集を交換扱いにしません。終了は`iotkit-edge-nodectl device retire`、個体識別hardware交換は`device replace`を使います。Candidate profileと既存seriesを照合し、`system_id`を維持してhardwareだけを交換します。Forced/unchecked executionを通常手順にしません。暗号化backupと後続のpermit済みhandoffがなければ、replacementはreadingもdedup claimもrestoreしません。暗号化backup candidateも、別contractのpermitとcredential-generation checkが完了するまでfencedのままです。

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
