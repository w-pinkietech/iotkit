# Site-wide Output Profiles Design

Date: 2026-07-20

Status: User-approved design

## 1. Purpose

IoTKit Siteの外部出力をsemantic ruleごとの手動route設定ではなく、Site全体の少数の
外部出力先として管理する。現場担当者はOutput Adapterを一度選ぶだけで、対応する既存ruleと
将来追加されるruleをその出力先へ接続できる。

内部ではrule単位のdurable route、変換失敗の隔離、future-only境界、MQTT outboxを維持する。
外部出力先はBroker接続、credential、CA、private keyを所有しない。

## 2. User model and terminology

通常のConsoleでは`profile`、`binding`、`route`を利用者用語として表示しない。

- 外部出力先: Site全体で有効にするOutput Adapter設定
- 送信する値: semantic ruleが作る数値、状態、累積値、アラーム
- 設定が必要: Adapterへ安全に対応付けるため人の選択が必要な値
- 対象外: Adapterが契約上受け取れない値

内部の永続model名は次とする。

```text
export_profile
  └─ profile_rule_binding
       ├─ references output_signal_identity
       └─ output_route
            └─ output_outbox
```

`export profile`をBroker connection profileと呼ばない。v1では全export profileが導入時に
設定された一つのactive Broker connectionを共有する。

## 3. Scope

v1は次を提供する。

- `yokakit.mqtt.v1`と`iotkit.mqtt-json.v1`をSite全体の外部出力先として登録
- 異なるAdapterを同時利用
- 一つのAdapter IDにつき一つのactive export profile
- 既存ruleと将来ruleへの自動binding
- Adapter互換性とYokaKit用途によるactive、needs-configuration、ineligibleの分類
- profileのdraining stopと停止履歴
- 完全なtopic/payload preview
- ConsoleとAPIでの状態確認

v1は次を提供しない。

- 同じAdapter IDの複数active profile
- profileごとに異なるBrokerへ配送する機能
- ConsoleでのBroker endpoint、credential、証明書、ACL設定
- rule単位の任意除外toggle
- 過去Observationのbackfill
- 停止済みprofileのresume
- DB cloneを分散lease等で自動検出する機能

## 4. Adapter application policy

### 4.1 IoTKit common MQTT

`iotkit.mqtt-json.v1`は`numeric`、`boolean`、`cumulative_value`、`alarm`の全ruleへ自動適用する。

topicは利用者に入力させず、bindingのimmutable identityから生成する。

```text
iotkit/v1/sources/<source-id>/signals/<signal-id>/observations
```

payloadはIoTKit共通Observation contractの完全形とする。

### 4.2 YokaKit

`yokakit.mqtt.v1`は次の対応だけを許可する。

| IoTKit kind | YokaKit kind | Binding |
|---|---|---|
| `cumulative_value` | `production` | 自動でactive |
| `boolean` | `onoff`または`gantt_chart` | 人がruleごとに選択するまでneeds-configuration |
| `alarm` | `alarm` | 自動でactive |
| `numeric` | なし | ineligible |

boolean用途を表示名、sensor type、現在値から推測しない。同じbindingから`onoff`と
`gantt_chart`の両方を出さない。用途を変更する場合は既存bindingを停止し、新しいpurpose-bound
bindingを作る。

alarmの`reason`は任意とする。空欄を送信停止理由にしない。Consoleはrule表示名を初期提案できるが、
保存されたreasonを表示名変更へ追従させない。

## 5. Identity

### 5.1 Source identity

YokaKitとIoTKit共通MQTTの`source-id`には`site_meta.site_id`を使う。callerから`source_id`を
受け取らず、typed application operationがDBから読み込んで強制する。

現行`site_id`は次の形式である。

```text
site-<128-bit lowercase hex>
```

DB open時に`site_meta`がexact singletonであり、`site_id`が`^site-[0-9a-f]{32}$`へ適合することを
検証する。欠損・破損時に自動再発行せずfail closedにする。

### 5.2 Signal identity

`signal-id`はMQTT topic上で論理出力信号を識別する安定した住所であり、bindingのライフサイクルIDでは
ない。Adapter契約、semantic rule、Adapter内用途の組ごとに一度だけ、Goの`crypto/rand`による
128bit乱数から次の形式で発行する。

