---
type: Architecture
title: "IoTKitシステムArchitecture"
description: "実行構成、dataとcustodyの流れ、code配置、concurrency、互換性ruleを定義します。"
language: ja
translation_key: architecture.system-overview
status: stable
revision: 17
---

# Architecture

IoTKitは、Rust製`iotkit-edge-node`とoperator CLIの`iotkit-edge-nodectl`、別processのRust製`iotkit-edge`、標準MQTT Brokerで構成します。Edge Nodeは単一SQLite DBを使い、Raspberry Pi上でsystemdにより無人運転します。IoTKit Edgeは導入時に`embedded`（SQLite）または`postgres`（PostgreSQL）の正本profileを一つ選びます。

この文書は、初見の開発者が10分で配置を理解するための地図であり、code配置の正本です。製品境界は[製品モデル](../concepts/product-model.md)、読む順序は[日本語ドキュメント](../index.md)に従います。

## 対象読者

| 読者 | 主に触る場所 | 良い状態 |
|---|---|---|
| 導入・運用担当者 | 導入手順、CLI、API、Console、error | 何をどこへ入れ、障害時に次に何を確認するかが分かる |
| 自作device開発者 | Ingest wire contract | Rustを読まず、少数commandと安定reason codeで接続できる |
| Adapter開発者 | host API、testkit、driver、既存Adapter | Storageやledgerを知らず、新しいsensor familyを追加できる |
| Core contributor | `edge-node/core/*`、binary、test | Crate責務と依存方向が機械検査される |
| Custody実装者 | Edge Node custody contract | Record family、ack、cursorがversion管理される |
| Application integrator | Output Adapter contract | Raw custodyへ依存せず、application向けtopic/payloadを受け取れる |

## 実行配置

```text
[1] Device
  -> [2] IoTKit Edge Node
  -> standard MQTT Broker
  -> [3] IoTKit Edge
  -> Output Adapter
  -> external Broker / application

[4] Fleet layer: 複数edge_idをまたぐ任意の上位層
```

このrepositoryは[2]と[3]を提供します。[1]はhardwareと公開ingest contract、[4]は外部systemです。IoTKit EdgeはEdge NodeのRust packageやDBを読みません。共有するのはversioned wire contract、fixture、schema semanticsだけです。

基準deploymentは、Edge NodeをRaspberry Piでnative実行し、BrokerとIoTKit EdgeをLinux host上のDockerで動かします。ただしco-locationは要件ではなく、認証付きMQTT/TLS contractを守れば別hostに分離できます。`scripts/bootstrap-edge.sh`は非secret bindingとoperator提供のTLS materialから、anonymous無効Broker、Edge Node別ACL、owner-only credential、handoffを生成します。Certificate発行、DNS、firewall、VPN、Edge Node変更は行いません。

IoTKit Edgeの`embedded`と`postgres`は同じ製品契約を満たします。SQLite fileはIoTKit Edge processと同じhostのlocal storageへ置きます。一つのEdgeが両DBへdual-writeしたり、障害時に空の別backendへfallbackしたりしません。Profileは導入時に固定し、SQLiteからPostgreSQLへの変更は停止、整合backup、全identity・cursor・outbox検証を伴うoffline operationです。[導入と復旧](../operations/installation-and-recovery.md)と[容量runbook](../operations/storage-capacity.md)に従います。

Raw custodyではcanonical record JSONとrecord hashを正本として保持します。Schema v11は、有効なmeasurement envelopeだけにnullableの導出`series_key`を追加します。設定のreal-signal previewはsignal referenceを解決し、両profileでindexされた`(edge_node_id, series_key, received_at DESC, ledger_epoch DESC, pub_seq DESC)`順にbounded raw tailを読みます。この読出しで保持済みraw historyのJSON field抽出やsortは行いません。SQLiteはfull keyをindexします。PostgreSQLは固定長の`md5(series_key)` discriminatorをindexし、完全なkeyを再照合します。そのため長い保持keyでもraw-preview B-tree tupleをoverflowせず、digest collisionで別signalのrecordを選びません。

## Data flow

```text
Adapter / HTTPS device
  -> collector
  -> SQLite readings + publication_log（同一transaction）
  -> MQTT publisher
  -> Broker（PUBACKはtransport確認だけ）
  -> IoTKit Edge raw store + cursor（同一transaction）
  -> accepted-through
  -> Edge Node purge eligibility
```

