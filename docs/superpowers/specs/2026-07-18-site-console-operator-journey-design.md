# IoTKit Site Console 利用者要求・導入Journey設計

Date: 2026-07-18
Status: Approved direction; implementation slice defined

## 1. 目的

IoTKit Site Consoleの中心目的は次である。

> センサーを接続したら、プログラミングなしで発見・設定・確認・保存・外部連携まで進められ、
> 問題が起きたら止まっている場所を画面から判断できる。

旧IoTKitは現場要求に応じて機能を拡大してきた。その実装方式や画面構成を再現するのではなく、
利用者が行っていた仕事を要求資産として継承する。本設計は
`2026-07-15-site-console-api-design.md`の画面中心の整理を、利用者Journey中心に改訂する。

## 2. 利用者

同じConsoleを権限に応じて利用する。別製品には分けない。

- 現場担当者: 現在値、履歴、異常を確認する。
- 設定担当者: 機器登録、信号設定、変換ルール、外部出力を設定する。
- システム管理者: アカウント、システム状態、監査を管理する。
- 導入担当者: ネットワーク、Broker、証明書等をConsole外の導入設定で準備する。

同じ人物が複数の役割を兼ねてよい。製品上のroleは既存の`viewer`、`admin`、
`system_admin`を維持する。

## 3. 求める操作感

- ログイン後30秒以内に現場が正常か判断できる。
- 新しいセンサーは自動的に「登録待ち」へ現れる。
- 実際の受信値を見ながら設定できる。
- Adapterが種類、値型、単位を報告済みなら候補を確認するだけでよい。
- 情報が不足していれば不足項目と次の行動を明示する。
- 値だけから種類や単位を推測しない。
- 内部IDや内部語は通常表示せず、物理照合・診断欄でだけ確認できる。
- 生値、補正後の値、変換結果を保存前に確認できる。
- 保存後いつから有効になるかを明示する。
- 失敗時は次に確認する場所を示す。
- 通常の現場作業にCLIを要求しない。
- 危険なhost操作とsecret管理だけを導入作業へ残す。

## 4. 情報の責任分離

同じ信号について、次の情報を混ぜない。

### 4.1 受信情報

AdapterとEdgeが報告した事実であり、Siteは上書きしない。

- Edge
- 取得元identity
- Adapterが報告した識別子
- measurement key
- channel
- value type
- canonical unit
- descriptor状態
- 生の受信値と受信時刻

### 4.2 現場設定

Siteの担当者が管理する。

- デバイス表示名
- 設置場所
- 信号表示名
- センサー表示分類
- 表示単位
- 表示桁数
- 設定状態

受信情報が完全な場合、センサー表示分類と表示単位は受信情報を初期値にする。担当者が
確認して保存した時点で設定済みとする。受信情報が不完全な場合は担当者が明示入力できる。
これはraw recordやEdgeのcanonical metadataを書き換えず、Siteでの表示・意味付けに使う
現場設定である。画面は情報の出所を「Adapterから」「現場設定」と表示する。

### 4.3 変換ルール

Site semantic definitionが管理する。

- scale
- offset
- 数値、ON/OFF、累積値、alarm
- 入力上昇時のしきい値と確定待ち時間
- 入力下降時のしきい値と確定待ち時間
- High側とLow側のどちらを有効状態とするか
- 有効状態への変化を数えるか、有効な受信ごとに数えるか

表示用profileへ変換ロジックを埋め込まない。生値とsemantic変換後の値を別に示す。
旧IoTKitの`hysteresisHigh`、`hysteresisLow`、`debounceHigh`、`debounceLow`、
`toggle`はこの判定ルールへ移す。確定待ち時間中の状態はDBへ保存し、Site再起動で失わない。
設定変更は従来どおりfuture-onlyとし、過去のraw recordやsemantic observationを書き換えない。

旧IoTKitの設定項目を同じ画面・同じDB表へ戻すこと自体は目的にしない。責務は次へ分ける。

- ADC倍率とoffset: semantic definitionの入力補正
- 立ち上がり、立ち下がり、各debounce、入力反転: semantic definitionの状態判定
- clear count: semantic counterに対する監査付き操作
- MQTT topicと追加payload: 外部出力route / adapter
- メール、接点出力の初期値・反転・連動先、撮影: 通知・action route / adapter
- 熱電対型: 対応Adapterが設定を受け付ける場合だけEdge側のAdapter設定

BravePI Mainboardとtransmitterのペアリングや熱電対型設定をSiteから行わない。既存iOSアプリと
Mainboardで管理される製品固有操作を、Site semantic definitionへ混ぜない。

### 4.4 外部出力

semantic observationと外部application contractの対応を管理する。Broker profile、credential、
CA、private keyは管理しない。

