# Console Output Delivery Status Design

Date: 2026-07-27

Status: User-approved design

Issue: #101

## 1. Purpose

IoTKit Consoleの外部出力画面を、設定フォームの一覧ではなく、現場担当者が配送状態を
短時間で判断できる画面へ再構成する。

画面上部では「正常に送信中」「設定が必要」「配送に問題」の件数を示し、各出力先では
状態、送信対象数、最終送信、配送待ちを最初に見せる。センサー・意味づけルールの設定と
topic、payload、内部IDは必要なときだけ展開する。

この変更は既存のOutput Profile、binding、durable outbox、追加・設定・開始・停止の
application operationを維持する。Broker endpoint、credential、証明書、ACLの設定は
Consoleへ持ち込まない。

## 2. Authority and constraints

設計は次を正本とする。

- `docs/okf/ja/architecture/system-overview.md`
- `docs/superpowers/specs/2026-07-20-site-wide-output-profiles-design.md` 8章
- `docs/superpowers/specs/2026-07-18-site-console-operator-journey-design.md` 6.6–6.7

特に次を維持する。

- Output Adapterは決定的な変換器であり、Broker credentialやretry policyを所有しない。
- MQTT配送はdurable output outboxを通り、外部出力の失敗はEdge Nodeのcustodyを止めない。
- Web handlerは薄く保ち、型付きapplication operationとstorage read-modelを利用する。
- 通常画面では`profile`、`binding`、`route`、config schemaを利用者用語として表示しない。
- viewerは状態と技術情報を閲覧できるが、変更はadminまたはsystem adminだけが行う。

## 3. Scope

### 3.1 In scope

- `/output` 上部の配送状態集計
- 稼働中または停止処理中の外部出力先カード
- 出力先単位の状態、送信対象数、最終送信、配送待ち
- rule単位の送信状態、設定待ち、外部登録待ち
- topic、payload、source/signal IDのread-only技術情報
- 未追加Adapterを分離した追加導線
- admin向けの既存追加・設定・開始・停止操作
- viewer向けの完全なread-only表示
- desktopと狭幅画面の横スクロール防止
- 追加前、設定待ち、送信中を含むbrowser journey

### 3.2 Out of scope

- Broker endpoint、credential、CA、private key、ACLのConsole設定
- 停止済み出力先の再開
- 停止履歴一覧
- rule単位の任意除外toggle
- 既存mutation routeまたはform contractの再設計
- `/output` routeの名称変更
- 配送retry policyの変更

## 4. Chosen approach

サーバー側で配送状態専用のConsole read-modelを組み立て、Askama templateで
deterministicに描画する。

templateだけを並べ替える案は、最終送信とoutbox滞留を集計できないため採用しない。
ブラウザからbindingごとのpublication APIを呼ぶ案は、JavaScript無効時に状態が欠落し、
集計がN+1 requestになるため採用しない。専用公開APIを新設する案は将来の選択肢だが、
Issue #101の範囲では既存application/storage read-modelの拡張で十分である。

既存のbinding publication snapshotを基礎に、Console向けに次を合成する。

```text
storage delivery facts
  + output profile and binding state
  + semantic rule and sensor labels
  + adapter publication preview
  -> Console output delivery read-model
  -> server-rendered /output
```

## 5. Read-model

### 5.1 Page summary

`ConsoleOutputSummary`は次を持つ。

- `sending_count`
- `needs_configuration_count`
- `delivery_problem_count`

件数の単位は外部出力先であり、各出力先は優先状態に従って一つの区分だけへ入る。
未追加Adapterと停止済みprofileは集計対象外とする。

### 5.2 Destination card

稼働中の`ConsoleOutput`は少なくとも次を持つ。

- profile IDとadapter ID
- 利用者向け名称と平易なAdapter名称
- profile state
- 優先状態、表示label、表示class
- active target count
- needs-configuration count
- ineligible count
- pending delivery count
- oldest pending timestamp
- last published timestamp
- future rule auto-add permission
- rule/binding rows

最終送信はbindingの`last_published_at`の最大値、配送待ちは`pending_count`の合計、
最古の配送待ちは`oldest_pending_at`の最小値とする。

### 5.3 Rule row

`ConsoleBinding`は次を持つ。

- binding ID、rule ID、sensorへのlink
- sensor名、meaning rule名
- binding stateと利用者向けlabel
- mode選択または外部登録が必要か
- topic、payload、preview provenance
- source ID、signal ID
- pending count、last published timestamp
- 変換またはpreview取得失敗の安全な表示

secretはread-modelへ含めない。payload previewは完全なJSONを表示するが、
実outbox、最新Observationからのpreview、sampleのどれかを明示する。

## 6. Status classification

### 6.1 Binding status

bindingは次の順で分類する。