Input Adapterは`Envelope`を共有ingest clientへ渡します。認証付きHTTP bindingも同じcollectorへ入ります。送信者が指定する`Envelope.source`ではなく、受信側が作るprincipalが権限を決めます。`AdapterEvent`と`AdapterCommand`はsupervisionとlegacy southbound用の凍結語彙であり、新しいingest pathではありません。

Edge Nodeはledgerとregistryから1 MiB以下のschema-2 complete descriptor snapshotを作り、Edge Node固有`descriptors` topicへQoS 1 retainedで送ります。明示的に永続化した任意`model_id`だけを含め、Adapter instance、物理locator、hardware/provider identifierは越境させません。Descriptor失敗はcustody processingを止めません。

Broker enrollmentはtransport接続許可だけで、activationではありません。Activation前のObservationはlocalに保存し、publication sequenceを与えず送信しません。IoTKit EdgeがdescriptorからEdge Nodeを発見し、admin typed operationでexact ledger epochのrequestを耐久enqueueします。Edge Nodeが検証・耐久適用して境界を固定し、その後のingestだけをoutboxへ入れます。IoTKit Edgeはmatching resultをcommitしてactiveにした後だけ、activation検査、raw保存、cursor更新を同一transactionで行います。

`mqtt_publish_task`が現行production exit bindingです。Broker PUBACKはtransport receiptであり、custody移転ではありません。IoTKit Edgeがraw recordとcursorをcommitし、対応する`accepted-through`をpublishするまでEdge Node outboxを保持します。

### Semanticとapplication export

IoTKit Edgeは100 msごとの独立したconvergence loopで、`semantic_projection_queue`にある耐久済みrule-record workだけを汎用semantic Observationへprojectし、versioned application eventを耐久outboxへenqueueします。Raw受理はmatchingするactive ruleごとにqueue rowを同一transactionで追加し、そのrule revisionとcalibration revisionをsnapshotします。Candidate選択はqueueを順序付けしてからimmutable raw recordとsnapshotをjoinするため、保持済みraw historyやreceipt historyを再scanしません。Queue rowはraw record件数やreceipt lagではなく、pendingのrule-record pair一件です。

一つのprojection transactionはObservation、routeのoutbox row、durable receipt、runtime stateを作成してからqueue rowを削除します。Poison inputではfailureとterminal receiptを書いてから削除します。それ以外のfailureは全てrollbackし、queue rowをretry可能なまま残します。Receiptは引き続きdurable idempotency authorityです。Pending counter resetのboundaryは、境界までのworkがdrainするまで同じruleの後続queue rowをfenceします。各tickは最大16 itemをadmitし、20 ms後には次のitemをadmitしません。一つのin-flight transactionはこのwall-time budgetを超え得ます。Cancellationを確認してitem間でyieldするため、recovery中もlogin、diagnostics、custody workを進められます。MQTT QoS 1 publishがPUBACKを受けた時だけoutbox rowをpublishedにします。Publish failureまたは15秒timeoutではoutbox rowを後のtick向けにpendingのまま残します。transaction内のprojectionまたはenqueue failureは変更をrollbackし、queue rowをrestart後もretry可能なまま残します。Criticalなstorageまたはprojection task failureはpayloadやcredentialをlogへ出さず、loopを黙って継続せずserviceをcancelします。

Raw batch transactionと`accepted-through`はsemantic projectionやapplication outputを待ちません。Application停止がEdge Node custodyを拘束しないためです。Semantic mappingとMQTT routeはfuture-onlyで、過去dataを暗黙にbackfillしません。

Output Adapterは汎用Observationとroute設定からexact MQTT publicationを作る決定的in-process transformerです。Broker接続、credential、retry、durable outbox、business masterは所有しません。`pinikiet.mqtt.v1`は最初の実装ですが、特権core pathではありません。

## Custody loop

1. **Ingest:** active incarnationでquarantineされておらずpublication admissionを通るObservationは、readingとoutbox rowを同じSQLite transactionで保存する。Activation前はlocalだけ、quarantine中はoutboxなし。
2. **Publish:** publisherはDB lock内でbatchを作り、lockを外してMQTT送信する。Network round-trip中にDB lockを保持しない。
3. **Ackとcursor:** IoTKit Edgeがrecordとcursorを正本storeへcommitしてから、exact epoch・publication・batch boundの`accepted-through`を返す。
4. **Purgeと劣化:** ack済み、custody policy対象外、未解決quarantine、未ack originalの順で扱う。未ack original削除は最後の明示data-loss classであり、`custody_lost` auditとgap annotationが必須。

