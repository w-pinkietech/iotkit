# IoTKit Site Console / API設計

Date: 2026-07-15
Status: Approved baseline; legacy capability inheritance under review

## 目的と正本

この文書は、`docs/redesign/decisions/D13-ui-scope.md`で延期していたIoTKit Siteの
管理UIと、その下に置くapplication service / HTTP APIを設計する。D13の既決を
置き換えるのではなく、CLIによるraw保管、`production_pulse`意味付け、MQTT export、
導入smokeが成立した後の次スライスとして具体化する。

目的は、IT専任者ではない工場の現場担当者が、工場LAN上のWindowsブラウザから次を
行えるようにすることである。

- Edge、デバイス、信号、MQTT出力の状態を日常確認する。
- デバイスと信号へ人間向けの名前と設置場所を付ける。
- 接点信号をfuture-onlyで`production_pulse`へ意味付けする。
- 実信号による保存前previewでカウント条件を確認する。
- Site単位の汎用MQTT出力を、内蔵Broker方式または外部Broker方式から選ぶ。

旧IoTKitは現場要求に応じて現在値、realtime chart、履歴検索、CSV/Excel、camera、threshold、通知、
host操作まで拡大した。この履歴を単なるlegacy burdenとして扱わず、利用者の仕事を示す要求証拠として
再評価する。継承するのはoperator capabilityであり、Node-RED、AngularJS、InfluxDB、固定port、
BravePI/BraveJIG固有分岐等の実現方法ではない。

IoTKitはYokaKitを内包しない。YokaKitは汎用MQTT出力を利用し得る外部applicationの
一つであり、Siteの共通schema、画面、設定名、認証、運用手順へYokaKit固有語彙を
持ち込まない。

## 対象外

初版では次を作らない。

- 品番、工程、ラインmaster、生産実績、OEE、alarm、業務dashboard
- 温度のsemantic meaningとapplication export
- 過去raw recordのbackfill、意味付け前データの遡及変換
- Siteを跨ぐ工場統合、fleet管理、複数Siteの上下関係
- adapter、BravePI Transmitter、BravePI Mainboardの設定・pairing・downlink UI
- Site HTTP APIを外部application向けの長期互換公開APIとすること
- credential、CA、証明書、Broker ACLのWeb UI発行・rotation
- SPA、Node-RED Dashboard、CDN依存、Node.js必須build
- MQTT descriptorのdelta、chunk、履歴replay、能力graph、双方向編集
- 生産KPI、OEE、品番/工程別chart等の業務dashboard
- camera録画、画像保管、画像認識、遠隔監視camera製品相当の機能
- 外部application向けcamera stream、埋め込みmedia API、cross-origin camera配信

外部applicationとの正規データ連携境界はMQTTである。HTTP APIはSite Consoleと
ローカル運用のための管理面であり、`/api/v1`はそのwire versionであって、外部連携
製品契約を意味しない。

## 旧IoTKit operator capability継承表

分類は「旧画面を再現するか」ではなく、「現場担当者の仕事をIoTKitのどこで継続するか」を表す。
`初期`はSite Consoleの最初の製品journeyへ含める候補、`後続`は要求を維持した上で別スライスにするもの、
`保留`は利用実態または責任境界の追加判断が必要なものとする。