1. 変換エラー
2. 設定が必要
3. 配送停止の可能性
4. 外部登録待ち
5. 配送中
6. 送信済み
7. 最初の値を待っています
8. 対象外

pendingが存在し、最古のpendingが現在時刻から5分以上前なら
「配送停止の可能性」とする。5分未満のpendingは異常ではなく「配送中」とする。

### 6.2 Destination status

出力先の優先状態は次の順とする。

1. 設定が必要、または変換エラー
2. 一つ以上のbindingで配送停止の可能性
3. 外部登録待ち
4. 停止処理中
5. 正常に送信中

上位二つをそれぞれpage summaryの「設定が必要」「配送に問題」へ入れる。
外部登録待ちは利用者の操作が必要なので「設定が必要」へ入れる。
停止処理中は異常ではなく「正常に送信中」区分へ含め、カードのlabelで区別する。

一つのbindingの失敗で他のbindingを停止扱いにしない。出力先カードは問題を優先表示するが、
各rule rowでは正常配送中の値も個別に示す。

## 7. Page structure

ページは次の順で描画する。

1. viewer向けread-only案内
2. 三つの配送状態summary
3. 稼働中の外部出力先カード
4. 未追加Adapterの「新しい出力先を追加」

外部出力先カードは次の階層にする。

```text
名称 + 優先状態
Adapterの平易な説明
状態 / 送信対象 / 最終送信 / 配送待ち
注意が必要な短い案内
送信する値
  └─ sensor / rule / 個別状態 / 必要な設定操作
技術情報（折りたたみ）
  └─ topic / payload / IDs / preview provenance
送信停止（折りたたみ）
```

停止済みprofileは通常画面に表示しない。同じAdapterのactiveまたはdraining profileが
なければ、未追加Adapterとして再度追加できる。

## 8. Mutation hierarchy and permissions

既存routeとCSRF、authorization、revision、auditの境界を維持する。

- 追加前: 「この内容で送信を開始」をprimary actionとする。
- 設定待ち: 必要なmode選択をrule row内で行う。
- 外部登録待ち: 明示checkboxと「送信開始」を一つのまとまりにする。
- 送信中: 通常は状態確認だけを見せる。
- 停止: カード末尾の折りたたみ領域に置き、停止境界を説明する。

停止説明は次の事実を含む。

- 停止後に受けた値はこの出力先へ送られない。
- 停止前に配送対象になった値はoutboxから配送を続ける。
- 停止した出力先は再開せず、必要なら同じAdapterを新しく追加する。

viewerにはsummary、出力先カード、rule state、topic、payload、IDを表示する。
追加・設定・開始・停止formは一切描画しない。

## 9. Responsive and accessibility behavior

desktopでは状態factsを横並び、ruleを表形式に近い行として表示する。
780px以下ではrule tableのheaderを隠し、各rowをlabel付きの縦積みcardへ変換する。
390px幅でもpage、card、technical previewに水平スクロールを発生させない。

topicとpayloadは`overflow-wrap`と`pre-wrap`を使い、card幅を押し広げない。
copy操作はbutton labelを持ち、JavaScriptがなくても全文を選択できる。
状態は色だけに依存せず、常に日本語labelと説明を表示する。
summaryとcard headingには適切なsection headingとaccessible nameを付ける。

## 10. Failure handling

- binding publication previewが失敗してもページ全体を500にしない。
- 該当bindingだけを「変換エラー」とし、安全な短い理由を表示する。
- storageから配送factsを取得できない場合は出力先を正常扱いにせず、
  状態を確認できない旨を表示する。
- ineligibleは障害ではなく「対象外」として個別表示する。
- last publishedがない場合は「まだ送信されていません」と表示する。
- pendingが0の場合は「なし」と表示する。

## 11. Verification

### 11.1 Rust tests

- binding delivery stateと5分境界
- destination priorityとpage summaryの相互排他集計
- pending合計、oldest pending、last published集約
- stopped profile非表示と同Adapter再追加
- preview失敗の局所化
- viewer/adminのtemplate表示差

### 11.2 Browser journeys

- 追加前: summaryが0で、adminだけに追加actionがある。
- 設定待ち: 出力先が「設定が必要」に入り、対象ruleだけに設定actionがある。
- 外部登録待ち: 登録確認と送信開始の優先順位が明確である。
- 送信中: 対象数、最終送信、配送待ち、技術情報が確認できる。
- viewer: 同じ状態と技術情報を見られるがmutation formがない。
- desktopと390px幅で`scrollWidth <= clientWidth`を満たす。

既存のConsole browser journeyとRust Console integration journeyも維持する。

## 12. Delivery

変更はIssue #101専用branchとDraft PRで提出する。CI、Rust tests、Clippy、
frontend unit tests、browser journeyが成功した状態で人のreviewへ止める。
PRのmergeやreleaseは別途明示承認を必要とする。