上流停止時はcursorが止まりbacklogが増えます。Capacityを超えると新規writeは明示失敗し、保存済みdataを黙って捨てません。現行契約は持続的圧力に対して「安全だが滑らかではない」です。`commissioning_smoke`は物理sensorを装わず通常custody pathを検証します。

## Control plane

Edge Nodeはprivate address client向けHTTPS APIを持ちます。State変更は`edge-node/core/ops`のtyped operation catalogを通し、permission tier、dry-run、無条件auditを持ちます。新しいmutation surfaceは必ずR14 descriptorとし、API/UI/AI/CLIから新しいSQL mutation pathを作りません。

初期ownershipとadmin recoveryはlocal-root maintenanceです。Unownedまたはrecovery中のboxはcontrol API/UIをbindしません。Passphrase resetはcredentialを置換し、operator tokenとsessionをすべて失効します。未認証network setup routeはありません。

IoTKit Edge側の変更も`edge/src/application/`のtyped operationを通します。HTTP、HTML、CLIはthin adapterであり、SQLへ直接writeしません。

任意機能のEdge Node recovery filesystem operationでは、local rootとeffective
ownerを一つのtrusted principalとして扱います。Config parentと保護対象の
file/directoryはgroup/otherの全accessを拒否します。Supported configure、
destination verification/probe、publication、retentionは、config隣接のstableな
owner-only nonblocking lockをoperation全体で一つ保持し、二つ目のsupported callは
`operation_busy`で失敗します。このlockはproduct code間の調整であり、同じ
effective UIDですでに動くhostile codeから保護するsecurity boundaryではありません。
そのcodeはfilesystem namespace保護の対象外であり、host containmentが必要です。

## 現行実装

V1候補は、BravePI温度・接点入力、汎用Input Adapter/driver、複数Edge Node、標準Broker、一つのIoTKit Edge、SQLite/PostgreSQL raw store、application-level `accepted-through`、future-only semantic projection、durable Output Adapter outbox、認証付きConsole、保存済みで有効な`cumulative_counter`計測ruleごとの処理済み累積結果を示すbounded live dashboard（numeric、boolean、alarmのrule cardとruleのないsignalは省略し、有効な累積ruleが一つもない場合だけdashboard全体に設定案内を一つ表示）、範囲付きhistory graph、汎用CSVを提供します。

BravePIはBLE、既存iOS applicationによるpairing、transmitter管理を所有し、IoTKitはBravePI Mainboard UART streamから始まります。Broker host certificate componentはbundle検証・atomic install、`lego` ACME更新、MQTT/HTTPS probe、失敗時rollbackを提供します。短命credential enrollment/rotationとarchive gap復元後のretained replayはv1後のhardeningです。

## Crate map

