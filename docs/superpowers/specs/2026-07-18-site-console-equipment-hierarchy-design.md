# IoTKit Site Console 機器管理階層化設計

Date: 2026-07-18
Status: Approved for implementation

## 1. 目的

Site Consoleの設置導線を、現場担当者が理解する物理構造に合わせる。

```text
Edge登録
  -> デバイス確認・登録
  -> センサー基本設定
  -> 現在値確認
```

現在の`Edge管理`、`デバイス管理`、`センサー設定`は追加時期が異なり、
同じ設置作業が複数画面へ分断されている。本変更ではEdge、デバイス、センサーを
親子関係として一つの「機器管理」画面へ統合する。

## 2. 利用者と成功条件

全画面はログイン必須とする。`viewer`は状態と受信値を閲覧でき、
`admin`と`system_admin`はEdge登録、デバイスprofile、センサーprofileを変更できる。

次を完了条件とする。

- 利用者がEdgeとデバイスを同じものだと誤認しない。
- 一つのEdgeに属するデバイスとセンサーを画面上で追跡できる。
- 未設定箇所と次に行う操作が一画面で分かる。
- 日常監視、基本設定、値の変換を混ぜない。
- Edge登録前の値がSiteの正式履歴へ入らないことが画面から分かる。
- 内部ID、credential、Broker接続設定を通常の操作面へ露出しない。

## 3. Navigation

利用者に見えるnavigationは次へ統一する。

```text
現場を見る
├ 概要
├ センサー
└ 受信履歴

設定
├ 機器管理
├ 値の変換
└ 外部出力

管理
├ 変更履歴
├ アカウント
└ システム
```

`/equipment`を機器管理の正規入口とする。既存の`/edges`、`/setup`、
`/devices`は互換性のため応答可能なまま残すが、navigationへ表示しない。
既存の`/signals`はURLを維持し、表示名だけを「センサー設定」から
「値の変換」へ変更する。

## 4. 機器管理画面

`/equipment`はEdgeを最上位とした階層を表示する。

```text
Edge
└ デバイス
   └ センサー
```

画面上部に次の設置Journeyを置く。

1. Edgeを登録
2. デバイス名と設置場所を登録
3. センサー名、種類、値の見せ方、単位を確認
4. 現在値画面で確認

Journeyは強制wizardではない。利用者は任意のEdgeまたはデバイスを開いて
状態確認と再設定を行える。

### 4.1 Edge

Edge行には次を表示する。

- 表示名。未設定時は`名前未設定のEdge`
- 設置場所。未設定時は`設置場所 未設定`
- 登録状態
- 配下デバイス数
- 配下センサー数
- 最終通信

登録状態は`未登録`、`登録処理中`、`登録済み`、`復旧確認待ち`とする。
`未登録`のEdgeだけにadmin向けの`Edgeを登録`操作を表示する。
操作前に「登録前の値はSiteへ保存されず、登録後の値から正式履歴が始まる」と明示する。

Edge node IDとledger epochは折りたたまれた診断情報にだけ表示する。
credential、ACL、Broker endpoint、certificate設定は表示・変更しない。

### 4.2 デバイス

Edge配下のデバイスごとに次を表示する。

- デバイス名
- 設置場所
- 設定状態
- 最終受信
- descriptor状態
- 配下センサー数

デバイスprofileが未完成なら入力欄を開いた状態にし、完成済みなら要約を先に表示する。
profile保存は既存revision保護、CSRF、role、auditを維持する。

### 4.3 センサー

デバイス配下のセンサーごとに次を表示する。

- センサー名
- センサー種類
- 現在のraw値
- 最終受信
- 設定状態
- Adapterが報告したmeasurement key、値型、canonical unit、channel
- 現場設定の名前、表示分類、値の見せ方、表示単位、表示桁数

Adapter由来の事実と現場設定を別領域に表示する。値だけから種類や単位を推測しない。
候補がある場合は入力初期値に使うが、担当者が保存するまで`利用可能`にしない。

補正、しきい値、ON/OFF判定、累積値はこの画面へ置かない。
デバイス内の全センサー基本設定が完了したら`値の変換`と`現在値を見る`への導線を出す。

## 5. 状態とデータ境界

設定状態と通信状態を別に表示する。

```text
Edge設定: 未登録 / 登録処理中 / 登録済み / 復旧確認待ち
デバイス設定: 未設定 / 登録済み
センサー設定: 未設定 / 利用可能
通信事実: 最終通信 / 最終受信 / 未受信 / descriptor状態
```

event-drivenなセンサーを一定時間無通信という理由だけで故障扱いしない。
liveness契約がない場合は最終受信日時だけを事実として表示する。

Edge登録前のmeasurementはSite raw archiveへ受理しない。Edge登録完了時に
Edge側の登録前ローカル値を削除し、登録後の最初の値からSiteの正式履歴を始める。
Edge登録後はデバイスまたはセンサーが未設定でもrawを保存する。

## 6. Application境界

新しい永続modelは追加しない。機器管理read modelは既存の次のtyped service結果を結合する。

- `ListEdges`
- `ListSetupDevices`
- Edge activation operation
- Device profile operation
- Signal profile operation

HTML handlerはStoreへ直接SQLを発行しない。結合処理はHTTP view modelだけを作り、
mutationのvalidation、revision、role、auditは既存application serviceへ委譲する。

deviceの所属先は`SetupDevice.Device.Edge`と`Edge.EdgeNodeID`の一致で決める。
対応するEdgeが見つからないlegacy deviceは破棄せず、
`所属するEdgeを確認できません`という隔離groupへ表示する。

## 7. Error behavior

- 一覧取得失敗: 画面全体を不完全な正常表示にせず、取得失敗として応答する。
- Edge登録失敗: 入力済み情報を失わせず、Edge状態と最終結果を表示する。
- revision競合: 再読み込みが必要な対象を示す。
- device保存失敗: 配下sensorの状態を変更したように見せない。
- sensor保存失敗: 同じdeviceの他sensorを未保存へ戻さない。
- descriptor情報不足: raw値と不足項目、現場で確認する次の操作を表示する。
- orphan device: hard deleteせず隔離groupに表示する。

## 8. Visual方針

既存PinkieTech brandの配色、server-rendered HTML、埋め込みCSS/JavaScriptを維持する。
SPAや新しいfrontend dependencyは追加しない。

- Edgeは外枠、deviceはその内側、sensorはtable/cardとして階層差を視覚化する。
- 常時すべてのformを表示せず、未設定項目を優先して開く。
- 現在値は大きく、内部識別子は診断欄へ小さく表示する。
- desktopを主対象とし、狭い画面では一列へ折り返す。
- 色だけに依存せず、状態labelと次の操作を文章で示す。

## 9. Verification

- Console testでnavigationが`機器管理`と`値の変換`に統一される。
- Console testで一つのEdge配下にdeviceとsensorが表示される。
- 未登録Edgeで登録操作と登録前データの説明が表示される。
- viewerにmutation formと物理照合identifierを表示しない。
- adminに既存activation、device profile、signal profile formを表示する。
- orphan deviceが隔離groupへ表示される。
- 既存のEdge activation、setup、monitor、semantic、output testを維持する。
- desktopとmobile幅で手動確認し、階層、form、empty state、error表示を確認する。

## 10. 対象外

- Edge表示名・設置場所を保存する新profile
- Edgeのdeactivation、reactivation、Site transfer
- Broker、credential、certificateのWeb設定
- semantic evaluatorの変更
- raw custody、activation wire contract、MQTT topicの変更
- camera、barcode、notification
- range chart、CSV export
