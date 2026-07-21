# D4: アダプタの解剖学(ドライバ/ランタイム/クライアント)

Status: 確定 (2026-07-02、3レンズレビュー済み: 概念整合/Rust実装/業界比較)
確定した定義は [../terminology.md](../terminology.md) が正本。本文書は根拠と含意を記録する。

## 決定

**アダプタ = device integration(transport backend + device driver) +
adapter runtime/composition + 契約クライアント**(北=取り込みクライアント、南=世話サービサ=D12決定8)
の3群を合成するパッケージであり、供給者としての**説明責任単位**。transportとdevice driverを
別境界として実装しても、device integrationという南側の一群であることは変わらない。

| 部品 | 話す相手 | 知らないもの | 実例(iotkit-next) |
|---|---|---|---|
| transport backend | raw I/O(open/read/write/combined write-read)。OS・board差を吸収 | IC register意味、データシート変換、取り込み契約 | rpi4b-transportのI2C/serial等 |
| デバイスドライバ | transportを介した検出・初期化・読取、コーデック、データシート変換。南向きencodeも含む | measurement key、source、series、取り込み契約(ドライバSPIには従う) | rpi-localのI2C driver、bravepi-codec、iotkit-sensor-driversのIC変換部品 |
| アダプタランタイム | スケジューリング、状態機械、panic隔離、driver lifecycle、南向きディスパッチ | 契約、IC、measurement写像 | iotkit-polling-adapter-runtime(現行I2Cポーリング型)、BravePI event_loop(イベント駆動型) |
| package固有composition | 対応device model、measurement写像、runtimeと契約clientの合成。series解決はコレクタ=D5 | ledger mutation、principal発行、再起動policy | rpi-local-adapter、bravepi-mainboard-adapter |
| 取り込みクライアント | 取り込み契約(エンベロープ/ack/spool)。北向き専用 | ハードウェア | `iotkit-ingest-client`。公式in-process adapterへprincipal-bound clientを注入 |

## レビューの評決と根拠

- 業界比較: EdgeXのDevice Service分割(SDK+protocol driver)、Home Assistant ADR-0004
  (「ドライバはHAを知らないPyPIライブラリに」)、Linux IIO/Zephyrのtransport分離と**三重に同型**。
- Rust実装: 既存クレートへの写像はほぼ完全。取り込みクライアントは実装済み。既存
  `AdapterEvent`互換投影は、Input Adapter v1でBravePI package内のprivateなlegacy wrapperへ隔離する。
- 概念整合: 2部品式は双方向デバイス(BravePI downlink/DFU)で破綻 → 3部品+南の席予約で解消。

## 確定した細部

1. **正規化の3分掌**: デコード(データシートの数学)=device driver / measurement写像(measurement_key+
   channelへの写像)=adapter package固有composition / 現場較正(オフセット・倍率)=R9。共有runtimeは写像を
   知らない。**series解決(台帳→system_id→series)は
   コレクタ**(D5反映 2026-07-02)。判定基準「データシート由来かデータ現場設定由来か」。
2. **監督マトリクス**: 再起動権限(R20)は形態①のみ。②③④は死活観測+検疫+エスカレーション。
3. **区別不能の精密化**: ②③④は北向きについて区別不能。南向き能力は**能力宣言**(R7台帳)で申告、推定しない。
4. **能力宣言**: 何を測り・何のコマンドを受けるかの宣言。R9/R14は南向き非対応デバイス宛の
   アクション設定を**設定時に拒否**(実行時に宙に浮かせない)。
5. **南向き最小サブセット**(Sparkplug rebirth の教訓): 再アナウンス要求(redescribe)等は
   **ack応答へのピギーバック**で実現。※ackを読まないfire-and-forget送信者(形態③④)には届かないため、
   D5は `declaration_version` のエンベロープ同梱+コレクタ側版不一致検知で補完する
   (D5波及 2026-07-02。下記引き継ぎ事項参照)。
6. **語の衝突回避**: OSのドライバは常に「カーネルドライバ」と書く(R1修正済み)。
   Eclipse Honoの"protocol adapter"(プラットフォーム側)との混同に注意——本設計のR2はアダプタではない。
7. **親子デバイス**: BravePIメインボード(親)自身もコマンド宛先。南向き宛先指定は
   「デバイス宛・アダプタ経由ルーティング」。

## クレート設計(Wave 0)

- `iotkit-ingest-contract`: Envelope / Ack / reason_code / envelope_id採番レシピ。**依存はserdeのみ**。
  共有適合テストスイートはこのクレートだけに依存する。
- `iotkit-ingest-client`: バインディングはfeatureフラグ(`inproc` default / `http` / `mqtt`)。
  公式アダプタのビルドにHTTPスタックを持ち込まない。
- **core/typesには足さない**(測定語彙/北向き境界/南向き境界の3語彙混在をこれ以上悪化させない)。
- 既存クレートのリネーム・event_loopの3責務分割・core/types分割は**やらない**(論点2確定後に判断)。
  [論点2確定(D12決定8 2026-07-08): 南向き語彙は新契約クレート、リネーム等の実判断はWave 1実装計画の宿題へ]