| Crate / path | 責務 |
|---|---|
| `edge-node/core/types` | Protocol非依存domain type。leaf |
| `edge-node/ingest/contract` (`iotkit-ingest-contract`) | `Envelope`、`Ack`、reason codeのwire contract。Runtime依存はserdeだけ |
| `edge-node/core/storage` | SQLite handleとcross-crate migration harness |
| `edge-node/core/supervision` | 凍結済み`AdapterEvent` / `AdapterCommand`語彙 |
| `edge-node/core/engine` | `AdapterEvent`からdevice stateをprojectするin-memory engine |
| `edge-node/core/ledger` | Device ledger、`system_id`、series identity、sighting、epoch、audit |
| `edge-node/core/timeseries` | Reading、staging、event time、query |
| `edge-node/core/publish` | Activation admission、publication outbox、target、cursor |
| `edge-node/core/collector` | Dedup、series解決、quarantine、activation admission、same-transaction enqueue |
| `edge-node/core/registry` | Standard catalogとdeployment overrideのmeasurement registry |
| `edge-node/core/ops` | Typed operation、permission、auth、dispatch、audit |
| `edge-node/core/recovery` (`iotkit-core-recovery`) | Optional Edge Node backup/recoveryのdurable state、完全migration set、read-only startup fence probe、recovery modelのredaction境界 |
| `edge-node/ingest/client` (`iotkit-ingest-client`) | Adapterが使うingest contract client |
| `edge-node/input/host-api` (`iotkit-input-adapter-host-api`) | Supervision非依存の公式Adapter composition API |
| `edge-node/input/testkit` (`iotkit-input-adapter-testkit`) | Conformance assertionとreference Adapter |
| `edge-node/ingest/http` (`iotkit-ingest-http`) | Ingest listener、TLS、上限制御。Control APIではない |
| `edge-node/input/runtimes/polling` (`iotkit-polling-adapter-runtime`) | I2C polling engine。Ingest・mapping・supervision非依存 |
| `edge-node/input/hardware/transports/rpi` (`rpi4b-transport`) | Raw I2C/GPIO/SPI/PWM I/O。歴史的名前はPi 4B限定を意味しない |
| `edge-node/input/hardware/sensor-drivers` (`iotkit-sensor-drivers`) | Sensor IC定数、identity metadata、datasheet変換 |
| `edge-node/adapters/bravepi-mainboard/codec` (`bravepi-codec`) | BravePI frame codec |
| `edge-node/adapters/bravepi-mainboard` (`bravepi-mainboard-adapter`) | BravePI transport + codec + driverからEnvelopeへの変換 |
| `edge-node/adapters/rpi-local` (`rpi-local-adapter`) | Direct Linux I2C Adapter、対応model catalog、measurement projection |
| `edge-node/adapters/trial-sample` (`trial-sample-adapter`) | 試用profileで明示設定するlocal限定sample Input Adapter。現場有効化には`IOTKIT_ENABLE_TRIAL_SAMPLE=1`が必要。inventoryのmodel idはhardwareではない`trial-sample-illuminance`（連続・三角波）と`trial-sample-contact`（状態・矩形波）。通常のadapter hostとcustody経路へ2系列の測定値を渡す |
| `edge-node/tools/bravepi-poc` (`bravepi-poc`) | BravePI実機PoC用tool。非配布 |
| `edge-node/apps/node` (`iotkit-edge-node`) | Edge Node composition root binary |
| `edge-node/apps/nodectl` (`iotkit-edge-nodectl`) | Edge Node operator CLI |
| `edge/` (`iotkit-edge`) | MQTT custody、storage、semantic、Output Adapter、認証付きConsole、backup、diagnostics、operator CLIのRust composition root |
| `edge/custody-contract` (`iotkit-edge-custody-contract`) | Edge Node MQTTのdescriptor、activation、record batch、custody ackを厳格検証するversioned wire contractのleaf Rust表現 |
| `edge/output-adapters/api` (`iotkit-output-adapter-api`) | ObservationからMQTTへの決定的変換とprovider非依存profile policyのleaf Rust API |
| `edge/output-adapters/testkit` (`iotkit-output-adapter-testkit`) | Descriptor、config、publication、決定性のdev-only共通conformance assertion |
| `edge/output-adapters/example` (`iotkit-output-adapter-example`) | Production registryへ登録しないvendor-neutralなcompile-tested作者例 |
| `edge/output-adapters/generic-mqtt-json-v1` | 組み込みIoTKit汎用Observation JSON変換 |
| `edge/output-adapters/pinikiet-mqtt-v1` | 組み込みPinikiet MQTT変換とprofile policy |
| `edge/frontend/src/` | SSR ConsoleのTypeScript browser behavior |
| `edge/openapi/edge-console-v1.yaml` | TypeScript生成元のbrowser JSON contract |
| `testdata/egress/v1/`, `v2/` | Rust conformance testがdecodeするwire fixture |

## 機械検査するlayer rule

1. Adapterは`edge-node/core/engine`へ依存しない。
2. Adapterは`iotkit-ingest-client`以外からdata planeへ到達しない。
3. `iotkit-ingest-contract`のruntime依存はserdeだけ。
4. `edge-node/core/types`と`edge-node/core/storage`はleaf。`edge-node/core/*`からAdapter・binaryへ上向き依存しない。
5. 新workspace crateは`scripts/check-layers`とこのmapへ分類する。
6. `iotkit-ingest-client`のworkspace依存はcollectorとcontractだけ。
7. `edge-node/core/supervision`のnon-dev dependent setを固定する。
8. Ingest HTTPとcontrol APIを分離する。
9. IoTKit Edgeはwire contractを使い、Edge Node内部packageやDBを読まない。
10. Input Adapterからsupervisionへのtransitive到達も検査する。

