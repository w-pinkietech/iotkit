# IoTKit Console レスポンシブレイアウト設計

Date: 2026-07-28

Status: Approved

Issue: [#105](https://github.com/w-pinkietech/iotkit/issues/105)

## 1. 目的

IoTKit Consoleの主要画面を、スマートフォンとタブレットで横溢れや
操作不能なしに利用できるようにする。

対象幅は次とする。

- スマートフォン: 360pxから430px
- タブレット: 768pxから1024px
- desktop: 1024pxを超える幅

端末区分とlayout modeは同じ境界ではない。961pxから1024pxはdesktop modeを
使うtablet landscapeまたは狭いdesktopの回帰帯として検証する。ブラウザの拡大、
OSの文字倍率、端末固有のsafe areaを別のlayout modeとしては扱わない。

## 2. 現状と再現結果

2026-07-28に実Consoleを390px、768px、781px、1024pxで描画し、
概要、センサー一覧、受信履歴、機器管理、外部出力、変更履歴、
アカウント、システムの8画面を確認した。

次の問題を再現した。

- 780px以下ではモバイルヘッダーとメニューボタンが表示されるが、
  `initializeMenu`が必須とする`.mobile-overlay`がtemplateに存在しない。
  そのためクリック処理が登録されず、非表示のサイドバーを開けない。
- 390pxの概要画面では現在値tableがdocument幅を超える。
- 390pxのアカウント画面では更新・password・無効化formを含むtableが
  document幅を超える。
- 781pxではdesktopサイドバーが残る一方、main contentの有効幅が足りず、
  受信履歴のtoolbarがdocument幅を超え、機器管理の行がclipされる。
- 既存browser journeyの390px検証は外部出力画面に限られ、共通shell、
  menu操作、他の主要画面を回帰検出できない。

## 3. 設計判断

既存のSSR template、CSS、TypeScript shellを維持し、問題のある共通境界と
画面だけを修正する。全画面を新しいcomponent systemへ作り直さない。

検討した案は次のとおり。

1. **既存UIを活かした重点修正（採用）**
   - 共通shell、breakpoint、狭幅で溢れる画面、回帰testを修正する。
   - desktopの視覚構成とserver-side read modelを維持できる。
2. **CSSだけの最小修正（不採用）**
   - 横scroll領域を追加するだけでは、概要やアカウントの操作性が悪いまま残る。
   - menuの状態、focus、ARIAを検証できない。
3. **mobile-first全面再設計（不採用）**
   - 長期的な自由度は高いが、既存8画面の情報構造を同時に変えるため
     Issue #105の範囲と回帰riskを超える。

## 4. 共通shell

### 4.1 layout mode

960px以下をcompact modeとし、961px以上をdesktop modeとする。

- compact modeでは`.console-shell`を単一columnにする。
- サイドバーは画面外に置くdrawerとし、メニューボタンで開く。
- desktop modeでは現行の256px固定sidebarとmain contentを維持する。
- 460px以下の既存compact spacingは維持する。

960pxを境界にする理由は、781pxで再現した「desktop sidebarを残したため
main contentの最小幅を満たせない」区間をなくし、1024pxでは現行desktop
構成を維持するためである。

### 4.2 drawerの状態

`body.menu-open`をdrawerの唯一の表示状態とする。

- menu button click: drawerを開閉する。
- overlay click: drawerを閉じる。
- Escape: drawerを閉じる。
- navigation link click: drawerを閉じてから通常の遷移を続ける。
- viewportがdesktop modeへ変わった場合: drawer状態を解除する。

templateへ`.mobile-overlay`を1つ追加する。初期状態は`hidden`とし、
drawerが開いたときだけ表示する。overlayはsidebarより背面、
main contentより前面に置く。

### 4.3 accessibility

- menu buttonは`aria-controls="sidebar"`を維持する。
- 閉じた状態は`aria-expanded="false"`、開いた状態は`true`とする。
- 開いたときはsidebar内の現在page link、なければ最初のnavigation linkへ
  focusを移す。
- overlayまたはEscapeで閉じたときはmenu buttonへfocusを戻す。
- `prefers-reduced-motion`ではdrawer transitionを実質無効にする。
- JavaScript初期化時に必要な要素が欠けている場合は例外を投げずno-opとする。
  正常templateに必要要素が存在することはcontract testで固定する。

## 5. 画面別layout

### 5.1 概要

概要の現在値一覧は、compact modeでtable headerを隠し、センサーごとの
stacked rowへ切り替える。

各rowは次を保持する。

- センサー名
- 現在値
- 受信状態
- 収集ノード

列を削除したり、値を省略したりしない。document全体の横scrollを発生させず、
長いセンサー名と収集ノード名はwrapできるようにする。

### 5.2 センサー一覧と機器管理

既存のcompact card layoutを維持する。breakpointを960pxへ揃え、
781px付近でdesktop rowがclipされる区間をなくす。

名前はellipsisを許可するが、状態、件数、現在値、navigation actionを
画面外へ隠してはならない。

### 5.3 受信履歴と変更履歴

履歴系tableは情報密度と列比較を維持するため、全列をcardへ変換しない。

- filter toolbarはcompact modeで1 columnにする。
- tableは`.table-wrap`内だけで横scrollを許可する。
- document自体の横scrollは禁止する。
- scroll領域の存在を視覚的に妨げないよう、containerのborderと余白を維持する。

### 5.4 アカウント

compact modeではaccount tableを行単位のcard layoutへ変換する。
各accountのidentity、role、状態、更新、password変更、無効化を同じcard内に
stackする。

form controlとbuttonは利用可能幅を超えず、主要buttonは幅100%とする。
操作のHTTP method、CSRF、確認message、権限条件は変更しない。

### 5.5 外部出力とシステム

既存の390px layoutを維持し、共通breakpointを960pxへ揃えた結果だけを
回帰確認する。出力状態、technical details、storage factsの意味や表示順は
変更しない。

## 6. data flowとerror境界

この変更はpresentationだけに限定する。

- HTTP route、request/response、session、CSRF、authorizationを変更しない。
- server-side read model、MQTT、storage、semantic projectionを変更しない。
- form field名、action、method、mutation dispatcherを変更しない。
- 表示内容や件数をviewportによって変えない。

JavaScriptはmenuの一時的な表示状態だけを所有し、永続化しない。
画面遷移後は閉じた状態から始める。通信失敗やserver errorの扱いは現行の
SSR error表示を維持する。

## 7. test

### 7.1 frontend unit

`edge/frontend/tests/unit/shell.test.ts`へ次を追加する。

- menu buttonでopen/closeし、`aria-expanded`とoverlay状態が同期する。
- overlay clickとEscapeで閉じる。
- navigation clickで閉じる。
- desktop modeへ変わると状態を解除する。
- focusがdrawerへ入り、閉じるとbuttonへ戻る。
- 必要要素が欠けたfragmentでも例外を投げない。

### 7.2 template contract

Console shellにmenu button、`#sidebar`、`.mobile-overlay`が一組ずつ存在し、
生成assetが同期していることを既存Console contractへ追加する。

### 7.3 browser journey

owner sessionで次の8画面を390px、768px、1024pxで描画する。

- `/status`
- `/sensors`
- `/logs`
- `/equipment`
- `/output`
- `/audit`
- `/accounts`
- `/system`

各画面で
`document.documentElement.scrollWidth <= document.documentElement.clientWidth`
を要求する。受信履歴と変更履歴はtable自体がcontainerより広くてもよいが、
scrollが`.table-wrap`内に封じられていることを確認する。

document幅が収まっていても`overflow: hidden`で内容が切れている状態は成功としない。
`.table-wrap`内のtableを除き、主要cardの状態、値、件数、操作、navigationが
cardの表示領域内にあることをbounding boxで確認する。

390pxではmenu buttonからdrawerを開き、navigation、overlay、Escapeで
閉じられることを確認する。browser exceptionがないことも既存journeyの
検査へ含める。

## 8. verification

実装後は小さい順に次を実行する。

1. frontend unit test
2. Console template/HTTP focused test
3. `scripts/test-edge-console-frontend.sh`
4. `scripts/test-edge-console-e2e.sh`
5. `scripts/check-source-layout`
6. Rust product behaviorへの影響を除外できない場合は`scripts/verify.sh`

Issue #105はConsole HTML、CSS、browser behaviorを変更するため、
browser journeyの成功を完了条件とする。物理sensorやRaspberry Piは不要である。

## 9. 対象外

- desktop画面の全面的なvisual redesign
- 新しいfrontend frameworkやcomponent libraryの導入
- API、認証、data model、MQTT、storageの変更
- Console外の公開siteやnative application
- browser zoom、200% text、safe areaに対する独立したlayout mode