## 5. デバイスと信号

「デバイス」はAdapterが同じ取得元として識別した信号のまとまりである。例えば
BravePI Transmitter、直結I2Cセンサー、ESP32、PLCの取得単位である。

```text
IoTKit Edge
  └ 取得元デバイス
      ├ 信号 CH1
      └ 信号 CH2
```

デバイスと信号が1対1でもよい。導入時はデバイスと配下信号を一続きで設定し、日常監視では
信号を横断一覧として扱う。

### 5.1 Consoleで使う言葉

`device`と`signal`は内部modelとAPIの用語として維持する。Consoleでは、Adapterが同じ取得元として
まとめたdeviceを「デバイス」、deviceから届く個々のsignalを「センサー」と呼ぶ。

```text
BravePI Transmitter（デバイス）
  ├ 温度センサー 24.8 °C
  └ 接点入力センサー ON
```

Consoleの用語は次に統一する。

| 内部model | Consoleでの表示 |
| --- | --- |
| device | デバイス |
| signal | センサー |
| device profile | デバイス名・設置場所 |
| signal profile | センサー名・種類・単位 |
| semantic definition | センサーの設定 |
| setup device | 登録待ちデバイス |

Consoleはdevice groupingを維持し、配下signalをセンサーとして示す。measurement key、channel、
value typeは「Adapterから届いた情報」の詳細にだけ表示する。「センサー」と「値」を別entityとして
二段表示してはならず、現在値はセンサーの属性として表示する。

主要navigationは次とする。

```text
現場を見る
├ 概要
├ センサー
└ 受信履歴

設定
├ デバイス管理
├ センサー設定
└ 外部出力
```

既存の`/monitor`、`/setup`、`/signals`等の内部URLは互換性のため維持してよい。利用者に見える
page title、navigation、form、empty state、error、auditでは上表の現場語を使う。

## 6. 利用者Journey

### 6.1 現場概要

- Site、Edge、sensor、storage、外部出力の状態
- 受信中、停止、登録待ち、設定エラーの件数
- 新しく見つかった機器の案内
- 保存容量不足、projection停止、配送停止
- 問題から該当画面への直接導線

未設定があっても設定済み信号の監視とraw保存を妨げない。

### 6.2 新しい機器の登録

新しい信号をばらばらに見せず、device refでまとめた登録待ちデバイスとして表示する。

```text
登録待ちデバイス
  ├ Edge
  ├ 物理照合用identifier
  ├ descriptor状態
  ├ 最終受信
  └ 配下信号
      ├ 生値
      ├ measurement
      ├ value type
      ├ unit
      └ channel
```

操作順:

1. 現場の機器とidentifierを照合する。
2. デバイス名と設置場所を付ける。
3. センサーを動かし、生値と更新時刻を確認する。
4. 各信号の名前、表示分類、表示単位、表示桁数を確認または入力する。
5. 必要なsemantic変換を設定し、結果をpreviewする。
6. 登録状態を確認する。

一括保存で途中入力を失わせない。device profileと各signal profileは個別にrevision保護して保存し、
画面上で進捗を集約する。完了状態を一つのmutable flagとして保存せず、必要項目から導出する。

「デバイス登録待ち」と「センサー設定待ち」は別に集計する。デバイス名と設置場所を保存した時点で、
そのデバイスは登録待ちではなくなる。配下signalのprofileが未設定でも、デバイス登録待ちの台数へ
加算してはならない。未設定のsignalは配下の「センサーを設定」と各センサーの「確認して保存」で
示す。どちらの状態も既存profileの有無と内容から導出し、完了flagは保存しない。

### 6.3 日常監視

- 全信号の現在値
- Edge、device、location、sensor typeによる検索・絞り込み
- 数値、ON/OFF、複数channelに適した表示
- bounded pollingによる自動更新
- 短いrecent trend
- descriptor状態と最終measurementを分けた表示

event-driven signalを5分無通信だけで故障と断定しない。明示的livenessがない場合は
「最終受信」を事実として表示する。

### 6.4 履歴とexport

- signalと期間を指定した検索
- tableとchart
- contactのstep表示
- numeric trend
- cumulative/pulse history
- multi-channel表示
- CSV
- bounded query

Excel専用出力とgraph画像保存は要求として保持するがCSVより後に実装する。

### 6.5 変換ルール

- numeric
- boolean
- rising threshold / falling threshold
- rising debounce / falling debounce
- high-active / low-active
- transition/notification trigger
- cumulative counter
- counter reset
- alarm
- 実信号preview
- future-only開始境界

画面は「用途」だけで抽象化せず、入力、判定、結果を示す。
debounceは、条件が継続したことを次の受信値で確認して状態を確定する。サンプル間隔より短い
debounceを指定しても、確認前に推測で状態を変更しない。これはrawの受信順に再現でき、再起動や
backlog処理でも同じ結果になる。