意図的な例外として、collector所有portをregistryが実装する`edge-node/core/registry -> edge-node/core/collector`、in-process bindingのためのingest clientからcollectorへの依存、BravePI subcrateのco-location、cross-crate joinを持つretention/record materializationのbinary配置、identity transactionを共有する単一ledger aggregateがあります。理由を理解せず「修正」しません。

## Code配置rule

先に接続境界を選び、その後でcrateを選びます。

1. Deviceがversioned Envelope/Ack contractを直接送れる場合は
   `edge-node/ingest/http`へ接続し、Rust Input Adapterは作りません。
2. 既存のdirect-I2C transport、polling lifecycle、identity、config形状に
   合う新しいsensor ICは`edge-node/adapters/rpi-local`へ追加します。
3. Discovery、wire protocol、security、lifecycle、identityが異なる場合は
   `edge-node/adapters/`配下に新しいfamilyを作り、host contractを実装します。

| 追加するもの | 配置先 |
|---|---|
| Acquisition間で再利用するdatasheet変換 | `edge-node/input/hardware/sensor-drivers` (`iotkit-sensor-drivers`) |
| 同じI2C transport・polling・identity・config形状の新IC | `edge-node/adapters/rpi-local`のtyped model catalog |
| Discovery、wire、security、lifecycle、identityが異なるdevice family | `edge-node/adapters/`配下の新しいsibling crate |
| Ingest wire変更 | `edge-node/ingest/contract`とconformance test |
| Edge Node state変更operation | `edge-node/core/ops` catalogとdispatch |
| IoTKit Edge state変更operation | `edge/src/application/`のtyped operation |
| Table / column | 所有crateのmigration slice |
| Control-plane route | `edge-node/apps/node/src/api/`のthin layer、logicは所有`edge-node/core/*` |
| Measurement HTTP binding | `edge-node/ingest/http` (`iotkit-ingest-http`) |
| CLI command | `iotkit-edge-nodectl`から`edge-node/core/*`を呼ぶ |
| IoTKit Edge acceptance/query/semantic/export | `edge/` |
| Raw bus/pin access | `edge-node/input/hardware/transports/rpi` (`rpi4b-transport`) |
| Tableを持つ、両binaryで必要、複数責務を持つNode module | `edge-node/core/<name>`へ昇格 |

## 主要data structure

- Seriesは`UNIQUE(system_id, measurement_key, channel_index, variant)`。`system_id`はledgerだけが発行する不変UUIDv7、`hardware_id`は交換可能な物理address、`user_label`は表示だけです。Hardware交換後も同じ`system_id`で履歴を継続します。
- `readings.seq`はbox内部の挿入順、`publication_log.pub_seq`は外部配送順です。Quarantine readingは`seq`を持ちますが、解除まで`pub_seq`を持ちません。
- 外部record identityは`(epoch, pub_seq)`です。Slice-1のfenced-candidate
  restoreはcandidateをcollect/publishできないため、epochをmintもactivateもしません。
  productionでのsame-ID box swap、新epochの発行、古いconsumer cursorの扱いは、
  後続のpermit/reconciliation contractで定義するfuture behaviorであり、出荷済みrestore
  operationではありません。

## Concurrency

Edge Node process全体で`Arc<Mutex<Connection>>`を一つ使い、全subsystemが`spawn_blocking`経由で直列化します。SQLiteはWAL + `synchronous=FULL`です。Publisherはnetwork round-trip中にDB lockを保持しません。Custody-critical retention purgeは一つのImmediate transactionで、失敗してもpurge済みdataをrollbackさせてはいけないhousekeepingはcommit後の別best-effort transactionで行います。

## Migrationと互換性

`edge-node/core/storage/migrate.rs`はcrateごとに分割されたversion空間を扱うため、最大versionではなく適用済みsetとの差分でmigrationを実行します。新しいon-disk schemaを古いbinaryで開く場合は`SchemaVersionAhead`で拒否し、downgradeで利用者dataを壊しません。

## 次に読む文書

- [日本語ドキュメント](../index.md)
- [Edge Node保管責任契約](../contracts/edge-node-custody-v1.md)
- [Output Adapter契約](../contracts/output-adapter-v1.md)