| 旧operator capability | 継承判断 | 新しい責任境界 | 時期 |
|---|---|---|---|
| 登録済みdevice一覧・detail | 形を変えて継承 | adapter/Edge descriptorで発見し、Site profileで名前・locationを付ける。source IDは通常画面へ出さない | 初期 |
| device追加・access type選択 | 形を変えて継承 | 製品別formで手登録せず、adapter導入とdescriptor収束を正とする。未対応sensor追加はadapter開発体験で扱う | 初期/後続 |
| device削除 | 形を変えて継承 | hard deleteせずretire/stale化し、raw、profile、mappingを保持する | 初期 |
| sensor名、unit、表示桁、offset | 一部継承 | 名前はSite profile、unit/value typeはdescriptor所有。表示桁と校正offsetは意味と適用地点を別設計する | 名前は初期、他は保留 |
| 現在値一覧・自動更新 | 継承 | bounded current-value read modelを低頻度pollingし、表示中pageだけ更新する | 初期 |
| realtime chart・接点変化表示 | 形を変えて継承 | 設置・稼働確認に必要な短いrecent windowだけを表示し、業務dashboardにしない | 初期候補 |
| 履歴の期間検索・集約chart | 形を変えて継承 | Site raw/queryから有界rangeを返す。全履歴scanや無制限series比較を許さない | 初期候補 |
| CSV/Excel download・graph画像保存 | 一部継承 | 汎用CSVを正とし、秘密/internal identityを除外する。Excel専用生成と画像保存は追加価値を確認する | CSVは初期候補、他は後続 |
| 接点count・threshold立上り/立下り | 形を変えて継承 | future-only semantic mapping/evaluatorへ移し、初版`production_pulse`から汎用thresholdへ拡張可能にする | pulseは初期、汎用thresholdは後続 |
| MQTT broker/topic設定・送信 | 形を変えて継承 | Site単位のversioned output candidate/test/activateとoutboxで扱う | 初期 |
| email宛先・threshold mail | 要求を保持して保留 | MQTT consumerへ任せるかoptional notifierを持つか、低費用・単体完結性を含めて別判断する | 保留 |
| 接点出力・外部機器駆動 | 要求を保持して分離 | generic typed command/downlinkが必要。adapter固有commandをSite semantic処理へ直結しない | 後続 |
| storage空き容量・保存停止表示 | 継承 | Site/Edge statusへ明示し、容量不足を正常表示で隠さない | 初期 |
| USB camera live view | 形を変えて継承 | optional Edge camera serviceのHTTP mediaをSite/Caddy HTTPS originから中継。MQTTは能力/health metadataだけ | Site Consoleの初期候補 |
| cameraの外部application出力 | 要求を保持して後続 | 初期版では公開media APIやcross-origin埋め込みを作らない。stable camera identityとmedia service分離だけを維持する | 後続 |
| camera録画・画像認識 | 外部/後続へ分離 | IoTKit初版をvideo platformにしない。必要時はmedia保存契約とapplicationを別設計する | 対象外 |
| barcode reader入力 | optional adapterとして後続 | 既存現場は別systemからYokaKitへ直接MQTT送信する。IoTKit直結が必要な場合だけD1 v2 + D7出口familyを設計する | 初期対象外 |
| BravePI transmitter/router/module設定、DFU、省電力 | adapter固有面へ分離 | IoTKit共通Site Consoleへ製品固有語彙を持ち込まない | 共通UI対象外 |
| browser時刻によるhost時刻変更 | 実現方法を廃止 | deploymentのNTP/時刻同期とhealth表示へ置換する | Web UI対象外 |
| system再起動・shutdown | operator面へ分離 | local CLI/service managerで扱い、匿名Site UIからhost権限を持たせない | Web UI対象外 |
| DB初期化 | Web実現を廃止 | backup/restore/明示的local recovery手順へ置換し、通常UIへ破壊操作を置かない | Web UI対象外 |
| Swagger、version、license | 一部継承 | version/license/diagnostic refはsystem情報、API schemaは開発成果物として分離する | versionは初期、Swaggerは対象外 |

camera live viewは旧IoTKitの公式要件と実装証拠がある。barcode readerは旧IoTKit標準UIの抽出証拠ではなく、
既存YokaKitのMQTT入力とユーザーが示した現場要求を根拠に、この表へ追加した新しい継承要求である。

この表の`初期候補`と`保留`は実装承認ではない。range query、camera metadata、文字列観測等の公開wire、
custody、retentionへ波及する項目は、それぞれの正本をFull laneで改訂してから実装する。

## 観測種類の境界

Site Consoleは数値sensorだけをIoTKitと定義しない。将来を含む入力を次の三つに分ける。

- measurement: 温度、圧力、照度、接点状態等のscalar/fixed vector時系列
- discrete event: barcode読取、button押下等の有界文字列/離散観測
- media: camera live stream、将来のsnapshot等

measurementは既存record/custodyを使う。discrete eventをIoTKitへ直接取り込む場合はD1 v2/D7で
取り込みと出口を同時に決める。
media byte列はmeasurement recordやMQTT brokerへ流さず、独立media serviceと明示的なretentionを持つ。
三者のfailureは相互にraw custodyを停止させない。

初期版のmedia consumerはSite Consoleだけである。将来、外部applicationが映像を組み込む場合は、管理用
HTTP APIを流用せず、認証、認可、同時視聴数、帯域、CORS/CSP、監査を含むversion付きmedia contractを
別途設計する。

## 選択した構成

```text
Windows browser
  -> HTTPS
  -> Caddy
  -> private Site HTTP listener
       -> server-rendered HTML / embedded assets
       -> /api/v1
            -> Site application service
                 -> Store
                 -> descriptor read model
                 -> semantic evaluator
                 -> output supervisor

Site CLI ----------^  (local actor; HTTP必須ではない)
```