- SDKは便宜品、**ワイヤ契約が規範**(Azure/EdgeXの言語マトリクス疲弊の教訓)。
  祝福されたRustライブラリ1本+他言語は薄いワイヤ実装で十分。
- **変換境界は一箇所**(2026-07-02、外部レビュー指摘反映): 移行期間中、旧語彙(AdapterEvent)と
  新契約(Envelope)の変換は**明示的ブリッジ1ファイルに限定**する(Wave 0のEdge Node内暫定ブリッジは
  Input Adapter v1でadapter package内のprivate legacy projectionへ置換)。新規コードがAdapterEventへの依存を
  **増やすことを禁止**——旧語彙は論点2確定までfrozen vocabulary(D12決定8で**移行ブリッジ削除まで**に
延長 2026-07-08)。北向き/南向き/測定語彙の再一体化を防ぐ。

## Input Adapter v1実装追記(2026-07-20、review完了)

公式in-process adapterの**北向きhost追加境界**は
[`docs/input-adapter-contract.md`](../../input-adapter-contract.md)で確定・実装した。これは完全なD4 adapter
契約やD12 care-servicer完成を表さない。compile-time catalog、
安定したtype/instance/sourceの分離、Edge Node所有principal、supervision非依存host/composition API、
Edge Node-private factory wrapperを採用する。ここでいう共有境界はadapter packageのhost/composition APIであり、
driver/runtime自身は取り込みclientを知らない。factoryはrestart権限やhealth真実を所有せず、静的な対応mappingを
実デバイスの能力宣言として扱わない。動的pluginと完全なcapability declaration convergenceはv1対象外。
`iotkit-ingest-client`が最終Ack/放棄を表すreceiptと同一Envelopeのretry ownershipを持ち、
`iotkit-input-adapter-host-api`がsource-bound送信、activity、bounded diagnostics、completion、shutdownを
提供する。Edge Nodeは静的catalog、instance設定、principal、inventory、再起動、healthを所有する。
RPi-localでは、配置設定がadapter packageのcatalogからmodelとmodel固有設定を選び、Edge Node-private factoryは
その不透明な設定をpackageへ渡す。host platform世代は設定・source・device identityへ含めない。位置identityを
維持したまま別modelへ黙って差し替えることは、台帳へ永続化したmodel fenceとの不一致としてruntime開始前に
原子的に拒否する。

## 論点2(南向き契約)への引き継ぎ事項

- 南向きは北向きと**別契約・別チャネル**。ランタイムが両チャネルを所有する合成点。
- **desired-state同期(宣言的・冪等・retained)とコマンド(命令的・タイムアウト・応答)を分ける**
  (Azure twin / AWS shadow パターン。R15のdesired/reportedと接続)。
- コマンド受領・TTL/冪等/タイムアウト管理(R4)・有界ジョブのライフサイクル報告はランタイムの南向きディスパッチ。
  DFU中のポーリング停止など北南の調停もランタイム。
- MQTTの「ingest専用リスナー」は「デバイス間pub/subはしない(Edge Node⇔デバイスの契約トピックのみ)」に
  精密化(南向きコマンドトピックの追加余地)。HTTP-onlyデバイスにはlong-poll等の受領経路。
- **redescribeの形態別分解**(D5波及 2026-07-02): 形態①②は南向きチャネルで強制。形態③④は
  能力宣言の世代番号(`declaration_version`)をエンベロープ同梱必須とし、コレクタが版不一致検知で
  当該デバイスを検疫に落とす。
- 「取り込みクライアント」の対名(南=コマンドサービサ等)は論点2で命名。[命名済み 2026-07-08:
  **世話サービサ(care servicer)**=D12決定8。「コマンドサービサ」はdesired同期・ジョブも受けるため不採用]

## 監査追記(2026-07-03 実装還流: Wave 0計画1・2で実データ破壊2件を検出した教訓の明文化)

- **ドライバの値域クランプ・飽和・値域補正の禁止**(確定した細部1の精密化): ドライバのデコードは
  データシート定義の変換のみ。**測定値のクランプ・飽和・レンジ丸めは禁止**——値域判定はR8
  (レジストリのカタログ物理限界)の専権であり、ドライバのcapは実測値を検出不能に改変する
  (実証: VL53L1Xの2000mm capが実測3mを「正常な2m」として保存していた。計画2最終レビューで検出・除去)。
- **単位対応表の宣言義務**(measurement写像の要件追加): アダプタのmeasurement写像は、
  「ドライバ出力単位 × 変換係数 → D6正準単位」の対応表をコード内に宣言・文書化する
  (例: LIS2DUXS12は g → ×1000 → acceleration_mg、派生値magnitudeは正準チャネル外につき破棄)。
  対応が暗黙だと単位事故はレビューを素通りする(実証: 加速度g/mGの千倍ずれをClaude系4層レビューが
  見逃し、外部codexの実コード照合のみが捕捉した)。写像移設(計画3)ではこの対応表が必須成果物。