```text
sig-<128-bit lowercase hex>
```

表示名、sensor type、Edge ID、IP、MAC、pin、Broker設定から生成しない。entropy取得に失敗した場合は
時刻、counter、MAC等へfallbackせずtransactionを失敗させる。

`output_signal_identities`をbinding履歴から独立したidentity台帳とする。identity keyは
`(adapter_id, rule_id, mode)`である。`adapter_id`にはversionを含み、`mode`は汎用MQTTの
`observation`、またはYokaKitの`production`、`onoff`、`gantt_chart`、`alarm`とする。

同じAdapter契約、rule、modeでprofileを停止・再追加した場合は、binding IDとfuture-only開始境界だけを
新しくし、signal IDは台帳から再利用する。別rule、別versioned Adapter、別modeには新しいsignal IDを
発行する。semantic ruleの設定変更ではsignal IDを維持し、payloadの新しい`series_id`で系列変更を表す。
`observation_id`は個々のObservationを一意に識別する。

`UNIQUE(adapter_id, rule_id, mode)`と`UNIQUE(source_id, signal_id)`を永続制約とする。identityは
profileやbindingの停止時に削除しない。表示名、sensor type、credential更新でも維持する。

### 5.3 Identity is not authentication

source/signal IDをsecretまたはpublisher認証として扱わない。Brokerは匿名publishを禁止し、Site固有の
credentialとexact source prefix ACLを使用する。

```text
yokakit/v1/sources/<site-id>/#
iotkit/v1/sources/<site-id>/#
```

別sourceへのpublishと不要subscribeをnegative integration testで拒否する。MQTT client IDは接続競合の
診断には使えるが、認証根拠またはclone防止保証にはしない。

## 6. Persistence

### 6.1 Export profile

`export_profiles`は少なくとも次を持つ。

```text
profile_id
display_name
adapter_id
adapter_schema_version
profile_config_json
state                 active | draining | stopped
auto_bind_future_rules
revision
created_at
drain_requested_at
stopped_at
```

credential、certificate、Broker URLを保存しない。active profileはAdapter IDごとに一つへ制約する。
`auto_bind_future_rules`はv1では常にtrueだが、継続的な外部送信許可を監査できるよう明示保存する。

### 6.2 Profile-rule binding

`output_signal_identities`は少なくとも次を持つ。

```text
output_identity_id
adapter_id
rule_id
mode
source_id
signal_id
created_at
```

`UNIQUE(adapter_id, rule_id, mode)`と`UNIQUE(source_id, signal_id)`を持つ。identity作成または再利用は
binding作成transaction内で行い、競合時に別IDを発行して同じ論理信号を分裂させない。

`output_profile_rule_bindings`は少なくとも次を持つ。

```text
binding_id
profile_id
rule_id
output_identity_id
reason
state                 needs_configuration | active | ineligible | draining | stopped
ineligible_reason
start_boundary
end_boundary
revision
created_at
activated_at
stopped_at
```

`start_boundary`と`end_boundary`は、対象signalのEdge ledger epochとaccepted source pub sequenceを
組にしたclosedな上流境界である。semantic observation row IDだけを境界に使わない。未projectionのraw
recordがactivationまたはstop transactionの後にsemantic Observationへなる場合も、観測時点が境界の
内側か外側かを判定できなければならない。

`UNIQUE(profile_id, rule_id)`を持つ。YokaKit booleanのneeds-configuration bindingは
`output_identity_id`を持たず、用途確定transactionで選択したmodeに対応するidentityを作成または再利用し、
その時点の上流開始境界を保存してactiveにする。設定待ち期間のObservationを後からbackfillしない。
APIとConsoleのread modelではidentityをjoinし、従来どおり`source_id`、`signal_id`、`mode`を返す。

### 6.3 Output route

現行`output_routes`はruleごとの実行、診断、outbox生成単位として維持する。binding IDとprofile revisionを
参照し、binding成立時にAdapterのexact versioned route configへ展開する。