Site ConsoleはGoのserver-rendered HTMLと限定的なJavaScriptで作る。templateと静的assetは
Go binaryへ埋め込む。現在値は低頻度polling、mapping previewだけはbounded SSEを使う。
WebSocketを常時接続の既定にしない。

HTML handler、HTTP API、CLIはStoreを直接操作しない。共通のtyped application serviceを
呼び、検証、future-only境界、監査を一箇所に置く。HTML用view modelとJSON/SSE DTOは
分離し、必要になった画面だけを将来SPAへ置換できるようにする。初版から巨大な汎用REST
surfaceを固定しない。

`core/ops`のR14 operation catalogはRust EdgeにおけるR14実装であり、Go Siteへimportまたはremote
dispatchしない。Site mutationはR14のSite実装として、小さなtyped operation dispatcherをapplication
service内に持ち、precondition、settings権限、監査を統一する。これはD13の「UIだけのmutation pathを
作らない」をSite内で満たすもので、Edge DBや`core/ops`へSiteから書き込まない。本設計の承認は、
`AGENTS.md`、`CLAUDE.md`、`docs/architecture.md`の曖昧だった全component共通表現をこの境界へ揃える改訂を伴う。

通常のCLIをHTTP API wrapperにはしない。Site daemonやCaddyが停止していても、ローカルで
診断、passphrase reset、未配送出力の明示的abandonを行える必要があるためである。通常CLIと
HTTP APIは同じapplication serviceを利用し、networkへ出さないlocal recoveryだけを明示的な
例外とする。

## HTTPSと公開境界

productionではCaddyだけが工場LANへ443を公開し、Site backendを直接公開しない。Site
backendは同一hostのloopbackまたはprivate container networkでCaddyからだけ到達可能にする。
Caddy admin APIは無効にし、Caddyが`Forwarded` / `X-Forwarded-*`を上書きする。Siteは任意の
incoming proxy headerを信用せず、明示したexternal base URL、allowed Host、trusted proxyだけを
受理する。

証明書方式はSite UIで選ばせない。deployment側のCaddy設定で次を選ぶ。

- public ACME CA
- enterprise ACME CA
- operator管理の既存cert/key
- 必要な現場だけexplicitなinternal/private CA

Caddyのdata directoryは永続化する。ACME/internal managed certificateはCaddyが更新し、静的
cert/keyはoperatorまたは外部toolが更新する。証明書失敗時にHTTPへfallbackしない。通常開発だけ
`127.0.0.1` HTTPを許可し、production設定はLANへのinsecure bindを拒否する。MQTT TLSはCaddyを
通さず、UI HTTPSと別のhostname、certificate、renewal failure domainとして扱う。

IoTKitは無料CA profileやWindows trust helperを製品機能として同梱しない。private/internal CAを選ぶ
現場では、Windows trust配布を含む導入手順をdeployment成果物としてoperatorが用意する。Site UIは
CA選択、trust install、certificate更新を代行しない。

## Edge descriptor snapshot

Implementation status: Edgeの永続revisionとsnapshot生成、retained MQTT binding/ACL、Siteのstrict decode、
revision-aware durable replica、public ref、Site-local profile、current-value read modelまで実装済み。
HTTP APIとSite Console HTMLは後続スライス。

### 必要性と責任境界

現在のmeasurement recordは`edge_node_id`、`series_key`、値、時刻だけを持つ。Siteは
`series_key`から内部`system_id`を取り出せるが、現場担当者が物理センサーを照合するための
人間向け識別子、測定名、単位、値型を得られない。

SiteがEdge HTTP APIをpullしたり、作業者に全項目を手入力させたりせず、EdgeがMQTTで
adapter-neutralなdescriptor current-state snapshotを送る。これはmeasurement custodyとは異なる
現在状態の複製であり、既存records streamや`accepted-through`へ結合しない。

```text
iotkit/v1/edge-nodes/{edge_node_id}/descriptors
```

- QoS 1、retained
- Edge write / Site readのexact ACL
- 正本はEdge ledger / measurement registry
- Edgeのdescriptor変更時とMQTT接続成立時にcomplete snapshotを再送
- Broker retained messageは正本でもcustody証明でもない
- 初版のencoded上限は1 MiB。超過時はtruncateせず診断errorにする

このtopic追加に合わせ、descriptor contract文書、D10、`iotkit-edgectl mqtt-binding`、production
bootstrap、Mosquitto ACLを同じ変更で更新する。records/accepted-throughとはtopic、ACL、retain、
failure behaviorを分離する。

### Snapshot v1

