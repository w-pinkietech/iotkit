# D3: プロセス決定とWave分割

Status: 確定 (2026-07-13、現在の実装ゲートを追記)

## 現在のプロダクト中心価値

> IoTKitは、さまざまなセンサーを小さなadapterで簡単につなぎ、IoTKit Edgeで現場データを
> 黙って失うことなく収集・正規化・保全・再送し、IoTKit Siteへ届けるIoTデータ収集基盤である。

この価値は次の2点の組み合わせにある。

1. **adapterの作りやすさ**: adapter作者はセンサー固有の通信、設定、値の読み取り、measurementへの
   写像だけを担当する。SQLite、MQTT、再送、保管責任、retention、認証をadapterへ持ち込まない。
2. **保管責任の確実な引き渡し**: Edgeは観測を耐久保存し、Siteがraw recordを耐久保存したと確認できる
   まで保持する。MQTT PUBACKだけではEdgeの保管責任を解除しない。

IoTKitは汎用IoTプラットフォーム、ダッシュボード、生産管理、独自MQTT brokerではない。R1〜R23と
Wave 1/2は将来の責務地図であり、現在すべてを実装するバックログではない。

## 現在の実装ゲート: 最初の実機縦切り

Wave 1の残項目を横に広げる前に、次の1本を完成させる。

```text
Temperature sensor
  -> BravePI Transmitter
  -> BLE Long Range
  -> BravePI Mainboard
  -> UART
  -> IoTKit Edge on Raspberry Pi
  -> MQTT QoS 1
  -> MQTT Broker
  -> IoTKit Site
  -> raw SQLite
  -> direct CLI query
```

### 作るもの

- BravePI MainboardのUARTを受動的に読む既存adapterと、温度frameのdecode。手元の機体は
  sensor type 261(MCP9600熱電対温度)と仮定して開始し、最初の実frameで型を確認する
- `device_number`を安定したhardware identityとして扱い、RSSIとbatteryを観測へ引き継ぐ経路
- Edgeでのcanonical record化、readingとoutboxのatomic保存
- EdgeのMQTT publish、`accepted-through`検証、cursor更新、保管責任に基づくretention
- Mosquitto等のMQTT Brokerと、Edge Nodeごとのstatic credential/topic ACL
- Siteでのraw recordと連続cursorのatomic保存、commit後の`accepted-through` publish
- 重複再送の冪等処理と、同一identity・異内容のconflict検出
- Site raw dataの直接CLI query
- Dockerによる開発・障害試験環境と、Raspberry Pi上のsystemd実行

### 完了条件

- ペアリング済みBravePI Transmitter 1台の温度観測が、UART経由でEdgeとSiteの双方に記録される。
- 実frameからsensor typeとpayloadを確認し、仮定と異なる場合も正しいdriverへ明示的に対応付ける。
- `device_number`、RSSI、battery、温度値が期待どおりdecode・保存される。
- EdgeまたはBravePI Mainboardの再起動後、UART受信が自動的に復帰する。
- 保持容量内では、Site、Broker、ネットワークの停止中もEdgeが収集を継続し、復旧後に欠けなく再送する。
- 同じbatchの再送でSiteのraw rowが重複せず、異内容ならconflictとしてcursorを進めない。
- SiteのSQLite commit失敗時は`accepted-through`を返さない。
- MQTT PUBACKだけではEdge cursorやpurge eligibilityが進まず、検証済み`accepted-through`だけで進む。
- Edge、Broker、Siteを個別に再起動しても同じ状態へ収束する。
- Docker試験に加え、Raspberry Piと実センサーで一連の流れを再現できる。

### 実機検証状況 (2026-07-13)

Raspberry Pi (Debian 13 / arm64) とペアリング済みBravePI実機で、現在のEdgeから
Siteまでの縦経路を確認した。