### 6.6 外部出力

- 送信するsemantic observation
- 外部用source IDとsignal ID
- output adapter
- topic
- 接続、最終配送、pending、failure
- payload例
- test

Broker接続設定とsecretは導入済みprofileを使い、Consoleは非秘密状態だけを扱う。

### 6.7 通知・action

旧IoTKitの要求として次を保持する。

- threshold通知
- MQTT通知
- email通知
- contact output
- count
- test通知
- 発生・失敗履歴

sensor evaluatorへ送信実装を直結せず、semantic observationを受ける出力として分離する。

### 6.8 Adapter固有機能

共通Consoleへprovider固有語彙を常設しない。Adapterが能力を提供する場合だけ追加面を表示する。

- pairing
- scan
- power state
- parameter read/write
- restart
- immediate uplink
- DFU
- command busy/success/failure/timeout

### 6.9 Camera

旧IoTKitの要求として保持する。camera一覧、名前、場所、live view、healthを扱う。初期版で録画、
画像認識、外部application向けmedia APIを作らない。

### 6.10 System、account、audit

- version、license
- Site、Edge、Adapter health
- storage
- MQTT接続・配送
- certificate期限・更新状態
- time sync
- diagnostics
- account発行、role変更、無効化、password reset
- 設定変更履歴

DB初期化、host reboot、secret表示は通常Consoleへ置かない。

## 7. 画面構成

```text
概要
├ 新しい機器
├ 現在の状態
└ 要対応

機器と信号
├ 登録待ち
├ 登録済みデバイス
└ 信号設定

データ利用
├ モニター
├ 履歴
├ 変換ルール
├ 外部出力
└ 通知・アクション

管理
├ Adapter固有機能
├ システム
├ アカウント
└ 変更履歴
```

最初の実装では既存URLを壊さず、`/setup`を追加する。`/devices`は登録済みデバイスの閲覧、
`/signals`は横断的な信号設定として残す。概要の登録待ち案内と`/setup`が導入Journeyを所有する。
後続で利用実績を見てnavigation labelを統合する。

## 8. 最初の実装slice

### 8.1 対象

- `/setup`登録待ち画面
- deviceごとのsignal grouping
- device identifierのadmin限定表示
- descriptor情報、生値、最終受信の表示
- device name/location設定
- signal name、sensor display type、display unit、decimal places設定
- Adapter由来と現場設定のprovenance表示
- 必須情報から導出した登録状態
- 概要の登録待ち件数と導線
- 既存monitor/logでeffective表示設定を使用
- APIとHTMLを同じtyped application serviceへ接続
- revision protection、role、CSRF、audit

### 8.2 対象外

- Edge canonical registryの変更
- custom measurement key作成
- Adapter設定・pairing
- recent trend、range query、CSV
- notification、camera
- Broker profile変更

### 8.3 Signal profile v2

既存profileを拡張する。

```text
signal profile
  display_name        required
  display_sensor_type required
  display_sensor_type_label required when type=custom
  display_value_kind  numeric | boolean
  display_unit_mode   unit | dimensionless
  display_unit        required when mode=unit
  decimal_places      0..6, numeric only
  revision
  updated_at
```

descriptor値は別tableのまま維持する。effective表示はprofile値を優先し、profile未作成時は
descriptor候補を使う。ただし候補だけでは「設定済み」にしない。担当者がprofileとして保存して初めて
登録済みになる。

`display_sensor_type`は初版で次の閉じた候補と`custom`を持つ。

- thermocouple
- contact
- illuminance
- distance
- voltage
- current
- pressure
- humidity
- acceleration
- custom

`custom`では日本語の自由な表示名を別途必要とする。このsliceではcustom labelを
`display_sensor_type_label`としてprofileに保存する。

`display_unit_mode`を持つのは「まだ単位を決めていない」と「単位を持たない値」を区別するためである。
`boolean`は`dimensionless`かつ`decimal_places=0`に固定する。`numeric`は`unit`または
`dimensionless`を担当者が明示して保存する。`unit`の場合だけ、1文字以上32文字以下の
`display_unit`を必要とする。

descriptorからの候補変換は閉じた対応表にする。

| descriptor measurement key | 表示分類候補 | 値型候補 | 単位候補 |
| --- | --- | --- | --- |
| `temperature_c` | `thermocouple` | `numeric` | `°C` |
| `contact_state` | `contact` | `boolean` | 単位なし |
| `illuminance_lux` | `illuminance` | `numeric` | `lx` |
| `distance_mm` | `distance` | `numeric` | `mm` |
| `voltage_mv` | `voltage` | `numeric` | `mV` |
| `current_ma` | `current` | `numeric` | `mA` |
| `differential_pressure_pa` | `pressure` | `numeric` | `Pa` |
| `relative_humidity_percent` | `humidity` | `numeric` | `%RH` |
| `acceleration_mg` | `acceleration` | `numeric` | `mg` |

