---
type: Concept
title: "IoTKit製品モデル"
description: "IoTKitの完全な製品範囲、component責務、権威の流れ、deployment選択、拡張境界を定義します。"
language: ja
translation_key: concepts.product-model
status: stable
revision: 3
---

# IoTKit製品モデル

状態: 現行の製品概念。

IoTKitは、工場・業務applicationより下に置く再利用可能なlayerです。センサー観測を収集し、障害をまたいで保全し、operatorが汎用的な意味を設定できるようにし、application固有のversioned messageとして出力します。製品、作業指示、工程、OEE、業務alarm、工場階層は所有しません。

## Component

| Component | 責務 | 所有しないもの |
|---|---|---|
| Device | 物理状態を測定または検出する | IoTKitの耐久性、業務上の意味 |
| Input Adapter | ベンダー・protocol固有の入力を汎用Edge Node ingest境界へ変換する | Storage、retry policy、custody、semantic rule、外部出力 |
| 認証付きHTTP ingest | process内Input Adapterを使わず、上限を持つcontract-native device Envelopeを受理する | Device firmware、ベンダー形式のdecode、業務上の意味 |
| IoTKit Edge Node | Deviceの近くでrecordを収集・正規化・耐久buffer・再送する | Edgeをまたぐ集約、業務logic |
| Internal MQTT Broker | Edge Nodeのrecordとcontrol messageを運ぶ | Applicationの耐久custody、データ権威 |
| IoTKit Edge | Raw custodyを受理し、Edge Node incarnationを発見・activationし、device/signal descriptor replicaを保存し、Edge scopeの表示・意味・出力設定、Console、耐久output deliveryを所有する | Edge Node側device identity・inventory・desired configurationの権威、工場・業務master、application workflow |
| Output Adapter | 汎用semantic Observationを一つの外部application用topicとpayloadへ決定的に変換する | Broker credential、retry scheduling、durable outbox state、semantic評価 |
| 外部application | IoTKit Observationを自身のdomainで使う | IoTKit raw custody、Edge Nodeのpurge権威 |

一つのIoTKit Edgeは複数のEdge Nodeを管理できます。IoTKit自身は工場という概念を必要としません。`edge_id`は一つのIoTKit Edge scopeを識別します。複数の`edge_id`をまたぐ集約は、任意のfleetまたはapplication layerの責務です。

## Dataと権威の流れ

1. ベンダー・protocol deviceはprocess内Input Adapterから報告し、contract-native deviceは認証付きHTTP Envelopeを送ります。両経路はEdge Node collectorへ合流します。
2. Edge Nodeは安定したIoTKit identityを解決し、Observationを保存します。activeなEdge Node incarnationであり、quarantineされておらず、publication admissionを通るrecordだけが、保存と同じtransactionでpublication stateを得てinternal Brokerへpublishされます。activation前のreadingはlocalに残り、後からcustody streamへreplayしません。quarantine中のreadingにはoutboxを作りません。後日解除する場合も、custody契約が定める耐久activation・publication admission gateを通ったものだけをenqueueできます。
3. IoTKit Edgeは検証済みraw recordと連続cursorを、選択された正本storage profileへ同一transactionで保存します。
4. IoTKit Edgeのapplication-level `accepted-through`だけがcustodyを移転し、対応するEdge Node recordをpurge可能にします。MQTT PUBACKだけでは移転しません。
5. IoTKit Edgeは、受理済みraw recordを変更せず、operatorが定義した汎用semantic ruleを評価します。
6. 選択されたOutput Adapterがexactな外部MQTT topicとpayloadを作ります。そのdelivery lifecycleはraw custody受理から独立しています。

## Deploymentの選択

Edge Nodeはlocal SQLiteをdurable bufferとして使います。IoTKit Edgeは次の正本storage profileから必ず一つを選びます。

- `embedded`: IoTKit Edge host上のSQLite。実測済みcapacity envelope内のstandalone・低concurrency deployment向け。
- `postgres`: PostgreSQL。別DB host、高いconcurrency、より大きい実測capacity envelopeが必要なdeployment向け。

両profileは同じ製品契約を実装し、検証済みcapacity envelope内でproduction利用できます。機能tierや信頼性tierではなく、deploymentの実測値で選択します。

IoTKit Edgeは両profileへdual-writeせず、空の別backendへ黙ってfallbackしません。SQLiteからPostgreSQLへの移行は、identity、cursor、outboxを検証する明示的なoffline migrationです。

BrokerはIoTKit Edgeと同じhostにも別hostにも配置できます。Hostname、network、certificate、credentialのprovisioningはdeployment責務です。Consoleは設定済みOutput Adapterを選択しますが、Broker infrastructureをprovisionしません。

## 拡張境界

- センサーまたはベンダーprotocolはInput Adapterと、必要なら再利用可能なdriverで追加する。
- または、対応可能なcontract-native deviceで認証付きHTTP ingest契約を直接実装する。
- 出力先applicationはOutput Adapterで追加する。
- ベンダー固有・application固有のidentifierを各Adapter内に留める。
- Wire fieldやrecord familyは、cross-language conformance fixtureを伴うversioned contract変更でだけ追加する。

BravePIとPinikietは最初に検証したintegrationです。どちらもIoTKitの汎用core modelを定義しません。

## Observationモデル（端末完結の再設計）

[#232](https://github.com/w-pinkietech/iotkit/issues/232) の再設計では、IoTKitはハードウェア1台につき1インスタンスで動き、端末の中でセンサー入力をObservationへ変換して標準のMQTT Brokerへ公開する。上の節が説明する中央のIoTKit Edgeと業務アプリケーション別のOutput Adapterは、この再設計の完了とともに削除される。本節はその後も残るObservationのモデルを、プロトコルに依存しない形で定める。MQTTへの写し方は[MQTT Output Adapter契約 v1](../contracts/mqtt-output-adapter-v1.md)にある。

Observationは、端末内の1つの処理pipelineが出力する1つの値である。pipelineはInput Adapterの出力を校正、二値化、ヒステリシス、デバウンス、累積カウントで変換し、pipeline-idで識別される。pipeline 1つにつき出力は1つで、複数のpipelineが同じ入力を参照できる。

kindは次の3つに固定する。production、alarm、Ganttなどの業務上の意味はIoTKitに入れず、受信側が対応付ける。

| kind | value | 単位 |
|---|---|---|
| measurement | ある時点で計測した数値 | pipeline設定に置き、payloadには含めない |
| accumulated-count | pipelineが算出した0以上の累積整数 | countとして扱う |
| state | 現在の状態を表すboolean | 持たない |

Observationは次の3つで連続性と順序を表す。

- **series**：同じpipeline出力の連続した世代。表示名や閾値などの調整、Broker設定、プロセス再起動、再接続では変わらない。kind、入力、trigger、単位といった構造の変更、明示的なリセット、定義のimport、状態の喪失で新しいseriesを始める。accumulated-countの新しいseriesは`value = 0`から始まり、その最初の値を即時公開する。
- **sequence**：series内で1から始まり、公開ごとに1増える整数。順序の判定と重複排除はこれで行う。
- **timestamp**：その出力を確定させた入力を端末が受信した実時刻。時計補正で逆行することがあるため順序には使わない。

Observationは端末に長期保存しない。端末は評価状態、現在の累積値またはstate、series、次のsequence、未送信のpublicationだけを保持し、履歴の保存は受信側のアプリケーションが所有する。