```json
{
  "schema_version": 1,
  "edge_node_id": "edge-node-01",
  "ledger_epoch": "epoch-01",
  "descriptor_revision": 4,
  "complete": true,
  "devices": [
    {
      "system_id": "018f0000-0000-7000-8000-000000000001",
      "identifier": "01234567",
      "state": "active"
    }
  ],
  "signals": [
    {
      "series_key": "018f0000-0000-7000-8000-000000000001:contact_input_state:na:primary",
      "system_id": "018f0000-0000-7000-8000-000000000001",
      "measurement_key": "contact_input_state",
      "channel_index": null,
      "variant": "primary",
      "unit": null,
      "value_type": "bool"
    }
  ]
}
```

`descriptor_revision`はEdge DBへ永続化し、descriptor内容が変わるtransactionで増加する。同じ
`ledger_epoch`の低revisionは無視し、同revision・異内容はconflictとして監査する。通常再起動と
reconnectは同revision・同内容を冪等publishする。復元fenceには新しいepoch概念を作らず、既存の
`ledger_epoch`を使う。

Edge ledgerのdevice metadataへ任意の`presentation_identifier`を追加する。adapter/device declarationは
物理照合に必要な短い表示値だけをこのfieldへ渡し、Edgeが長さ、文字種、control characterを検証する。
BravePI adapterではTransmitter番号を設定できるが、共通schemaへ`serial_number`や`device_number`を
作らない。既存`hardware_id = ble:{number}`からgeneric codeがprefixを除いて自動生成することは禁止し、
adapter-owned projectionまたは明示migrationだけが表示値を作る。

descriptorの`identifier`はこの`presentation_identifier`の複製で、任意、非一意、交換で変化し得る
非権威表示値である。内部キー、認可、履歴継続判定に使わない。通常のdescriptorに`hardware_id`、SSID、
advertisement name、adapterの自由文、credential、provider payloadを含めない。

Siteは`system_id + measurement_key + channel_index + variant`から再構成した`series_key`が送信値と
一致することを検証する。`unit`と`value_type`はEdge registryのresolved metadataであり、Siteが変更
しない。measurementがdescriptorより先に来てもraw受理を止めず、placeholder signalを作って後から
enrichする。descriptorが先なら「データ未受信」と表示する。

complete snapshotから消えたdevice/signalをhard deleteしない。`stale`またはEdgeの明示的`retired`
として残し、Site固有profileとsemantic mappingを消さない。新しいsnapshotはSiteの表示名、設置場所、
mappingを上書きしない。

## Site-local identityとread model

Site内部のsource identityは次の既存複合キーを維持する。

```text
device source = (edge_node_id, system_id)
signal source = (edge_node_id, series_key)
```

HTTP URLにはこれらを直接出さず、最初に観測したときSiteが128 bit randomの`device_ref`と
`signal_ref`を発行して永続化する。refは認可tokenではなく、推測困難な安定resource referenceである。
descriptor、raw、profile、mappingは内部source identityで結び、API adapterがrefを解決する。

Site-local profileはdescriptor複製と別tableに持つ。

- device profile: 必須`display_name`、必須free-text`location`、revision
- signal profile: 必須`display_name`、revision
- identifier: descriptor所有、設定sessionだけに表示、編集不可
- semantic mapping: 既存の`(edge_node_id, series_key)`にfuture-onlyで結ぶ

工場、工程、ラインの階層masterは作らない。`location`は例えば
`第2工場・プレスライン出口`という一つの自由記述であり、YokaKit等のbusiness hierarchyと同期しない。

current-value read modelはcursor付きのbounded projectionで最新のvalid measurementをsignalごとにmaterializeし、
値、unit、event time、Site受信時刻を返す。validityと独立した最終受信時刻も保持し、一覧表示でraw履歴を
走査しない。event-only接点は長時間無通信でも故障とは限らないため、初版はmeasurement時刻だけから`stale`や
故障を断定しない。descriptorの`current/stale/retired`と、measurementの`never received/last received at`を
別field・別表示にする。将来adapterが明示的liveness契約を提供するまで、両者を一つの「正常」に畳まない。
descriptorやprofileの欠落はraw custody、ack、semantic projectorを停止させない。

## Application service

初版application serviceは次のuse caseだけを持つ。

- `GetSiteStatus`
- `ListDevices` / `GetDeviceProfile` / `UpdateDeviceProfile`
- `ListSignals` / `GetSignalProfile` / `UpdateSignalProfile`
- `GetSemanticMapping` / `PutSemanticMapping` / `DeactivateSemanticMapping`
- `StartMappingPreview` / `ObserveMappingPreview` / `StopMappingPreview`
- `GetOutputStatus` / `PutOutputCandidate` / `TestOutputCandidate` / `ActivateOutputCandidate`
- `ListAuditEvents`
- local-only `ResetSettingsPassphrase` / `AbandonOutputRevision`

