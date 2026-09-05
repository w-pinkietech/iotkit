---
type: Runbook
title: "IoTKit Edgeの導入と復旧"
description: "導入、日常確認、証明書、account、backup、restore、移行、rollbackの完全な手順です。"
language: ja
translation_key: operations.installation-and-recovery
status: stable
revision: 30
---

# IoTKit Edgeの導入と復旧

> **移行中の注記（#232 子Issue 5）。** 本書が説明する中央の`iotkit-edge`、`scripts/bootstrap-edge.sh`、`deploy/compose.edge*.yaml`は#251 で削除した。暗号化backupとfenced restoreは#253 で削除し、復旧は下記「バックアップ」「復元」「端末の交換」の3点コピーに置き換えた。導入と日常確認の章の全面改訂は#250 の最終PR（5e）で行う。それまでは[試用profile](trial-profile.md)が唯一の実行可能な導入手順である。

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
- **状態 / 因果診断:** Sensor input → Input Adapter → Edge Node collector → internal Broker path → raw custody → semantic projection → external outputの順でevidenceを読みます。一つだけ表示される最初のcriticalまたはwarning actionから始めます。各stageは、確認できる最終成功時刻（不明なら**まだ確認できません**）、boundedな影響範囲、慎重な原因、次の確認を表示します。current durable/process factから再計算し、後続の一致する成功evidenceでactive stateを自動clearします。手動でincidentをdismissする操作はありません。`unknown`はcurrent evidence不足でありhealthyではありません。Edge Node heartbeatは90秒未満がfresh、90秒以上300秒未満がwarning、300秒以上がcriticalです。Retainedだけのheartbeatは過去detailであり、古いraw値も上流evidenceがhealthyなときだけadvisoryです。どちらもsensor停止の証明にはなりません。
- **機器管理 / 収集ノード:** discovery、登録、最終descriptor通信、診断対象generation。**登録済み**は認可stateで、online保証ではない。
- **ライブ:** 保存済みで有効な`cumulative_counter`計測ruleごとにcardを表示し、calibrationとruleを適用した最新の処理済み累積current valueと最終受信を、graphとは独立して示す。Graphは画面を開いてからの結果だけを示す。同じsignalに複数の有効な累積ruleがあれば別cardになり、numeric、boolean、alarmのrule cardとruleのないsignalは表示せず、有効な累積ruleが一つもない場合だけdashboard全体に設定案内を一つ表示する。累積graphは画面を開いてから全期間のbucketを伸ばし、各bucketの保存された終端stateをstaircaseで示す。Browser側は最大1,000bucketにboundedし、sessionが長くなったらbucket幅を広げて時間窓を巻き戻さない。開始後の処理済み値がまだなければ、過去のcurrent valueが表示できる場合でもgraphを空にして待機中と示し、cardからsensor詳細へ進める。Browserは画面がvisibleな間だけ5秒ごとに表示領域内の最大12件を更新する。一度取得した最終受信の経過時間とgraphの時間窓は、IoTKit Edgeの画面開始時刻を基準にBrowserの単調な経過時間で進めるため、一時的に再取得へ失敗しても進み続ける。未受信は明示し、5分以上新着がないruleは停止と断定せず**要確認**にする。Rawと過去dataは**受信履歴**で確認する。要確認時はsensor、Adapter、Edge Node、Broker、IoTKit Edge、semantic projectionの順に確認する。ライブとSensor詳細の**実信号プレビュー**のgraph横軸は有効なsemantic observed/event time、最終受信とcurrentの鮮度はIoTKit Edgeのraw receipt timeを使う。実信号プレビューは同じ直近60秒・1秒bucket graphで、bounded input history全体を評価してbooleanと累積のstateを維持する。累積ruleのresult cardには保存済みcurrent totalを示し、実信号プレビューには仮計算の直近60秒deltaを明記する。numeric、boolean、alarmと新規draftの上段graphは直近60秒のままだが、保存済み累積ruleを選択中は上段の受信値/設定結果graphと下段の保存済み累積staircaseが同じ画面を開いた時刻（表示開始）から現在までの横軸を使う。上段は重なり合う直近応答をbrowser内で継続し、表示点は全期間を代表する最大1,000点にまとめる。保存済みruleは別のstaircase graphで、選択した保存済みruleの表示開始後の累積を示す。永続化済みcurrentの変化を保存順に追加し、変化がなくても保存済み値を単調な画面時刻まで延長する。表示点は最大1,000点にboundedする。新規draftは保存後に累積開始と示す。Staircaseは1秒bucketの平均ではなく、保存順の保存済みcurrent stateを示す。正常に保存済みの点を取得できないsessionは表示開始後の保存済み変化なしと示し、履歴取得失敗は取得できないと示す。
- **受信履歴:** sensor・Edge Node・期間を一画面で絞り、選択中sensorと一致するbounded graphとrecent rawを確認。Raw graphの横軸はIoTKit Edge receipt日時を表示time zoneで示し、縦軸は値の範囲とsensor単位を示す。同条件CSVは汎用Observation exportで業務帳票ではない。
- **出力:** Active purpose-bound route。Pending publicationはBroker PUBACKまで削除しない。
- **システム:** Filesystem、DB size、raw/semantic/pending projection/outbox件数、最終backup、原因別診断。Console応答だけでEdge Node/Broker正常と判断しない。
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
2. Consoleが使える場合は因果status sectionを読み、最初のactionable criticalまたはwarning原因から確認する。`unknown`、登録済み、descriptor、過去raw dataをonlineの根拠にしない。IoTKit Edgeのsupervised fatal taskが終了した場合はConsoleを意図的に利用できないため、hostのservice managerとservice logを使う。
3. DNS/routeとcertificateを確認する。
4. Mosquitto authとexact-topic ACLを確認する。`iotkit/v1/edge-nodes/{edge_node_id}/status`について、Edge NodeのwriteとIoTKit Edgeのread permissionも含める。
5. Edge Node `accepted-through`を確認する。未受理recordはEdge Nodeに残す。
6. IoTKit Edgeのsemantic projection queueとoutput queueを確認する。同じObservation identityでretryする。
7. 復旧後、raw cursor、pending semantic projection、pending outputの収束前に保持dataを削除しない。