profile設定を直接Adapterの`Transform`へ渡さない。profile expanderはprofile config、immutable rule
descriptor、binding identityからexact route configまたはclosedなineligible reasonを返す。
`Transform(route config, Observation)`は純粋変換のまま維持する。

## 7. Transaction boundaries

### 7.1 Profile activation

利用者が確認画面で有効化するまでpublishしない。有効化transactionは次をatomicに行う。

1. export profileを作成する
2. その時点の全active semantic ruleを分類する
3. auto-active bindingへ論理出力identityを作成または再利用し、ruleのsourceに対するfuture-only上流開始境界を保存する
4. needs-configuration、ineligible bindingも永続化する
5. active bindingのconcrete output routeを作る
6. 継続的なfuture-rule自動送信許可を監査する

開始境界を非同期reconcilerで再計算してはならない。

### 7.2 Future rule creation

semantic rule作成transactionはactive export profileを同じtransaction内で評価する。

- 自動対応可能: binding、論理出力identity、開始境界、routeを同時作成
- YokaKit boolean: needs-configuration bindingを作る
- 非対応: ineligible bindingを作る

reconcilerはmaterializeの再試行に使えるが、保存済み開始境界を作り直さず、同じidentity keyへ
別signal IDを発行しない。

### 7.3 Rule retirement

rule retire時にbinding/routeを即時停止しない。ruleの終了境界内にあり、retire transaction後にprojection
されるObservationを変換し、outboxをdrainする。projection終了と未変換Observation/outboxの解消後に
bindingをstoppedへ進める。retired ruleをfuture profileへ再bindingしない。

### 7.4 Profile stop

外す操作は削除ではなくterminal stopである。

```text
active -> draining -> stopped
```

停止transactionは各active child binding/routeへ、そのruleのsourceに対する終了境界を保存する。
開始境界より後かつ終了境界以下のraw recordから作られるObservationを、stop要求後にprojectionされた
ものも含めてoutbox化する。対象projectionが境界へ到達し、既存outboxをPUBACKまで配送してから
stoppedにする。profile、binding、route、identity、診断、監査をDBから削除しない。

停止済みprofileはresumeしない。「もう一度追加」は新profile、新bindingとしてfuture-onlyで開始するが、
同じAdapter契約、rule、modeのsignal IDは再利用する。停止期間のObservationを送信しない。

## 8. Console

### 8.1 External destinations

`/outputs`は外部出力先カードを表示する。

- 表示名
- Adapterの平易な名称
- 使用中、設定が必要、対象外、停止、変換エラーの件数
- MQTT配送中、5分以上滞留、最終配送時刻
- 「今後追加される対応値も自動追加」
- 使用中、配送終了処理中

通常画面ではprofile/route、config schema等の内部語を使わない。
停止済みprofileは現行の外部出力先カードとして表示しない。DBと変更履歴には残し、同じAdapterを
新しいprofileとして追加できる状態へ戻す。通常画面へ停止履歴一覧を新設しない。

### 8.2 Add destination

1. `YokaKitへ送る`または`汎用MQTT JSONで送る`を選ぶ
2. 現在のruleを「自動で送信」「設定が必要」「対象外」に分類して確認する
3. 「現在の対応値と今後追加する対応値を自動送信する」継続許可を明示する
4. `この内容で送信を開始`でatomic activationする

YokaKit booleanだけruleごとに`ON/OFFとして使う`または`稼働区間として使う`を選ぶ。未選択ruleだけ
保留し、他ruleを止めない。

### 8.3 Rule creation and sensor detail

rule作成画面は、そのruleがどの有効な外部出力先へ自動追加されるかを保存前に表示する。保存後の
センサー詳細には外部出力先名badgeとbinding状態を表示する。

### 8.4 Stop

停止確認は次を明示する。

> 新しい値の変換を境界で終了します。すでに配送対象になった値はMQTT配送を続けます。
> 停止中に発生した値は、後から外部へ送信されません。

### 8.5 Technical information and preview

source ID、signal ID、完全topicは読取専用の「技術情報」に表示し、個別にcopy可能にする。

payload previewを省略した手書きJSONにしない。

- bindingが永続化済みで最新Observationがある: 保存済みconfigと最新Observationを実Adapterへdry-runし、
  「最新値を使った変換結果」として完全なtopic、QoS、retain、payloadを表示する