Storeはapplication serviceに必要な狭いinterfaceとして注入し、HTTP DTOをdomain/store型として使わない。
mutationはprofile/mapping/output revision更新と監査を同じSQLite transactionでcommitする。秘密値とraw
payloadを監査detailへ入れない。

## HTTP API v1

### 匿名read surface

```text
GET /api/v1/status
GET /api/v1/devices
GET /api/v1/signals
```

この現場向け製品では、工場LAN内の日常利用の摩擦を下げるため、匿名でdisplay name、location、現在値、
最終受信時刻、正常/要確認、semantic mapping有無まで表示する。生産状況を推測し得る情報であることを
認識した上での製品判断である。匿名responseにidentifier、source identity、raw JSON、履歴、Broker endpoint、
credential、audit detailを含めない。

全responseは`Cache-Control: no-store`、同一origin限定、CORS default denyとする。Site UI backendを
factory LANへ直接公開せず、CaddyのHTTPS originからだけ利用する。

`/devices`と`/signals`はopaque cursor、default limit 50、最大100、`device_ref` / `signal_ref`昇順の
安定paginationを持つ。pollingは一覧全件ではなく、表示中pageまたは`updated_after`差分だけを更新する。
未設定profileは匿名画面で`未設定のデバイス` / `未設定の信号`と表示し、内部IDをfallback表示しない。

### Settings session

```text
POST   /api/v1/settings-session
GET    /api/v1/settings-session
DELETE /api/v1/settings-session
```

`POST`は共有settings passphraseをArgon2id hashと比較し、成功時にrandom bearer tokenを発行する。browserへ
渡すbearer token、DBへ保存するtoken hash、監査と画面へ出せる非秘密`session_ref`は別々に生成する。cookieは
productionで`Secure; HttpOnly; SameSite=Strict`、idle expiry 30分、absolute expiry 8時間とする。
`GET`はunlock状態と期限だけを返し、`DELETE`はlockする。passphrase変更/resetは全sessionを失効する。

全mutationとlogoutはCSRF tokenとOrigin検査を必須とする。passphrase試行はsourceごとに有界化し、成功・
失敗を秘密なしで監査する。Argon2id検証はSite全体で同時2件までとし、検証待ちqueueも有界化する。共有
passphraseのため個人識別を主張せず、network mutation actorは非秘密`session_ref`、CLI actorは
`local_cli`とする。bearer tokenとtoken hashをaudit responseへ出さない。

初回所有権確立と忘れたpassphraseのresetはSite host上のlocal CLIだけで行う。passphrase未設定状態で
設定APIをLANへ開放しない。開発loopback profileだけcookieのSecure要件を緩和できる。

### Profileとsemantic mapping

```text
GET /api/v1/devices/{device_ref}/profile
PUT /api/v1/devices/{device_ref}/profile

GET /api/v1/signals/{signal_ref}/profile
PUT /api/v1/signals/{signal_ref}/profile

GET    /api/v1/signals/{signal_ref}/semantic-mapping
PUT    /api/v1/signals/{signal_ref}/semantic-mapping
DELETE /api/v1/signals/{signal_ref}/semantic-mapping
```

個別GETと全mutationはsettings sessionを要求する。GETはresource `ETag`を返し、PUT/DELETEは
`If-Match`を必須とする。欠落は`428 Precondition Required`、revision不一致は
`412 Precondition Failed`とし、複数画面からの無音上書きを防ぐ。

初版meaningは`production_pulse`だけで、作業者は内部語を見ず次を必須選択する。暗黙defaultは作らない。

- `on_transition`: 対象状態へ変化した瞬間に1個
- `on_notification`: 対象状態の通知を受信するたびに1個
- `target_value`: ON (`1`) またはOFF (`0`)

内部では既存`active_edge` / `active_sample`へ対応する。BravePI専用の反対値補完を共通semantic処理へ
入れない。PUTは現在のaccepted cursorを開始境界にした新mapping revisionを作り、設定前rawを処理しない。
DELETEも現在cursorを終了境界にしてfuture-onlyでdeactivateし、raw、既存semantic event、既にenqueue済みの
出力を削除しない。

### Mapping preview

```text
POST   /api/v1/mapping-previews
GET    /api/v1/mapping-previews/{preview_id}/events
DELETE /api/v1/mapping-previews/{preview_id}
```