- `/dev/serial0` (`ttyAMA0`, 38400 8N1) から既存POCで、温度センサー
  `246880020140018b` と接点入力センサー `246880020140018c` のframeをdecodeした。
  温度、接点の0/1、RSSI、batteryが取得でき、decode errorは観測されなかった。
- 温度センサーはEdge上で `hardware_id = ble:246880020140018b`、
  `measurement_key = temperature_c` へ正規化された。未登録状態の45秒間に4観測が
  sightingと`staged_readings`へ保存され、温度25.5625〜26.1875°C、RSSI -68〜-66 dBm、
  battery 100%を確認した。
- CLIでsightingを承認してdeviceをactive化した後、次の45秒間に4観測が非検疫の
  `readings`へ保存され、対応する4行が`publication_log`へ同じ順序で作られた。
  温度は26.0〜26.3125°Cだった。
- 停止後のSQLiteに`PRAGMA quick_check`を実行し`ok`を確認した。試験終了後はEdgeが
  停止し、UARTが解放されることも確認した。
- 同じPi上の独立したDocker環境で、認証・topic ACL付きMosquittoとIoTKit Siteを起動した。
  Edgeに保持されていた`pub_seq` 1〜8をMQTT QoS 1で送り、Site raw SQLiteの8行と、
  Site commit後の`accepted-through = 8`によるEdge cursor更新を確認した。
- UART収集とMQTT出口を同時に35秒動かし、新しい温度3観測が`pub_seq` 9〜11としてSiteへ
  保存され、Edge cursorも11へ進んだ。Site CLIからraw recordを直接照会できた。
- broker再起動時、Siteが再接続後にrecords topicを再購読しない不具合を実機で発見した。
  購読をPahoの接続callbackへ移し、broker再起動をend-to-end試験へ追加した。修正版Siteは
  containerを再起動せず再購読し、その後の実機配送と`accepted-through`を完了した。

2026-07-14にはEdge/Site命名変更後のcommit `c0826a0`を使い、実験用labを旧DB・旧設定・旧MQTT
credentialごと削除して新構成から再作成した。新しい`edge_node_id`と専用credential/topic ACLを生成し、
温度センサー`ble:246880020140018b`を新DBで承認・active化した。BravePI MainboardのUARTから受信した
新しい温度観測11件が`pub_seq` 1〜11としてIoTKit Siteの11行へ同じ`edge_node_id`・`ledger_epoch`で
保存され、Edgeの`accepted-through` cursorも11へ収束した。停止後のEdge DBとSite DBはともに
`PRAGMA quick_check = ok`であり、Edge停止後にUARTが解放されたことも確認した。破棄したpre-releaseの
identity key、topic namespace、credentialは再利用していない。

これにより `BravePI Transmitter -> BLE Long Range -> BravePI Mainboard -> UART -> IoTKit Edge SQLite -> MQTT
-> MQTT Broker -> IoTKit Site raw SQLite -> accepted-through -> Edge cursor` は実機確認済みとなった。
平文MQTTは同一Piのloopbackだけを使う実験設定であり、実運用のTLS要件を緩和しない。
SQLite commit失敗、内容conflict、全コンポーネントの再起動組合せ、長時間停止中の連続収集は
引き続き現在の実装ゲートの未完了条件である。接点入力はUART decodeまでの確認であり、
Edgeへの通常取り込みは温度経路の次に行う。

実験用Piでの初回native release buildは、空のbuild cacheからRust toolchainと依存crateを
最適化したため一時的にCPUを飽和させ、SSH応答も遅くなった。日常の実機反復はdebug buildを
使い、正式配置用release binaryは開発機またはCIでarm64向けに作ることを優先する。

### このゲートでは作らないもの