表にないmeasurement keyは勝手に分類せず、`custom`を候補にして表示分類名の入力を要求する。
descriptorにcanonical unitがある場合は上表の単位よりdescriptorを優先して候補表示する。
候補は入力支援であり、保存済みprofileを自動更新しない。

通常画面ではセンサーの物理的な種類と測定対象を混同しない。初版で扱う温度入力は
「熱電対」と表示し、Adapterから届く`temperature_c`は接続元の測定キーとして別に表示する。
既存DBの`temperature`は互換入力として受理するが、画面では「熱電対」と読み替え、
次回保存時に`thermocouple`へ正規化する。「光」は測定量として確立した「照度」と表示する。

### 8.4 登録状態

- `waiting_for_device`: device profileなし
- `waiting_for_signal`: device profileあり、未profile signalあり
- `metadata_missing`: descriptorもprofile候補も不足
- `ready`: device profileあり、全signal profileあり

登録状態と通信状態を混ぜない。descriptorの`current` / `stale` / `retired`と最終受信時刻は
別field・別表示にする。どの状態もraw custody、semantic projection、output deliveryを停止しない。

「profileあり」は、行が存在するだけでなくSignal profile v2のvalidationを満たすことを指す。
旧profileの表示名はmigration後も保持するが、v2項目を確認して保存するまでは
`waiting_for_signal`または`metadata_missing`として扱う。

### 8.5 Application/API境界

HTML handlerがstoreへ直接依存しない。Site application serviceに登録画面専用のread modelを置く。

```text
ListSetupDevices(actor) -> []SetupDevice

SetupDevice
  device              DeviceSummary
  identifier          admin以上だけHTTP DTOへ含める
  setup_state
  signals             []SetupSignal

SetupSignal
  signal              SignalSummary
  raw_latest
  descriptor_facts
  profile
  profile_complete
  candidate
```

JSON APIにはadmin以上向けの`GET /api/v1/setup/devices`を追加し、設定候補と物理照合情報を返す。
既存の`GET /api/v1/devices`と`GET /api/v1/signals`へidentifierを追加しない。profile更新は既存の
`PUT /api/v1/devices/{device_ref}/profile`と`PUT /api/v1/signals/{series_key}/profile`を維持し、
signal側requestをSignal profile v2へ拡張する。

Consoleの`GET /setup`はviewerにも表示するが、identifierはadmin以上だけに表示する。profile変更は
admin以上に限定する。概要の登録待ち件数、`/setup`、monitor、logは同じapplication read modelから
導出したprofile完成判定とeffective表示設定を使う。

## 9. Error behavior

- descriptor未着: raw値と「Adapterから種類・単位が届いていません」を表示する。
- identifierなし: 照合情報なしと表示し、内部system IDを代用表示しない。
- revision競合: 入力を失わず、再読み込みが必要な対象を示す。
- signal保存失敗: device profileをrollbackしたように見せず、該当signalだけ失敗表示する。
- descriptor更新: 現場profileを上書きせず、候補との差を注意表示する。
- 不正unit/precision: field単位でerrorを表示する。

## 10. Security

- 全画面login必須
- viewerは閲覧のみ
- admin以上がprofile変更
- identifierはadmin以上の照合欄だけ
- source identity、series key、credential、raw payload全文は通常画面に出さない
- 全mutationはCSRF、Origin、revision precondition、個人auditを持つ

## 11. Verification

- migration: 既存profile保持、upgrade/reopen
- store: profile v2 CRUD、descriptor候補との分離、grouping、登録状態
- application: validation、revision、role、audit
- HTTP: API DTO、HTML form、CSRF、内部identity非露出
- Console: 登録待ち一覧、device grouping、候補表示、不足表示、保存後ready
- browser: desktop/mobile、検索、設定保存、更新後monitor/log反映
- regression: raw acceptance、semantic projection、output deliveryに影響しない

## 12. 完了条件

- 新しいdeviceが自動的に登録待ちへ現れる。
- 同じdeviceのsignalが一画面へまとまる。
- 生値、descriptor type/unit、最終受信を見ながら設定できる。
- metadataがあれば候補入力され、なければ不足と入力欄が示される。
- device name/locationと全signal profileを保存するとreadyになる。
- monitorとlogが現場設定した名前、種類、単位、桁数を使う。
- viewerは状態を見られるが変更できない。
- raw custody、semantic、outputの既存testが維持される。