previewはsettings sessionに束縛したmemory-only resourceである。POST時点のsource cursorより後に届く
対象signalだけを、production evaluatorのpure functionで評価する。semantic mapping table、event、outbox、
MQTTへ一切書かない。

- TTL 5分
- 1 session同時1件、Site全体同時5件
- 最大100 input event
- 128 bit random preview IDとsession ownership検査
- logout、session expiry、Site restart、signal identity/descriptor revision不整合で終了
- SSE heartbeat 15秒、event size/countを有界化

設備を動かせない場合、preview未実施でもwarning付きでmappingを保存できる。「過去分は数えない」「保存後の
信号から有効」を画面に明示する。

### Audit

```text
GET /api/v1/audit-events?cursor=...&limit=...
```

settings sessionを要求し、limitは1〜100、default 50とする。操作時刻、actor class/session ID、operation、
resource ref、成功/失敗、secretを除いた要約を返す。個人名を返さない。

成功mutationの変更監査はmutationと同じtransactionでcommitする。認証拒否とtransaction開始前のvalidation
拒否は、元mutationとは別のbounded security/operation auditへ記録する。DB自体が書けない失敗ではdurable auditを
保証できないため、秘密なしのstructured local logとhealth errorを残し、「全失敗がDB監査される」と主張しない。

### Error contract

JSON API errorは次の形に統一する。

```json
{
  "error": {
    "code": "revision_mismatch",
    "message": "設定が別の画面で更新されました。再読み込みしてください。",
    "field": null,
    "request_id": "req_..."
  }
}
```

validationは400、未unlockは401、CSRF/権限は403、不在は404、rate limitは429、内部失敗は500/503を使う。
error、log、request ID相関情報へpassphrase、credential、private key、payload全文を出さない。body、header、
SSE、query、同時request、read limitを有界化する。

HTML/API responseにはstrict CSP（`default-src 'self'`を基礎にinline script禁止）、
`X-Content-Type-Options: nosniff`、frame embedding禁止、適切な`Referrer-Policy`を付ける。未検証の
`template.HTML`、`eval`、任意URLへのredirectを使わない。

## Site Console画面

旧機能継承の棚卸しを踏まえ、画面構成は次を候補とする。`モニター`、`ログ`、camera widgetの
初版範囲は、bounded query/media contractを決めてから確定する。設定unlockは独立した常設画面ではなく、
変更操作を始めるときのsession開始導線とする。

1. `状態`: Site、Edge、最終受信、storage、MQTT出力、要確認件数
2. `モニター`: 現在値、接点状態、短いrecent trend、optional camera live view
3. `デバイス`: display name、location、descriptor状態、最終受信
4. `信号`: display name、現在値、unit、最終受信、意味付け有無
5. `信号設定`: identifier照合、名前、カウント条件、live preview、保存
6. `ログ`: signalと期間による有界検索、chart/table、汎用CSV export
7. `出力`: mode、非秘密接続情報、candidate test、active/draining/pending状態
8. `監査`: 設定変更履歴
9. `システム情報`: version、license、診断reference。再起動、DB初期化、secret表示は置かない

日本語を既定とし、内部語の`active_edge`、`active_sample`、`series_key`、`ledger_epoch`を通常画面へ出さない。
大きな操作領域、明確な確認文、入力例、成功/失敗後の次行動を示す。identifierと診断IDは設定unlock後の
物理照合欄にだけ表示する。

実装順は、既にread modelがある`状態`、`デバイス`、`信号`の匿名read surfaceを最初に通す。その後、
recent trend/log queryの負荷上限とCSV field contractを決めて`モニター`、`ログ`を追加する。cameraは
Edge media serviceとSite proxyのcontractが成立した時点で`モニター`のoptional widgetとして加える。
設定、preview、output、auditは同じapplication service/API境界を利用し、read surfaceと別のmutation pathを
作らない。

## 汎用MQTT出力

### 一つのSite、一つのactive output

初版はSiteごとにactive outputを一つだけ持ち、次のmodeを選べる。

- `embedded_broker`: SiteがIoTKit内蔵Brokerのapplication topicへpublishし、外部applicationが購読
- `external_broker`: Siteがoperator指定の外部Brokerへpublish

意味付け、application contract、stable `event_id`、QoS 1 outboxはmodeから独立する。外部application名を
保存しない。初版meaningは`production_pulse`一つなので、output revisionは一つのexact topicを持つ。

### Credential境界

