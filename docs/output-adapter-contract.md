# IoTKit Output Adapter contract v1

Status: Implemented 2026-07-19

## 1. Purpose

Output Adapterは、IoTKit Siteで確定した汎用的な意味データを、外部application固有のMQTT契約へ
変換する境界である。YokaKitは最初の実装だが、汎用契約はYokaKitのtopic、用途名、payload fieldを
知らない。

```text
raw record
  -> semantic rule
  -> generic Output Observation
  -> Output Adapter + versioned route config
  -> exact MQTT publication
  -> durable Site outbox
  -> external Broker
```

Sensor Adapterは物理機器からIoTKitへ値を取り込む。Output AdapterはIoTKitから外部applicationへ
意味データを出す。この二つを同じadapter lifecycleや設定形式へ統合しない。

## 2. Contract boundary

Output Adapterが所有するもの:

- stableなAdapter IDと設定schema version
- 対応する汎用Observation種別と、外部側modeの組み合わせ
- route設定の構文・値・互換性検証
- generic Output ObservationからMQTT topic/payloadへの決定的変換
- QoS、retainを含む1件のMQTT publication

Output Adapterが所有しないもの:

- semantic ruleの評価、しきい値、debounce、累積
- Broker endpoint、TLS、certificate、credential、client ID
- MQTT接続、publish、PUBACK、retry、backoff
- SQLite、outbox、配送済み状態、監査
- Siteのaccount、role、Console
- 外部applicationのbusiness master、工程、生産実績、OEE

AdapterはSite process内で動く純粋変換である。storage、clock、network、environment、secretへ
アクセスしてはならない。同じroute設定とObservationからは、byte単位で同じpublicationを返す。

## 3. Adapter identity and capabilities

各Adapterは`Descriptor`を返す。

```go
type Descriptor struct {
    ID                  string
    DisplayName         string
    ConfigSchemaVersion int
    Modes               []Mode
}

type Mode struct {
    Key         string
    DisplayName string
    Accepts     []ObservationKind
}
```

- `ID`は小文字ASCIIのstable IDとし、外部application、transport、major contractを識別する。
  初期実装は`iotkit.mqtt-json.v1`と`yokakit.mqtt.v1`。
- `ConfigSchemaVersion`はroute設定JSONのexact versionである。
- `Mode.Key`は外部application側の用途であり、汎用Observation種別ではない。
- `Accepts`はそのmodeへ変換できる汎用Observation種別の閉じた集合である。
- 同じAdapter内でmode keyを重複させない。

DescriptorはConsoleの入力候補に使えるが、表示用metadataである。最終的な保存可否は
`ValidateConfig`が同じ規則で検証する。

## 4. Generic Output Observation

Adapterの入力は次のprovider-neutralな構造である。

```go
type Observation struct {
    ObservationID string
    SeriesID      string
    Sequence      int64
    ObservedAt    int64
    Kind          ObservationKind
    Value         json.RawMessage
    Reading       *float64
}
```

v1の`ObservationKind`は次の4種類である。

| Kind | Value | Meaning |
|---|---|---|
| `numeric` | finite JSON number | 補正または変換済みの数値 |
| `boolean` | JSON boolean | 汎用ON/OFF状態 |
| `cumulative_value` | non-negative JSON integer | 起点以降の累積値 |
| `alarm` | JSON boolean | `true`=発報、`false`=解除 |

`production`、`onoff`、`gantt_chart`は外部applicationの用途であり、この汎用kindへ追加しない。
IoTKit内部の実装名が`cumulative_counter`でも、Output Adapter境界では利用者の合意どおり
`cumulative_value`とする。

`observation_id`と`series_id`は小文字canonical UUID、`sequence`は1以上の単調増加値、
`observed_at`はUnix epoch millisecondsである。Adapterはidentity、時刻、値を作り直さない。
`reading`はalarm判断時の任意の有限数値であり、存在しない場合に推測しない。