- profile有効化前または最新Observationがない: 全必須fieldを持つschema-complete topic/payloadを
  「サンプル」と明示する。未発行のsource/signal/observation identityを実際の送信値と称さない
- outbox作成後: durable outboxの実topic、QoS、retain、payloadを「実際の送信内容」として表示する

previewはoutboxを作らずpublishしない。credential、certificate、secretを表示しない。

## 9. API and authorization

APIは少なくとも次のtyped operationを提供する。

- list/get export profiles and binding summaries
- preview profile activation
- activate profile with explicit confirmation and revision precondition
- configure pending YokaKit boolean binding
- request profile drain/stop
- get binding technical information and dry-run/actual publication

viewerは状態、topic、payload previewを閲覧・copyできる。adminとsystem adminはprofile作成・停止、
boolean用途、alarm reasonを変更できる。Broker connection設定はConsole/APIの対象外とする。

全mutationはCSRF、Origin、role、revision preconditionを検証し、actor、profile、Adapter、
対象rule件数、継続許可、outcomeを監査する。config、payload、credential、内部error文字列を監査へ
複製しない。

## 10. Failure handling

- 一つのbindingの変換失敗は他bindingを止めない
- needs-configurationは異常ではなく部分稼働として表示する
- ineligibleは異常件数へ含めない
- 短時間pendingは「配送中」、最古未配送が5分以上なら「配送停止の可能性」
- profile全体を単純な赤/緑にせず、送信中件数と要対応件数を併記する
- Broker PUBACKをYokaKitのInput登録済みまたはapplication受理済みと表示しない
- YokaKit exact topicのInput登録はYokaKit側の別操作であることを明示する

状態優先度は次とする。

```text
設定または変換エラー
  > MQTT配送停止の可能性
  > boolean設定待ち
  > 送信中
  > 停止中
```

## 11. Backup, restore, and clone safety

正式backupはsite ID、profile、binding、output signal identity、semantic state、outboxを一緒に保持する。
Broker credentialとprivate keyをDB backupへ含めない。

cold restoreは次の順序を要求する。

1. 旧Siteを停止する
2. 旧credentialを失効または接続不能にする
3. DBを復元する
4. 新しいcredential/connection profileを導入する
5. semantic series generationを新世代へ切り替えるrestore operationを実行する
6. 出力を明示的に再開する

restoreではsource/signal IDを維持する。古いsnapshotからの復元後に同じseries IDとsequenceを再利用しない。

DB cloneを新Siteとして利用しない。新Siteは新規DBから初期化する。同一credential cloneをBroker側で
接続競合として可視化できても、それを唯一のclone防止保証としない。

## 12. Verification

最低限次を自動検証する。

- fresh DBとmigration後DBのprofile/binding schema
- site ID singleton、syntax、破損fail-closed
- signal IDのsyntax、identity key単位のuniqueness、entropy failure rollback
- 同じAdapter・rule・modeのprofile再追加ではsignal IDを再利用し、別modeでは再利用しないこと
- profile activationと既存rule bindingのatomic future-only境界
- future rule作成とbinding/route作成のatomicity
- YokaKit cumulative/alarm自動対応、boolean pending、numeric ineligible
- generic adapterの全kind自動topic生成
- profile stop、rule retire、projection、outboxのdrain
- profile再追加が新binding、同じ論理出力identity、future-only境界となりbackfillしないこと
- 一bindingの変換失敗が他bindingを止めないこと
- 別source prefixへのpublishと不要subscribeをACLが拒否すること
- complete dry-run previewとdurable actual publicationがAdapter出力とbyte単位で一致すること
- Consoleの権限、確認、部分稼働、対象外、配送滞留表示と、停止済みprofileの通常画面からの除外
- DB cloneではcredentialが存在せずpublishできない導入境界
- restore後にsemantic series generationが更新されること

実Mosquitto gateではYokaKitとIoTKit共通MQTTを同時に有効化し、複数ruleのtopic/payload、QoS 1、
retain false、Broker停止中のoutbox保持、復旧配送、停止drainを確認する。