Site UI/APIへBroker credential発行権限を持たせない。現行のMosquitto password/ACL fileはBrokerへ
read-only mountし、Siteから変更できない境界を維持する。

- embedded modeのapplication subscriber: bootstrap/local CLIがexact read-only credentialを一つ生成し、
  Git外・0700 directoryのone-time handoffとして渡す
- external modeのSite publisher: operatorがusername、password file、CA fileをowner-only deployment fileとして
  設置し、Siteへread-only mountする
- UI/HTTP API: mode、endpoint、topic、credential installed有無、接続状態だけを扱う
- password、private key、secret file内容: HTML、API、SQLite、argv、environment、log、auditへ入れない
- 再発行、失効、rotation: 初版はlocal CLI / deployment作業。Web UIは後続

APIとDBはsecret pathそのものではなく、deploymentが許可した固定`credential_ref` / `tls_profile_ref`だけを
参照する。任意filesystem pathをUIから指定させない。

embedded modeのexact topicはdeployment allowlistへ事前登録する。bootstrap/local CLIはSite userのexact
write ACL、application subscriberのexact read ACL、password DBを同時生成し、必要なBroker reload/restartと
接続切断をoperator作業として扱う。UIで未準備topicを自由入力させない。external modeもdeploymentが許可した
endpoint/topic/profileだけをcandidateにできる。

### Candidate、test、activate

```text
GET    /api/v1/output
PUT    /api/v1/output/candidate
DELETE /api/v1/output/candidate
POST   /api/v1/output/candidate/test
POST   /api/v1/output/candidate/activate
```

すべてsettings sessionを要求する。`GET /output`はactive revision、candidate、connection、draining revision、
pending/failed countを返すが秘密を返さない。candidate PUTは非秘密設定だけを保存し、ETag/If-Matchを使う。

candidateのPUT、DELETE、test、activateはすべてcandidate ETag / `If-Match`を必須とする。activate
transaction内でも、成功testが同一candidate revision・同一content digestで直近10分以内であることを
再検査する。

testはDNS、TCP、TLS hostname/CA、MQTT authentication、CONNACKまでを確認する。外部Brokerのexact publish
ACLを無害に確認できる標準手段はないため、通常application topicへtest payloadを混ぜない。画面には
「接続確認」であり、最初のevent deliveryまでpublish ACLを証明しないことを明示する。candidate内容が
変わればtest結果を無効化し、activateには直近10分以内の同一candidate test成功を要求する。

output revisionはmode、endpoint、exact topic、`credential_ref`、`tls_profile_ref`をimmutableに束縛する。
drainまたは明示abandonが完了する前に、そのrevisionが参照するcredential/CA fileを交換・削除しては
ならない。初版はdraining revisionを同時に一つまでとし、pendingが残る間の次のactivateを`409`で拒否する。
次へ切り替える必要がある場合、operatorは旧endpointを復旧してdrainするか、local CLIで明示abandonする。

activateはsemantic eventの現在最大`event_row_id`を一つのtransactionでcutover境界にする。

```text
old output: end_at_event_row_id = boundary
new output: start_after_event_row_id = boundary
```

旧revisionは境界以前の未配送eventを旧endpointへdrainし続け、新revisionは境界より後のeventだけを新endpointへ
送る。未配送eventを新Brokerへ黙って移動せず、二つの出力へ同じeventを新規enqueueしない。旧endpointが
復旧しなくても新しいeventの配送を止めず、UIは旧pending件数と`draining`警告を表示する。

cutover時点でsemantic event tableには存在するがoutboxへまだenqueueされていない境界以前のeventも、旧revisionが
その`end_at_event_row_id`までenqueueする。新revisionは`start_after_event_row_id`以前を読まない。

旧pendingを破棄する操作は初版UIへ出さない。local CLIの`output abandon --revision ... --reason ...`だけが、
明示理由、件数、event境界を監査した上で`abandoned`へできる。abandoned eventを新outputへ再配送しない。

MQTT deliveryはat-least-onceで、PUBACK後だけpublishedにする。timeoutや切断ではpendingのまま再試行し、
consumerはstable `event_id`でdedupできる。output outageや失敗がraw custody / `accepted-through`を止めない。

## Failure behavior

- descriptor不正/oversize: 最後のvalid snapshotを保持し、Site statusを要確認にする。raw custodyは継続する。
- descriptor巻き戻し/conflict: 適用せず監査し、既存profile/mappingを保持する。
- profile/mapping validation失敗: transactionをcommitせず、field errorを返す。
- preview対象signal停止: heartbeatを続け、TTLで終了する。実event/outboxは生まない。
- output candidate test失敗: active outputを変更しない。
- output activate transaction失敗: old/new境界を一切変更しない。
- output publish失敗: pendingを保持し、bounded retry/backoffと状態表示を行う。
- Site DB failure: mutationを失敗させ、成功表示や監査だけを残さない。
- Caddy/certificate failure: HTTPへfallbackせず、local CLI healthで診断できる。

