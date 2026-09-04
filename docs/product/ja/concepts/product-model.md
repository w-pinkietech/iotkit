---
type: Concept
title: "IoTKit製品モデル"
description: "IoTKitの完全な製品範囲、component責務、権威の流れ、deployment選択、拡張境界を定義します。"
language: ja
translation_key: concepts.product-model
status: stable
revision: 5
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

Observationは次の4つで連続性、順序、時間を表す。

- **series**：同じpipeline出力の連続した世代。表示名や閾値などの調整、Broker設定、プロセス再起動、再接続では変わらない。kind、入力、trigger、単位といった構造の変更、明示的なリセット、定義のimport、状態の喪失で新しいseriesを始める。accumulated-countの新しいseriesは`value = 0`から始まり、その最初の値を即時公開する。
- **sequence**：series内で1から始まり、公開ごとに1増える整数。順序の判定と重複排除はこれで行う。
- **経過時間（uptime）**：端末の起動から、その出力を確定させた入力を受信するまでの経過時間。単調時計から取るため、同じ起動の中では2つのObservationの差が実際の経過時間に等しい。端末の再起動でゼロに戻るが、seriesは変えない。サイクル時間や欠測区間の長さはこれで測る。
- **実時刻（unix epoch）**：その入力を受信した実時刻。端末が自分の時計を信頼できる（NTP同期済みなど）ときだけ持ち、それ以外は「不明」。RTCのない端末の起動直後やNTPに届かない現場では不明が続く。カレンダーへの割り付けに使い、順序には使わない。

Observationは端末に長期保存しない。端末は評価状態、現在の累積値またはstate、series、次のsequence、未送信のpublicationだけを保持し、履歴の保存は受信側のアプリケーションが所有する。

## 設定の所有（端末完結の再設計）

再設計後の端末は、設定を「変更にプロセス再起動を要するもの」と「動かしたまま変えられるもの」に分けて所有する。前者はTOMLファイル、後者はSQLiteに置く。

| 所有 | 項目 | 反映 |
|---|---|---|
| TOML | edge-node-id、MQTT Brokerへの接続、DBのpath、statusのheartbeat間隔、pipeline定義の書き出し先、Console APIのbind、Input Adapterのインスタンス | プロセス再起動 |
| SQLite（Consoleから編集） | pipeline定義 | 即時 |
| SQLite（状態） | 評価状態、累積値またはstate、series、次のsequence、未送信のpublication、pipeline定義のハッシュ | — |

TOMLのテーブルは次のとおり。edge-node-idは端末を識別する安定したIDで必須とし、hostnameなどからの暗黙の既定値は持たない。edge-node-idとpipeline-idはどちらも[MQTT Output Adapter契約 v1](../contracts/mqtt-output-adapter-v1.md)の識別子の制約に従い、違反する値は起動エラーになる。

~~~toml
[edge_node]
id = "rpi1"                       # 必須。Broker namespace内で一意
db_path = "/var/lib/iotkit/iotkit.db"

[output.mqtt]
enabled = true
host = "mqtt.example"
port = 8883
password_file = "/run/secrets/iotkit-mqtt-password"
trust_mode = "bundle_only"        # system_roots または bundle_only
ca_file = "/etc/iotkit/broker-ca.pem"

[status]
heartbeat_interval = "60s"        # 5s〜1h。既定は60s

[pipelines]
export_path = "/var/lib/iotkit/pipelines.toml"  # 既定はDBと同じディレクトリ

[api]
enabled = true
bind = "0.0.0.0:8443"

[adapters.instances.<name>]
# Input Adapterのインスタンス。Input Adapter契約 v1を参照
~~~

`pipelines.export_path`は、pipeline定義の変更がコミットされるたびにDBから書き出すバックアップである。起動時には読まず、復元は明示的なimport操作で行う。

## pipeline定義（端末完結の再設計）

pipelineは、Input Adapterの出力1つをObservation 1つへ変換する処理の単位である。pipeline 1つにつき出力は1つで、複数のpipelineが同じ入力を参照できる。定義はSQLiteに保存し、Consoleと`nodectl pipeline`からtyped operation（`pipeline.create` / `update` / `delete` / `reset` / `import`）で編集する。項目は、変更が新しいseriesを始める**構造項目**と、seriesを継続する**調整項目**に分かれる。

| 項目 | 区分 | 内容 |
|---|---|---|
| `id` | 構造 | pipeline-id。識別子の制約に従う |
| `kind` | 構造 | `measurement` / `state` / `accumulated-count`。変更できない（削除して作り直す） |
| `input` | 構造 | `adapter`（Input Adapterのインスタンス名）、`subject`（任意。デバイスの識別。省略時はsubjectを問わない）、`measurement_key`、`channel_index`（任意）、`value_index`（既定0） |
| `trigger` | 構造 | `accumulated-count`にだけ必須。初期版は`on-transition`のみ |
| `unit` | 構造 | `measurement`にだけ必須。他のkindでは禁止 |
| `display_name` | 調整 | 表示名（128文字以内） |
| `calibration` | 調整 | `scale`（有限かつ0以外、既定1.0）、`offset`（有限、既定0.0） |
| `detector` | 調整 | `mode`（`high-active` / `low-active`）、`rise_threshold`、`fall_threshold`（`fall_threshold <= rise_threshold`）、`rise_debounce_ms`、`fall_debounce_ms`（0〜300,000）。`measurement`では禁止、他のkindでは必須 |

seriesの開始は次の規則で決める。

- 構造項目の正規化ハッシュを状態と一緒に保存し、起動時と定義変更時に定義のハッシュと比較する。不一致か状態行がなければ新しいseriesを始める。
- 明示的なリセット（Console、`nodectl pipeline reset <id>`）と`nodectl pipeline import <file>`は新しいseriesを始める。importは全定義を置き換え、fileにないpipelineは削除として扱う。
- `accumulated-count`の新しいseriesは、開始したトランザクションの中で`sequence = 1, value = 0`を公開する。
- pipelineを削除すると、そのtopicへ長さ0のpayloadをretain有効で公開し、Brokerが保持する最新値を消す。

入力1件の処理は「評価状態、現在値、次のsequence、outboxへの挿入」を1つのSQLiteトランザクションで書く。失敗した入力は破棄し、評価状態は失敗前のまま残す。pipelineごとの破棄件数、最後のエラー、時刻はメモリに保持してConsoleに表示する。累積値が2^53−1に達したpipelineは、それ以後の入力を破棄してエラーとして表示する。

定義の変更がコミットされるたびに、全定義を`pipelines.toml`へアトミックに書き出す。書き出しの失敗は定義を巻き戻さず、エラーとして表示して次の変更時に再試行する。