Edgeの`ledger_epoch`、`pub_seq`、Siteのraw row ID、custody cursorは外部applicationの入力ではないため
この境界へ渡さない。

## 5. Versioned route configuration

設定はUTF-8 JSON objectであり、`schema_version`を必須とする。Adapterは未知field、未知version、
複数JSON値、末尾garbageを拒否する。暗黙のdefaultやfield推測で古い設定を読み替えない。

```go
type Adapter interface {
    Descriptor() Descriptor
    ValidateConfig(json.RawMessage, ObservationKind) error
    Transform(json.RawMessage, Observation) (MQTTPublication, error)
}
```

`ValidateConfig`は次を一度に検証する。

- JSONがAdapterの設定schemaに適合する
- 指定されたmodeが存在する
- source Observation kindとmodeが互換である
- topicへ使う外部identityが安全なclosed syntaxである
- 外部contract固有の値域を満たす

設定変更は新しいroute revisionとしてfuture-onlyに適用する。過去Observationを暗黙にbackfillしない。
routeを停止しても、すでにdurable outboxへ入ったpublicationを黙って削除しない。

route設定へBroker credential、CA、private key、tokenを保存しない。`config`はviewerにも表示可能な
非秘密の変換設定だけで構成する。接続profileとsecretはdeployment設定が所有する。

## 6. Registry and route persistence

Siteは組み込みAdapterをregistryへ登録する。v1のregistryはcompile-timeであり、runtime plugin discoveryを
行わない。重複Adapter ID、invalid descriptor、未知Adapter IDはroute作成時に拒否する。

generic routeは論理的に次を保存する。

```text
route_id
rule_id
adapter_id
config_schema_version
config_json
start_after_observation_row_id
active
created_at
```

`config_schema_version`は選択されたAdapter descriptorと一致しなければならない。既存の
YokaKit専用routeはmigrationで`adapter_id=yokakit.mqtt.v1`とversion付きconfig JSONへ変換する。
outboxの`route_id`は維持し、配送待ちmessageを失わない。

Console/APIが利用する汎用面は次である。

- `GET /api/v1/output-adapters`: 利用可能なdescriptorとmode
- `GET /api/v1/output-routes`: route、非秘密config、配送件数
- `POST /api/v1/output-routes`: rule、Adapter ID、version付きconfigからfuture-only routeを作成

既存のYokaKit専用APIは移行用の互換入口であり、内部では同じgeneric route operationを使う。
API handlerやConsole handlerからSQLへ直接書き込まない。

## 7. Transformation and errors

`Transform`はObservationと設定を再検証し、成功時にexactly one `MQTTPublication`を返す。
一つのObservationを複数topicへ出す場合はrouteを複数作る。v1ではAdapter内fan-outを提供しない。

エラー分類:

- `ErrInvalidDescriptor`: Adapter実装または登録metadataの不備
- `ErrInvalidConfiguration`: route設定の決定的な不備
- `ErrInvalidObservation`: generic Observationの決定的な不備
- `ErrUnsupportedObservation`: source kindと外部modeの非互換
- `ErrInvalidPublication`: 生成されたMQTT publicationの不備

純粋変換には一時的network errorという分類は存在しない。変換失敗時はoutboxへ不正なmessageを入れず、
routeを要対応として可視化する。元のsemantic Observationを削除、配送済み扱い、別modeへ推測変換
してはならない。

## 8. MQTT publication

```go
type MQTTPublication struct {
    Topic   string
    QoS     byte
    Retain  bool
    Payload json.RawMessage
}
```

v1では次を必須とする。

- topicは空でないexact UTF-8 topic
- topicにNUL、`+`、`#`を含めない
- QoSはexact `1`
- payloadはvalid JSON
- Adapterはpublishしない
- delivery layerがpublicationをSQLite outboxへdurable保存してからBrokerへ送る
- PUBACKまでは同一topic/payloadをretryする