## Migrationと配置

Site SQLiteはschema migrationを導入し、descriptor cache、public refs、profiles、settings auth/session、audit、
output revisions/cutoverを追加する。既存raw、cursor、semantic mapping/event/outboxを破棄しない。新tableは
Site Storeが所有するが、migration SQLとapplication queryを分離してtest可能にする。

既存CLIの`query`、`mapping-set/list`、`route-add/list`、`semantic-query`は即座に削除しない。新application
serviceへ移し、Site Console journeyが成立した後に、重複する低水準route commandをdeprecatedにするかを
別判断する。

既存のmappingごとのMQTT routeを新しい単一outputへ自動集約しない。旧routeが存在するDBは
`legacy_output`として従来のendpoint/topic/outboxを継続し、新しいoutput activateを拒否する。operatorが
旧pendingをdrainし、local CLIの明示migrationでlegacy routeを終了境界へ固定してから、初回output candidateを
activateする。migrationは件数と境界を表示・監査し、旧eventを新outputへ再配送しない。公開前の使い捨て環境では、
operatorが明示的にDB再作成を選べるが、製品が自動削除してはならない。

production deploymentはCaddy、Site、Brokerの責務を分ける。Caddy data、Site data、Mosquitto data、TLS、
password/ACL、handoffを別の永続領域とし、secretをCompose environmentやrendered configへ展開しない。

## Verification方針

実装中は変更箇所に対応するfocused testだけを実行する。project全体testとDocker end-to-endを繰り返さず、
コード修正完了後のPR前に一度まとめて全体検証する。

初版の必須testは次である。

- application service: profile revision、future-only mapping put/delete、同transaction監査
- descriptor: strict decode、series_key整合、revision rollback/conflict、reconnect同一snapshot、stale化、
  measurement先着/descriptor先着
- auth: Argon2id、rate limit、cookie、idle/absolute expiry、CSRF、Origin、reset全session失効
- API: anonymous field absence、no-store/CORS deny、ETag/If-Match、error contract、bounded pagination
- preview: POST後だけ、2 mode × 2 target value、pure/no-write、ownership、TTL、limit、restart破棄
- output: candidate test非破壊、atomic cutover、old drain/new future-only、failure retry、local abandon監査、
  credential非露出
- HTML:主要journeyのhandler/component test、内部語とsecretの非表示、keyboard操作
- Docker: CaddyだけがHTTPS公開、backend非公開、descriptor retained収束、embedded/external output切替、
  Site/Broker restart後のpending収束

Raspberry Piではdescriptor publisherが実台帳からBravePI Transmitterの人間向けidentifierと温度/接点signalを
生成し、Site UIで照合できることだけを確認する。SSR、auth、SQLite、Caddy、外部Broker cutoverをPiで重複
検証しない。接点の実信号カウントはユーザーが実施可能な時だけ最終smokeとして行う。

最終pre-PR gateは既存方針どおり、`scripts/verify.sh`、Go全package、必要なDocker integration、新しい
Site Console E2Eを一度まとめて実行する。失敗後は原因箇所のfocused testで修正し、成功証拠が必要な
検査だけを再実行する。

## 完了条件

- 工場LANのWindows browserからCaddy HTTPS経由でSite Consoleを使える。
- 匿名の日常画面でデバイス/信号の名前、location、現在値、受信状態を確認でき、secret/internal identityは
  露出しない。
- settings unlock後に名前、location、接点の2方式/ON-OFFを設定でき、全mutationがrevision保護と監査を持つ。
- previewが実信号を表示するが、semantic event、outbox、MQTTへ書かない。
- Edge descriptor snapshotが製品固有schemaなしでidentifierとsignal metadataをSiteへ収束させる。
- Site固有profile/mappingがdescriptor更新、Edge restart、retireで消えない。
- embedded/external Brokerを選べ、candidate testとatomic future-only cutoverを経て配送できる。
- 旧output pendingは旧revisionへdrainし、黙った移送、損失、二重enqueueがない。
- credential、private key、passphraseがUI、API、DB payload、log、audit、Gitへ出ない。
- raw custodyと`accepted-through`がdescriptor、UI、semantic、output failureから独立して継続する。
- focused testと最終一回の全体検証が成功する。
