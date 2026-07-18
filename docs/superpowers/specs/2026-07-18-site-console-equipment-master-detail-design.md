# IoTKit Site Console 機器管理一覧・詳細設計

Date: 2026-07-18
Status: Approved for implementation by operator feedback

## 1. 背景

階層型機器管理の最初の実装は、Edge、デバイス、センサーの親子関係を一画面へ
正しく表示した。しかし、一覧、状態確認、診断、設定フォームを縦方向へすべて展開したため、
対象を探すためのスクロールが長く、情報の優先順位も分かりにくい。

根本原因は視覚装飾ではなく、データ階層をそのまま一つの画面階層へ変換したことである。
機器管理を一覧・詳細型へ変更し、一度に扱う対象を限定する。

## 2. 採用する構成

次の三画面へ分ける。

```text
/equipment
  Edge一覧

/equipment/edges/{edge_ref}
  Edge詳細
  └ デバイス一覧

/equipment/devices/{device_ref}
  デバイス詳細
  └ センサー一覧・基本設定
```

breadcrumbで現在位置を示す。

```text
機器管理
機器管理 > factory-edge-01
機器管理 > factory-edge-01 > 乾燥炉入口 BravePI
```

`/equipment`を正規入口とし、既存`/edges`、`/setup`、`/devices`は互換用に残す。

## 3. Edge一覧

一覧画面の目的は、Siteが把握しているEdgeと要対応箇所を短時間で判断することである。

各Edgeを一行またはコンパクトなカードとして表示する。

- Edge名。Site固有名がなければ`edge_node_id`
- 登録状態
- デバイス数
- センサー数
- 最終通信
- 要設定件数
- `詳細を見る`

上部の四段階Journey、配下デバイス、センサー、設定フォームは表示しない。
未登録Edgeのprimary actionは`登録内容を確認`とし、登録操作自体はEdge詳細に置く。

desktopではtable相当の横並び、mobileでは一行をcardへ折り返す。
検索、pagination、並べ替えは初回sliceへ含めない。

## 4. Edge詳細

Edge詳細の目的は、Edge登録と配下デバイスの状態確認である。

headerに次を表示する。

- Edge名
- 登録状態
- 最終通信
- デバイス数とセンサー数
- breadcrumb

未登録Edgeでは登録前データの扱いを説明し、adminに`Edgeを登録`を表示する。
登録処理中、登録済み、復旧確認待ちでは既存状態説明を表示する。

登録済みEdgeだけが配下デバイス一覧を表示する。デバイス一覧には次を表示する。

- デバイス名
- 設置場所
- 設定状態
- センサー数
- 最終受信
- `詳細を見る`

デバイスprofile formとセンサー情報は表示しない。Edge node ID、ledger epoch、最終登録結果は
折りたたみ診断欄へ置く。

## 5. デバイス詳細

デバイス詳細の目的は、一台の物理デバイスとそのセンサーを照合・設定することである。

headerに次を表示する。

- デバイス名
- 設置場所
- 所属Edgeへのbreadcrumb
- 設定状態
- 最終受信
- 現物との照合番号。admin以上だけ

デバイス名と設置場所のformは一つの設定panelに置く。未設定なら開き、
設定済みなら折りたたむ。

センサーは横断階層へさらに入れ子にせず、同じ深さの一覧として表示する。
各sensor row/cardは次を持つ。

- センサー名
- 種類
- 現在値
- 最終受信
- 基本設定状態
- `基本情報を編集`

編集対象のセンサーだけformを展開する。Adapter由来のmeasurement key、値型、canonical unit、
channelは編集panel内の`Adapterから届いた情報`に置く。値の変換は別画面へのlinkとし、
この画面へsemantic formを置かない。

## 6. Visual hierarchy

- page headerの直下にbreadcrumbを置き、英語のsection kickerを主情報にしない。
- primary informationは名前、状態、現在値、次の操作とする。
- cardの入れ子は最大一段とする。
- 診断情報は通常閉じる。
- formは設定操作を開始した対象だけ開く。
- 大きな空白と巨大な縦cardを減らし、一覧の走査性を優先する。
- 既存PinkieTechの濃紺、orange、tealを維持する。
- 状態は色だけでなく日本語labelを付ける。

## 7. Data and application boundary

新しいDB table、API、MQTT contractは追加しない。既存の次を利用する。

- `ListEdges`
- `ListSetupDevices`
- Edge activation operation
- Device profile operation
- Signal profile operation

HTTP view modelは既存一覧を`edge_ref`または`device_ref`で選択する。該当しないpublic refは404とする。
内部`edge_node_id`、`system_id`、`series_key`をURLへ使わない。

mutation後の戻り先は、同じEdge詳細またはデバイス詳細とする。`return_to`は
`/equipment/edges/{public_ref}`と`/equipment/devices/{public_ref}`だけを安全な相対pathとして許可し、
scheme、host、query、fragment、追加slashを受理しない。

## 8. Permissions and errors

- viewerは全詳細と現在値を閲覧できるが、formとidentifierを見られない。
- admin以上がEdge登録、device profile、signal profileを変更できる。
- revision、CSRF、Origin、auditは既存mutation pathを維持する。
- 未登録Edgeの配下デバイス設定は表示しない。
- orphan deviceはEdge一覧の警告から専用隔離sectionへ誘導し、hard deleteしない。
- 対象refが存在しない場合は空の正常画面にせず404を返す。
- 一覧取得失敗は500とし、不完全な正常表示をしない。

## 9. Verification

- `/equipment`はEdge summaryだけを表示し、device/sensor formを表示しない。
- Edge詳細は選択したEdgeだけを表示する。
- 未登録Edge詳細はactivation操作だけを表示し、device formを表示しない。
- 登録済みEdge詳細はdevice summaryを表示し、sensor formを表示しない。
- デバイス詳細は選択したdeviceとそのsensorだけを表示する。
- viewerにはmutation formとidentifierを表示しない。
- dynamic `return_to`は正規equipment pathだけを受理する。
- 不明なEdge/device refは404を返す。
- 既存のactivation、profile、monitor、semantic、output testを維持する。
- desktop 1440px、mobile 390pxで実レンダリングを確認する。

## 10. 対象外

- Edge profileの新しい編集operation
- fleet検索、並べ替え、pagination
- JavaScript SPAとclient-side router
- Broker、certificate、credential設定
- semantic evaluator、raw custody、MQTT contract変更
- camera、barcode、notification