- BravePIのBLE、ペアリング、トランスミッタ管理、送信間隔/出力設定、Long Range到達距離の再検証
- BravePIへの取得要求、downlink command、接点出力
- YokaKit連携、UI、dashboard、Site projection、cloud/fleet管理
- 第三者デバイスingress、pairing、オンボーディングUI
- credential enrollment/rotation、multi-Edge運用、MQTT Broker HA
- Site backup/restore、archive repair、汎用fan-out
- calibration UI、local rule、通知、南向きcommand/DFU
- 汎用adapter SDK、adapter code generator、宣言型driver DSL

既に実装済みのHTTP ingress、control API、operation catalog等は削除しないが、このゲートの完了条件から外し、
必要な保守以外の機能追加を止める。既存のrpi-local/OPT3001経路も維持するが、このゲートの完了条件には
含めない。最初の実機縦切り後、手元のBravePI接点入力センサーを2種類目として実機確認し、その実績から
adapter templateやSDKの必要性を判断する。

### BravePIとの責任境界

- センサー、トランスミッタ、BLE Long Range、ペアリング、メインボード上の端末管理はBravePIの責任とする。
- ペアリングは既存iOS applicationで行う。IoTKitにpairing UIや設定経路を作らない。
- ペアリング済み端末のデータはメインボードからUARTへ自動送出される前提とする。
- IoTKitの責任はUART streamの受信から始まり、frame decode、正規化、耐久保存、Siteへの引き渡しを担う。

以下のWave分割は長期ロードマップと既存決定の参照用に維持する。現在の実装順は上記ゲートを優先する。

## 決定1: 3段階Wave分割を採用

- **Wave 0「動く最小」**(目安1〜2ヶ月): 自社一号現場に置いて実データ収集を開始する。
  一号現場はプロセス内アダプタ2種(BravePI UART / I2C直結)のみ=第三者デバイス向け装備は実装しない。
- **Wave 1「他人に配れる」**: HTTP/MQTT ingress、トークン+TLS、オンボーディング/検疫UI、出口契約実装、
  操作カタログ+dry-run、desired/reported、スナップショット自動退避。
- **Wave 2「OSSとして公開」**: クライアントライブラリ3種+接続ガイド、A/B更新+OSイメージ、
  インシデントバンドル+AIハーネス統合、カメラsidecar、振動契約実装。

**読み替え規則**: 各文書の「第一波必須」は「**契約定義v1に含める**」と「**Wave 0で実装する**」の2つに分離して読む。
契約文書は最初から本番形で書き、実装だけを削る。

## 決定2: 契約の「凍結」を「安定意図(stable-intent)」に格下げ

最初の外部消費者または外部デバイス作者が現れるまで、D1の凍結リストは
「破壊的変更には移行ノートを書く」規律に緩める。買い手ゼロの時点での完全凍結は負債。

## 決定3: YokaKit再設計の凍結

yokakit-next(Go+Vue、ほぼ完成)は**現状のままリファレンス消費者**として使い、
ゼロからの再設計はIoTKit Wave 1出荷後に判断する。
例外: **機能カタログ抽出**(現場価値の棚卸し)だけは出口契約設計の入力として並行実施可。

## 決定4: 「第ゼロ波」(レガシー環境でのAI診断実証)は見送り

価値は認めるが現実的でないと判断。柱3の価値検証は、Wave 1でAIハーネスを開発する際に
自社現場(Wave 0稼働中のEdge)に対して行う。

## 決定5: 制御プレーンの認証方式

**サーバー側TLS(自己署名+フィンガープリントピン留め)+ operatorトークン**に統一。
mTLSは不採用(取り込み面のper-device mTLS不採用、CA基盤を作らない決定と整合)。
ai-connectivity図のmTLS表記は本決定で上書き。
※**2026-07-12 Plan 6改訂**: D13由来のsetup-mode例外（未認証の閉集合）は廃止。
未所有中はnetwork API/UIをbindせず、local `iotkit-edgectl`で所有権確立後にoperatorトークン認証へ入る。

## 設計課題キュー(レビューで確定した優先順)

