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
routeの停止は新しい変換を止める設定であり、既存outboxのMQTT配送は継続する。Consoleもrouteの
使用中・停止中とMQTTの配送中・滞留を別々に判定する。

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

`output_routes`は現在、利用者がruleごとに作る設定ではなく、Site全体の`export_profile`から
`profile_rule_binding`を経て展開される実行単位である。profile expanderがSite ID、
versioned Adapter ID・semantic rule ID・外部用途で特定される論理signal ID、rule kindから
exact route configを作る。Adapter自身はSite、rule一覧、将来ruleの自動追加を知らない。

Console/APIが利用するSite全体の操作面は次である。

- `GET /api/v1/export-profiles`: 外部出力先とbinding状態を一覧する
- `POST /api/v1/export-profiles`: 対応する現在・将来ruleへの継続適用を確認する。汎用出力は即時開始し、
  YokaKitはtopicとIDだけを準備する
- `PUT /api/v1/output-bindings/{binding_id}`: YokaKit boolean用途を確定し、topicとIDを準備する
- `POST /api/v1/output-bindings/{binding_id}/start`: topicを外部へ登録した確認後に、
  その時点より後のデータだけを送信開始する
- `POST /api/v1/export-profiles/{profile_id}/stop`: 新規変換を境界で終了し、既存配送をdrainする
- `GET /api/v1/output-bindings/{binding_id}/publication`: 完全なtopic/payloadを確認する

次のroute APIの読み取りは診断のため残す。

- `GET /api/v1/output-adapters`: 利用可能なdescriptorとmode
- `GET /api/v1/output-routes`: route、非秘密config、配送件数

個別routeを作るAPIとConsole操作は提供しない。これらから任意topic、source ID、signal IDを
作ることはできない。旧個別routeがDBに残っている場合はmigrationで停止し、未配送outboxだけを
既存の配送経路で処理する。API handlerやConsole handlerからSQLへ直接書き込まない。

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

Siteはrouteごとに、自由文ではないclosedな`last_transform_error_code`と発生時刻だけをdurable保存する。
初版のcodeは`adapter_unavailable`、`config_version_mismatch`、`invalid_observation`、
`transform_failed`である。config JSON、payload、credential、内部error文字列を診断欄へ複製しない。
一つのrouteの決定的変換失敗は同じbatchにある別routeの変換・outbox保存を止めない。失敗した
Observationにはoutbox rowを作らない。候補はrouteごとの古い順を保ち、最後に変換を試みた時刻が古い
routeから一件ずつinterleaveする。エラー中のrouteは最古の未変換Observationだけを再試行候補にする。
一つのrouteで最初の変換失敗が起きたら、そのbatch中の同route後続候補を処理しない。これにより壊れた
routeのbacklogがbatch limitを独占せず、正常routeの連続入力も修復済みrouteの再試行を飢餓させず、
後続成功が未解決の古い失敗を隠さない。routeの最古の未変換Observationが再び正常に変換できた時点で
error状態を解消する。

Consoleではこの変換状態をOutput Adapterの列へ表示し、`pending`、`published_at`から導くMQTT配送状態と
分ける。Output Adapter自身がBroker接続、retry、PUBACKを担当しているように表示してはならない。
短時間の`pending`は異常ではないため「配送中」として表示する。最古の未配送messageが5分以上残った
routeだけを「配送停止の可能性」として要確認にし、最終配送時刻と件数を同じ欄へ表示する。

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

Site全体の外部出力先ではtopicを人へ入力させない。`source_id`は`site_meta.site_id`、
`signal_id`は`(versioned adapter_id, semantic rule_id, mode)`ごとに暗号学的乱数から一度だけ発行し、
profile expanderが次を生成する。

```text
iotkit/v1/sources/<site-id>/signals/<signal-id>/observations
```

出力先を停止して同じAdapter・rule・modeで再追加した場合、新しいbindingとfuture-only開始境界を作るが、
`signal_id`は再利用する。別modeや別versioned Adapterには別の`signal_id`を発行する。payloadの
`series_id`が意味づけ系列を、`observation_id`が個々のObservationを識別するため、bindingの
ライフサイクルをsignal ID変更で表現しない。

Adapterへ渡す内部route設定は次の閉じたschemaを維持する。これはlegacy routeの読取りと、
Adapterを純粋変換に保つための実行形式であり、通常Consoleの入力schemaではない。

```json
{
  "schema_version": 1,
  "topic": "iotkit/v1/sources/site-0123456789abcdef0123456789abcdef/signals/sig-0123456789abcdef0123456789abcdef/observations"
}
```

`topic`は空でない完全なMQTT topic名である。ワイルドカード`+`、`#`、NUL、不正UTF-8、65,535 byte超を
拒否する。Adapter内でテンプレート、placeholder、sensor名展開は行わず、一つのrouteは一つのexact
topicだけを持つ。

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

YokaKitではtopicが入力登録契約でもある。profile追加やboolean用途確定だけではpublishせず、
Console/APIがexact topicとpayload例を提示する。導入担当者がそのtopicをYokaKitへ登録した後、
明示的な開始操作でaccepted cursor境界を保存して`prepared -> active`へ遷移する。登録確認より前の
Observationは後から送らない。

YokaKit source statusはsemantic Observationの変換ではないため、Observation routeとは別の
source-level publicationとして扱う。

production bootstrapはDB作成前に`site-<32hex>`を発行し、Siteの`--site-id`とBroker ACLへ同じ値を
渡す。`site-output`は
`iotkit/v1/sources/<site-id>/signals/+/observations`、
`yokakit/v1/sources/<site-id>/signals/+/observations`、同sourceのstatusだけを書ける。
既存DBを別の`--site-id`で起動した場合は起動を拒否する。

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