## 6. Edge Node登録の復旧

- **未登録:** Descriptorは見えているがrecord batchを保存・ackしない。
- **登録処理中:** Durable state。同じrequestをmatching result commitまでretryするため、各service restart後に再登録しない。
- **復旧確認待ち:** Descriptor、stored generation、activation resultがconflict。両DBを保全しidentity/restore履歴を調べ、row削除、別identity発行、state table直接編集をしない。
- Publication sequenceを一度でもallocateしたstreamへfresh activationを行わない。V1はstandalone outbox adopt、reactivation、Edge間transfer、identity reuseをしない。
- 登録はMQTT credentialをcreate/rotate/revokeせずBroker enrollmentを置換しない。
- 登録時にlocal reading boundaryを固定し、旧prefixをbounded background cleanupする。Normal processingから見えなくしますが、SQLite page、backup、mediaからのforensic eraseは保証しない。

## 7. バックアップ

端末の状態は次の3点である。

- TOML（`edge-node-id`、`[output.mqtt]`、`[status]`、Input Adapterのインスタンス）
- SQLiteファイル（pipeline定義、評価状態、累積値またはstate、series / sequence、PUBACK前のoutbox）
- `pipelines.toml`（DBから書き出したpipeline定義のバックアップ。起動時には読まない）

停止した端末からこの3点をコピーして保管する。暗号化backup、snapshot CLI、fenced restoreは提供しない。

## 8. 復元

停止した端末、または交換したハードウェアへ、保管した3点を同じ相対配置でコピーする。既存のSQLiteは作り直さず、コピーしたファイルを使う。pipeline定義をファイルから入れ直すときは`nodectl pipeline import`を使う。import後は全pipelineが新しいseriesで始まる。

端末はNTP同期を必須とする。consumerはheartbeatの`timestamp`と受信時刻の差から時計のずれを検出できる。

## 9. 端末の交換

ハードウェアを交換するときは、旧端末を停止し、3点を新端末へコピーして起動する。累積値とseriesの連続は、コピーしたSQLiteが届いた範囲でのみ保たれる。コピーできなかった場合は新しいseriesになる。暗号化backupや復旧権限による再稼働は使わない。

## 10. SQLiteからPostgreSQLへのoffline移行

IoTKit Edgeを停止し、未ack dataはBroker/Edge Nodeに保持させます。Dual-write、自動fallbackをせず、IoTKit tableのない空DBへ移行します。Running Edgeがdeployment lockを持つため停止忘れでは開始しません。Protected consistent snapshotから全tableをcopyします。Connection情報はmode `0600` JSONへ置き、DSN/passwordをargvへ渡しません。

Offline profile移行が受理するSQLite sourceは現行schema v12だけです。v9、v10、v11のsourceでは、先にそのSQLite DBを現行IoTKit Edgeで起動してtransactionalなv12 upgradeの完了を待ち、停止してから移行します。このupgradeは導出raw series keyを保持し、latest Edge Node運用status row、current epoch raw receipt、active rule/route診断lookupのindexを追加します。rowのbackfill、保持済みhistoryのcopy、heartbeat historyの作成は行いませんが、index構築は保持済みraw、Observation、outbox historyを読むため、history量に応じた時間と一時的なdatabase/WAL容量を確保します。Offline copyは保存済みraw-series value、v11 raw-preview index、v12 latest-status table、v12 diagnostic indexを保持・検証し、targetで別の値を導出しません。

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

1. 停止した端末からTOML、SQLiteファイル、`pipelines.toml`の3点をコピーして保管する。
2. Git commit、Compose設定、image IDを記録し、credential/keyをGitへ入れない。
3. 新versionを取得・buildし、Brokerを動かしたままEdgeだけ停止する。
4. Schema v12では更新前の3点を保持し、導出raw-series-key backfill、raw-preview index、latest-status table、current epoch raw receiptとactive rule/route診断index、SQLite WAL増加に必要な時間と十分な空き容量を確保する。必要な有効canonical measurement envelopeをstartup migrationでbackfillするが、status tableはheartbeat historyをbackfillせず、診断indexはhistory rowをcopyしない。一方でindex構築は保持済みraw、Observation、outbox historyを読む。Migrationはschema v12を完全にcommitするかrollbackする。
5. 新Edgeを起動する。Schema migrationはstartup transaction。
6. HTTPS login、diagnostics、cursor再収束、pending semantic projection、pending outbox、history graph、CSVを確認する。Restart recovery完了として扱う前にqueueをdrainする。
7. 失敗時はEdgeを停止する。旧binaryでmigration済みDBを開かず、旧commit/imageへ戻し、更新前の3点を§8と同じ手順で戻す。Identity/credentialを作り直さない。

これはmanual updateです。Migration後にimageだけ戻すのはrollbackではなく、対応する更新前DBも戻します。
