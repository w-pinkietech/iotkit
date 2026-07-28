# IoTKit Console センサールール選択・プレビュー設計

Date: 2026-07-28

Status: Approved

Issue: [#89](https://github.com/w-pinkietech/iotkit/issues/89)

## 1. 目的

センサー詳細で、operatorが現在編集または作成している計測ルール・累積ルール・
異常検知ルールと、右側の実信号プレビューを常に一致させる。

受信した現在値は入力の事実として残し、その下に選択中ルールの結果を独立して
表示する。ON/OFF、累積値、正常/異常を色だけに依存せず文章でも判断できる状態を
完了条件とする。

## 2. 現状と再現結果

2026-07-28にproduction `iotkit-edge serve`と`console_fixture`をWSL上で起動し、
実Consoleから温度、照度、接点入力の設定journeyを操作した。

次の問題を再現した。

- 異常検知ルールがない照度センサーで「異常検知」tabを開くと、左側は空状態を
  示す一方、右側は非表示の「計測ルール」tabにある累積値を表示する。
- 異常検知ルールの作成入口は「計測ルール」tabの「値の使い方を追加」にしかなく、
  空の「異常検知」tabから作成できない。
- 作成formで「異常検知」を選んでも、右側は直前に開いていた保存済みルールの
  結果を表示し続ける。
- 作成formの説明とfield名が、異常検知を選んだ後も「測定値」や
  「有効とみなす側」のまま残る。
- 接点入力で累積ルールを選んでも、プレビュー上部は入力の`ON`だけを
  「現在の値」として表示し、選択ルールの累積値を同じ重要度で示さない。
- graphの読み上げ用要約は補正後の数値範囲だけを説明し、ON/OFF、累積値、
  正常/異常を含まない。

現行frontendでは、全tabを横断して最初の`details.semantic-rule-card`を開き、
開いているcardの`rule_id`をプレビュー対象にしている。tab切替はpanelの
`hidden`だけを更新し、プレビューへ選択変更を通知しない。新規作成formは
rule cardではないため選択候補にもならない。これらが、非表示ルールへの
fallbackと作成draftの無視を生む根本原因である。

## 3. 設計判断

既存の複数ルールpreview API、SSR template、TypeScript frontendを維持し、
frontendに明示的なpreview targetを導入する。

検討した案は次のとおり。

1. **選択同期方式（採用）**
   - tab、保存済みrule card、新規作成formを一つの選択状態として扱う。
   - 右側を「受信した現在値」と「選択中ルールの結果」に分ける。
   - 現行APIの`rule_id`、`display_name`、`kind`、`points`を再利用できる。
2. **ルール別preview方式（不採用）**
   - 各rule card内にもpreviewを置くと比較は容易になるが、同じ受信履歴と
     graphを複数箇所で更新するため情報量とruntime処理が増える。
3. **tab別preview component方式（不採用）**
   - 計測ルールと異常検知に別々の右panelを持たせると明確だが、
     graph、polling、error状態が重複する。

## 4. 画面構造

### 4.1 受信値と選択ルール結果

右側の`実信号プレビュー`は次の二つを明確に分ける。

1. **受信した現在値**
   - Edge Nodeから最後に届いたraw入力を、センサーprofileの表示形式で示す。
   - 最終受信時刻と受信中・停止中の状態を維持する。
2. **選択中ルールの結果**
   - rule名とkindを表示する。
   - `numeric`: 補正後の数値と表示単位。
   - `boolean`: `ON`または`OFF`。
   - `cumulative_counter`: `累積 N`と、最新入力での増分がある場合は`今回 +N`。
   - `alarm`: `正常`または`異常`。

rule結果には文章を必須とし、色やgraphのactive bandだけで状態を伝えない。
graphはraw入力、補正値、threshold、active band、累積線を現行どおり使うが、
凡例とsummaryは選択中kindに合わせる。

### 4.2 計測ルールtab

「値の使い方を追加」はこのtabに残し、作成候補を次に限定する。

- 測定値
- ON / OFF
- 累積値

保存済みrule cardと新規作成disclosureは、同じtab内で一つだけ開ける。
開いた対象がpreview targetになる。

### 4.3 異常検知tab

異常検知専用の「異常検知を追加」disclosureを配置する。作成formは
`kind=alarm`を固定し、次の専用文言を使う。

- 異常とみなす側
- 異常になるしきい値
- 正常に戻るしきい値
- 異常確定待ち
- 復帰確定待ち

ruleがない場合も、別tabへ誘導せず、このdisclosureから作成できる。
disclosureを開くまでは「選択中のルールはありません」と表示し、
別tabの結果へfallbackしない。

## 5. preview targetとdata flow

### 5.1 target identity

各semantic formはpreview requestで使う安定したtarget IDを持つ。

- 保存済みrule: 現行`rule_id`
- 計測ルールdraft: page内で安定した`draft-normal`
- 異常検知draft: page内で安定した`draft-alarm`

`buildRequest`はformのtarget IDを、そのdraftを含むrequestの`rule_id`へ使う。
配列indexからdraft IDを推測しない。

### 5.2 tab選択

`initializeSettingTabs`はpanel表示とURLの`tab` query更新に加え、tab root上で
内部の選択変更eventを通知する。previewは初期化時に現在表示中panelを読み、
以後このeventとdisclosureの`toggle`を監視する。

各tabは自身の最後に開いたtargetを保持できる。rule cardの相互排他は全pageでは
なく同じtab panel内だけに限定する。tabを切り替えたときは表示中panelの
開いたtargetだけを採用し、非表示panelの開いたcardを参照しない。

### 5.3 response選択

preview requestは、calibrationと保存済み・draftの全rule候補を現行どおり
一度に評価する。semantic overlayと結果表示には、active target IDと一致する
responseだけを使う。

active targetがない場合でも、成功したresponseのraw入力をgraphの基礎に利用
してよい。ただし補正線、threshold、active band、累積線、rule結果は表示せず、
raw-only状態として描画する。別ruleのsemantic結果を代用しない。

active targetがresponseにない、またはそのtargetだけがerrorの場合も、
別ruleへfallbackしない。

## 6. errorと空状態

- target未選択: raw値を維持し、「選択中のルールはありません」と表示する。
- 受信履歴なし: rule名を維持し、結果を「受信待ち」とする。試す値は利用できる。
- draft validation失敗: 該当field近くにerrorを表示し、rule結果を
  「設定内容を確認してください」とする。
- selected targetのpreview error: raw値は維持し、rule結果だけを
  「判定結果を更新できません」とする。
- request全体の通信失敗: 最終受信値は維持し、rule結果に更新失敗を示す。
- 別ruleの最後の成功結果を、現在の選択結果として表示しない。

保存、retire、counter reset、CSRF、authorization、revision conflictの
server-side処理は変更しない。

## 7. accessibility

読み上げ用summaryは、raw範囲と補正範囲に加え次を含める。

- 選択rule名とkind
- 最新の補正値
- booleanの`ON`または`OFF`
- cumulative counterの最新累積値
- alarmの`正常`または`異常`
- 表示した受信件数

選択変更後は、既存の`role=status`領域で結果が変わったことを通知する。
tabのARIA、keyboard操作、focus管理は現行を維持する。作成disclosureとfieldには
visible labelを持たせ、placeholderだけを説明に使わない。

## 8. test

### 8.1 frontend unit

`edge/frontend/tests/unit/preview.test.ts`と関連unit testへ次を追加する。

- tab切替で、表示中panelの開いたruleだけをpreview targetにする。
- 空の異常検知tabで、非表示の累積ruleへfallbackしない。
- 計測ルールdraftと異常検知draftが、それぞれ安定したIDでrequest・responseに
  対応する。
- create disclosureと保存済みcardが同じpanel内で排他的に開く。
- selected targetがerrorでも別ruleへfallbackしない。
- numeric、boolean、cumulative counter、alarmの結果文を表示する。
- 読み上げsummaryがrule名、ON/OFF、累積値、正常/異常を含む。
- request失敗時にraw値を残し、rule結果だけを失敗状態にする。

`edge/frontend/tests/unit/shell.test.ts`では、tab選択eventと既存URL・ARIA・
keyboard挙動が共存することを固定する。

### 8.2 Rust Console contract

`edge/tests/console_contract.rs`へ次を追加する。

- 計測ルール作成formに`alarm`候補がない。
- 異常検知tabに専用作成formと安定したdraft target IDがある。
- 異常検知formが専用labelを使う。
- previewに受信値とrule結果の独立した表示hookがある。

HTTP route、method、field名、typed application operationは現行のcontractを
維持する。

### 8.3 実browser journey

production `iotkit-edge serve`とfixtureを使い、次を確認する。

- 温度: 保存済み高温alarmを開き、最新結果が`正常`または`異常`と表示される。
- 照度: 空の異常検知tabが累積結果を表示せず、同じtabでalarm draftを作成・
  preview・保存できる。
- 接点入力: booleanとcumulative ruleを切り替え、`ON/OFF`と`累積 N`が
  選択へ追従する。
- 保存後のredirectとreloadで、tab、rule card、preview targetが一致する。
- browser exceptionがなく、読み上げsummaryにも選択結果が含まれる。

## 9. verification

実装後は小さい順に次を実行する。

1. `npm run check --prefix edge/frontend`
2. `cargo test -p iotkit-edge --test console_contract`
3. `scripts/test-edge-console-frontend.sh`
4. `scripts/test-edge-console-e2e.sh`
5. `scripts/check-source-layout`
6. `node scripts/battle-tested-review.mjs select --base origin/master`
7. Rust product behaviorへの影響を除外できない場合は`scripts/verify.sh`

Windows native Rust buildは既存のUnix permission APIにより成立しないため、
RustとbrowserのverificationはWSLまたはLinux devcontainerで実行する。

## 10. 対象外

- mapping previewのpublic OpenAPI schema変更
- semantic evaluator、storage schema、MQTT、output custodyの変更
- rule kindの追加
- 複数ruleの同時比較UI
- センサー一時停止・再開
- Console全体のnavigationやvisual redesign
- mobile専用UI