0. ~~BravePI device_number焼き込み/設定可能の実機・仕様確認~~ **解決(2026-07-02)**:
   実機確認は不可のため公式仕様書(BVPMB-01 Rev 1.3)の逆抽出分析で判断——**焼き込み固有値**
   =個体識別型hardware_idで確定(詳細はD5宿題節)。Wave 0ブロッカー解除
1. **series識別モデル**(最優先。dedupキー・認可・復旧・出口契約の根元。Wave 0のコード前に確定)
   → **確定候補: D5**
2. ~~ADR 42本(monojoh-authority)の生死棚卸し~~ **完了(2026-07-02)**: 全42本を判定
   (維持16/要改訂21/廃止1/保留4)。結果・権威規則・処置案は [../adr-inventory.md](../adr-inventory.md)。
   ADR本文への反映は各キュー確定時に処置案どおり実施(新規発見: 0031に安定意図段階と決定文書→ADR
   昇格ルールの穴)
3. 出口契約とADR 0028/0029/0030/0032/0035の統合方針(push/pull、アーカイブ責任消費者との接続)。
   0030は劣化契約(R17)とアーカイブ責任消費者ackを上位規則とし、no-silent-dropを
   「契約外の黙示破棄禁止」に読み替える。必須入力(D5波及): **検疫遷移の配送**(解除の再エンキュー・
   遡及検疫annotation=D5決定3)+**エポック複合カーソル**(D5決定3)
4. ~~測定レジストリの正本二義性の解消+初期語彙~~ **確定(2026-07-02)**:
   [D6](D6-measurement-registry.md)(二層+copy-on-enable+初期語彙10キー+構造化値型record)。
   **Wave 0の設計ブロッカーはこれで全解除**
5. ~~論点2: 南向き契約(コマンド/ヘルス/ライフサイクル)~~ **確定(2026-07-08)**:
   [D12](D12-southbound-contract.md)(世話のみ・権限3分類・DFUキャンペーン・受領側認証)。
   「取り込みのみ可」の読み替えは解除——形態③(外部アダプタ)は能力宣言による南向き任意参加(D12決定5)
6. ~~UIスコープ文書~~ **確定(2026-07-08、初期所有権は2026-07-12改訂)**: [D13](D13-ui-scope.md)(器と予算・未所有時local recovery+
   管理者パスフレーズ1本・画面在庫・NOTリスト・時間集約派生series予約)

## Waveマーキング(責務台帳への割当。実装計画時に精緻化)

- **Wave 0実装**: R1, R3, R6(初期語彙+検証最小), R7(台帳最小・CLI登録/承認+replace-hardware
  ガードレールCLI版=D5), R8, R11(範囲クエリ+CSV),
  R12(ヘルスJSON最小), R16, R17(retention+水位), R18(スキーマ列のみ・値はedge固定), R20, R22(手動export。
  スナップショット形式とエポック規則はD2 §3.5「R22最小契約」に従う)
- **境界の明文化(2026-07-02、外部レビュー指摘反映)**:
  - **R13**: Wave 0は**append-only監査イベント行(ledger_events)のみ**=R13の最小下地。
    R13本体(構造化イベント履歴・インシデントバンドル)はWave 1。D6の「自動有効化+R13監査イベント」は
    Wave 0ではこのイベント行への記録を意味する
  - **R9較正**: Wave 0では**較正は恒等(オフセット0/倍率1)で未実装**。R9本体はWave 1。
    ただし `value_semantics` 列・series値域検証(D6値域3層)・較正要再確認**状態の列**は初日から
    スキーマに存在する(後からの列追加でスキーマ破壊しない)
- **Wave 1実装**: R2, R9, R10, R13, R14, R15, R19, R21(手順化), R23, R4/R5(D12の範囲で確定:
  世話動詞・DFUキャンペーン・形態①②の受領経路。接点出力駆動は対象外=D12決定1)
- **Wave 2実装**: R21(A/B自動化), R5拡張, 振動/カメラ関連