`retain`は外部contractが決める。通常のObservationは原則`false`、source status等の別契約では
`true`を選べる。

組み込みAdapterの共有fixtureは`testdata/output/v1/`に置く。fixtureはAdapter ID、version付き設定、
汎用Observation、期待するtopic、QoS、retain、payloadを一組で固定する。
`scripts/test-site-output.sh`は実Mosquittoに対して汎用JSONとYokaKitの両routeを配送し、Broker停止中は
outboxへ残ること、再起動後に同じexport identityがPUBACK済みへ収束することを検証する。
YokaKit repositoryを隣接checkoutした環境では、`scripts/test-yokakit-consumer-contract.sh`が同じfixtureを
YokaKitの実decoderへ渡し、送信側とconsumer側のcontract driftを検出する。

## 9. IoTKit MQTT JSON v1 binding

`iotkit.mqtt-json.v1`は、特定applicationに依存しないIoTKit共通JSONをexact MQTT topicへ出力する。
`numeric`、`boolean`、`cumulative_value`、`alarm`の全汎用kindを、意味を変更せず受け入れる。

route設定は次の閉じたschemaとする。

```json
{
  "schema_version": 1,
  "topic": "factory/line-a/production"
}
```

`topic`は空でない完全なMQTT topic名である。ワイルドカード`+`、`#`、NUL、不正UTF-8、65,535 byte超を
拒否する。テンプレート、placeholder、暗黙のsensor名展開は行わない。複数routeへ同じtopicを設定でき、
一つのrouteは一つのexact topicだけを持つ。

payloadは次の閉じたIoTKit共通contractである。

```json
{
  "schema_version": 1,
  "observation_id": "d36cb7b3-7010-43b3-afc6-1931ed705dea",
  "series_id": "a921df88-6af2-46ca-a5f1-f346bf4433bb",
  "sequence": 42,
  "observed_at": 1784190000123,
  "kind": "cumulative_value",
  "value": 1524
}
```

alarm判断時に入力Observationが`reading`を持つ場合だけ、有限数値の`reading`を追加する。Adapterは
`kind`、`value`、identity、時刻を用途名へ読み替えない。QoSは1、retainはfalseである。

この共通contractを受け取れない会社固有システムには、IoTKit外部のConnectorで変換する。v1は
Connector実装、SDK、実行基盤を提供しない。

## 10. YokaKit MQTT v1 binding

`yokakit.mqtt.v1`は次のmodeだけを提供する。

| Generic kind | YokaKit mode |
|---|---|
| `cumulative_value` | `production` |
| `boolean` | `onoff` |
| `boolean` | `gantt_chart` |
| `alarm` | `alarm` |

`numeric`をYokaKit用途へ推測変換しない。IoTKit v1は文字列Observationを持たないため、
YokaKitの`barcode` modeも提供しない。

route設定は`schema_version=1`、`source_id`、`signal_id`、`kind`、任意の`reason`を持つ。
topicとpayloadはYokaKitの合意済み
`YokaKit MQTT Purpose-Bound Signal Contract v1`へ変換する。

YokaKit source statusはsemantic Observationの変換ではないため、Observation routeとは別の
source-level publicationとして扱う。

## 11. v1 exclusions

- runtimeで共有libraryを読み込むplugin ABI
- ConsoleからAdapter binaryを追加する機能
- external process、WASM、gRPC Adapter
- Connector実装、Connector SDK、Connector実行基盤
- HTTP/Webhook出力
- AdapterごとのBroker credential
- camera stream、barcodeの数値偽装
- application ACKのない外部contractをdurable application receiptと呼ぶこと

新しい外部サービスは、まずこのin-process契約を実装してcontract testを追加する。別transportや
第三者配布可能なpluginが必要になった時点で、security、resource limit、upgrade、isolationを含む
別major contractとして設計する。
