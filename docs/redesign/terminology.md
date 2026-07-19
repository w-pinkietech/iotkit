# IoTKit / YokaKit 再設計 用語集

Status: 会話合意済み (living document)
Updated: 2026-07-20

この文書は再設計に関わるすべての文書・コード・会話で使う統一語彙を定める。
ここにない語を新しく使うときは、まずこの文書に追加する。

## 配置の4段(「箱」の役割階層)

システムの構成要素は必ず次の4段のどこに住むかを明示する。
段の判定は**役割**で行う(2026-07-08 D10改訂——ホスト型IoTKit Siteの導入により物理配置基準を廃止):
**保持する状態が単一の `site_id` に束縛されるなら[3]、site_idを跨ぐなら[4]**。物理配置は既定値
([3]=サイト内LAN)にすぎず、判定基準ではない。

| 段 | 用語 | 英語 | 実体 | 説明 |
|---|---|---|---|---|
| [1] | デバイス | device | BravePI Transmitter、直結I2Cセンサー、第三者の自作デバイス(ESP32/PLC等) | 測定・作動する末端。IoTKit Edgeの配下 |
| [2] | **IoTKit Edge**(短縮: Edge、役割名: Edge Node) | IoTKit Edge / Edge Node | 現場に置くRaspberry Pi。IoTKit Edgeが動く | 収集・正規化・耐久buffer・再送を担う。責務台帳 R1〜R23 はすべてこの箱の責務 |
| [3] | **IoTKit Site**(短縮: Site) | IoTKit Site | 単一サイトのMQTT Broker、Archival Store、query、site-local sensor semantic mapping、application接続・export境界を担う箱。Standaloneでは不在可、Site-managedでは必須(D8) | 初期実装はGo+SQLite。複数Edge Nodeを集約し、YokaKitは別applicationとして接続する |
| [4] | クラウド | cloud | **site_idを跨ぐ上位層**(オプション)。商用クラウドに限らず、本社サーバールーム等もここ。※ホスト型[3]と同じデータセンターに同居しうるが、単一site_idに閉じるインスタンスは[3]である(役割基準) | クラウドLLM API、クラウドYokaKit、複数拠点統合・fleet管理・DR複製が住む。**サイト横断は必ずここでやる(IoTKit Site同士に上下関係を作らない)** |

### 禁止・注意語

- `Edge`は`IoTKit Edge`の会話上の短縮として使える。アーキテクチャ上の役割は必ず`Edge Node`と書き、`Node`単独は使わない。
- `Gateway`はIoTKitの構成要素名として使わない。業界カテゴリを説明する一般名の`IoT gateway`に限って使用できる。
- **「サーバー」を単独で使わない**。[3]の製品名はIoTKit Siteであり、[4]とは区別する。
- **「上流」(upstream)** = Edgeから見たデータの届け先([3]または[4]にいる消費者)。「下流」(downstream) = Edgeから見たデバイス側。
- **IoTKit Site[3]をIoT gatewayと呼ばない**。業界にはIgnitionのように中央サーバーを"Gateway"と呼ぶ流派が実在するため、明示的に区別する。
  また**BravePI Mainboardはデバイス[1](親デバイス)であり、Edge Nodeではない**(BLE受信機のため誤呼称されやすい)。
- **対クラウド境界(「サイトの門」)は箱ではなく出口契約(R10)である**。StandaloneではIoTKit Edge 1台に
  取り込み契約(入口)と出口契約(出口)の両面が載る([3]は不在)。「第二の門はどの箱か」という問いは立てない。

### 業界対応表(第三者向け)

| うちの箱 | 業界標準図での位置 | 各生態系での呼び名 |
|---|---|---|
| [2] IoTKit Edge | 「IoT gateway/エッジPC」 | AWS: Greengrass core device / SiteWise Edge gateway、Azure: IoT Edge gateway、Sparkplug B: Edge Node |
| [3] IoTKit Site | 「MQTT Broker or IoT Hub」 | Sparkplug B: MQTT Server + Primary Host Application、ヒストリアン(PI Server)、**Ignitionの"Gateway"はここ(混同注意)** |

## 三本柱(再設計の一級要件)

| 柱 | 名称 | 一行定義 |
|---|---|---|
| 柱1 | 止まらないデータプレーン | 他の箱が全部死んでいても、Edge単独でデータ収集・保全が継続する |
| 柱2 | オープンな接続契約 | 入口(デバイスをつなぐ)も出口(データを使う)も公開・版管理された契約。特権的な裏口は存在しない |
| 柱3 | AI運用可能な制御プレーン | 全運用状態が構造化データで読め、全運用操作が型付きカタログで叩ける。判定基準:「専門家がSSHなしで直せるなら、AIにも直せる」 |

## 中核概念

| 用語 | 英語 | 定義 |
|---|---|---|
| プロバイダ | provider | センサー/デバイスのベンダー技術体系(BravePI、BraveJIG等)。コアの語彙に漏れてはならない |
| ホストプラットフォーム | host platform | Edgeを実行するOS・CPU architecture・board能力。Raspberry Pi 4B/5等。起動可否と診断には使うが、adapter/source/device identityには含めない。board世代を手設定で選ばせず、実装が必要な能力を検査する |
| transport backend | transport backend | Linux I2C、serial、GPIO等のraw I/Oをopen/read/writeする薄い境界。IC register意味、データシート変換、measurement、取り込み契約を知らない。I2Cはraw read/writeに加えてrepeated STARTを保つcombined write-readを提供する |
| デバイスドライバ | device driver | transport backendを介して特定IC/機器の検出・初期化・読取・protocol/register処理と**データシートの数学**(生値→物理量)を担う。measurement_key、source、series、取り込み契約を知らない。※**OSのカーネルドライバとは別概念**。OS側を指すときは必ず「カーネルドライバ」と書く |
| 対応デバイスモデル | supported device model | 1つのInput Adapter buildが扱える機器モデル。安定したmodel ID、モデル固有設定、device driver生成、measurement写像、inventory表示情報をadapter package内のcompile-time catalogへ束ねる。catalogは対応可能モデルであり、接続済みinventoryではない。配置設定はcatalogから接続対象を選ぶが、host platform名を選ばず、model IDをdevice identityへ混ぜない。同じ位置へ別modelを黙って割り当てる変更は台帳fenceで拒否する |
| ドライバSPI | driver SPI | ドライバが結果を返す先の型体系(`SensorReading` 等)。ドライバは取り込み契約を知らないが、この**第二の契約**には従う。名前と版管理を持つ(Home Assistantのライブラリ地獄の予防) |
| アダプタランタイム | adapter runtime | ドライバを実際に走らせる**胴体**: スケジューリング(ポーリング/イベントループ)、検出・接続の状態機械、ドライバのライフサイクルとpanic隔離、南向きコマンドのディスパッチと有界ジョブ実行。共有runtimeは契約、IC、measurement写像を知らない。**adapter package固有のruntime/composition層**がmeasurement_key+channelへの写像を所有し、series解決はコレクタが所有する(D5)。`iotkit-polling-adapter-runtime` がI2Cポーリング型の現行実例。BravePIのevent_loop/serial_sourceはイベント駆動型の胴体 |
| 取り込みクライアント | ingest client | **取り込み契約と話す**コード(**北向き専用**): 正規形のエンベロープ化・envelope_id採番・送信・ack処理・(必要なら)spool。バインディング(プロセス内/UDS/HTTP/MQTT)を選ぶ。全アダプタで共有可能な1ライブラリ。ESP32ファームのHTTPクライアントと同種。南向きは別契約・別チャネル(D12。対名=世話サービサ) |
| アダプタ | adapter | **device integration(transport backend+device driver)+adapter runtime/composition+契約クライアント(北=取り込みクライアント、南=世話サービサ、D12)の合成パッケージ**であり、供給者としての**説明責任単位**: 認証済み送信者として識別され、死活観測(R7/R12)の対象になり、南向き参加時は契約の宛先になる。形態4種: ①**公式アダプタ**(Edge内プロセス)、②**衛星アダプタ**(同一コード別筐体)、③**外部アダプタ**(第三者製、任意の言語)、④**契約ネイティブデバイス**(device driver不要。ランタイム+クライアント=ファームウェアそのもの)。**監督(再起動権限、R20)は形態①のみの性質**——②③④は死活観測+検疫+エスカレーションのみ。②③④は**北向きについて**Edgeから区別不能。南向き能力は能力宣言で申告される(推定しない)。※ネットワーク入口(R2)はアダプタではなくコレクタの玄関。翻訳責任は常に送る側 |
| 能力宣言 | capability declaration | アダプタ/デバイスが「何を測り(measurement_key×channel構成)、何のコマンドを受けられるか」をデバイス台帳(R7)に宣言する仕組み。南向きの宛先解決と、R9/R14の事前条件検証(南向き非対応デバイス宛のアクション設定を**設定時に拒否**)の基準。再アナウンス要求(redescribe)等の最小応答はack応答へのピギーバックで受動参加可能(ただしackを読まないfire-and-forget送信者=形態③④には届かないため、宣言の世代番号 declaration_version のエンベロープ同梱+コレクタ側版不一致検知で補完する=D5)。詳細はD12(未宣言動詞の拒否時点=D12決定2) |
| 北向き / 南向き | northbound / southbound | 北向き=デバイス→Edgeのデータ方向(D1で契約確定)。南向き=Edge→デバイスの機器の世話方向(D12で契約確定: 照会・構成・コマンド・有界ジョブ)。公式①・衛星②は双方向市民(②は第一波必須)、外部③は能力宣言による任意参加、契約ネイティブ④の南向き参加は保留(D12決定5) |
| コレクタ | collector | Edge側の受理の権威。エンベロープの受理・重複排除・確認応答・逆圧を担う(R8) |
| エンベロープ | envelope | アダプタ/自作デバイス→コレクタ間の配送単位。安定ID(envelope_id)を持ち、再送しても安全 |
| 取り込み契約 | ingest contract | エンベロープをコレクタに届けるための公開ワイヤ契約。バインディングは複数(プロセス内チャネル / UDS / HTTP / MQTT ingest専用リスナー)だが論理契約は一つ |
| 正規化 | normalization | プロバイダ固有の生データ→測定レジストリ準拠の正規形への変換。アダプタ(=つなぐ側)の責任。**3段に分掌**: ①デコード(データシートの数学: 生値→物理量)=device driver、②measurement写像(measurement_key+channelへの写像)=adapter package固有のruntime/composition層、③現場較正(オフセット/倍率、現場設定の数学)=R9(コレクタ後段)。**series解決**(送信者アイデンティティ+subject_hint→台帳→system_id→series_id)は写像ではなく**コレクタの責務**(D5)。判定基準:「データシート由来の数学はドライバ、現場設定由来の数学はR9」 |
| 派生系列プロセッサ | derived-series processor | R9のうち、受理済み観測から較正・累積count・時間集約等の**新しいseries**を決定的に生成する概念境界。元観測を上書きせず、導出revision/入力series/適用境界を持つ。名前だけを理由にcrate化しない |
| ローカルルール評価器 | local rule evaluator | R9のうち、受理済み・非検疫の観測/状態へ型付き有界条件を評価し、型付きaction intentだけを生成する概念境界。I/Oや変更を直接行わず、実行はR14 dispatch・権限・監査・TTL/冪等性を通す。自由script/rule engineではない |
| 測定レジストリ | measurement registry | 測定種別・単位・型の語彙の定義と版管理(R6)。正規化の目標形を定める権威。**二層構造**: 命名の典拠=標準語彙カタログ、受理の正本=現場レジストリ(D6) |
| 標準語彙カタログ | standard vocabulary catalog | measurement_keyの命名・正準単位(UCUM)・値型・意味論クラス・物理限界値域・チャネル役割を定めるリポジトリ資産(Edgeバイナリに同梱)。契約仕様書の一部として公開=柱2の実体。受理判定には直接使われない(診断・候補提示のみ)(D6決定1) |
| 現場レジストリ | site registry | Edge DB内の測定レジストリ正本(D2)。カタログから有効化(copy-on-enable=コピーして固定)したエントリ+現場カスタム定義(`custom.`名前空間)+エイリアス表の合成。R8受理判定の唯一の参照先(D6) |
| 出口契約 | egress contract | Edge→[3]/[4]の消費者へのデータ公開契約(R10、**上流向き**——「北向き」はD1側の語であり出口には使わない)。複数消費者・消費者別カーソル・at-least-once。本体はD7 |
| 消費者 | consumer | 出口契約でデータを受け取る側。YokaKitは特権なしの一消費者 |
| パブリッシャ | publisher | 出口契約の送信側実装(Edge内、outboxから配送する部品) |
| outbox | outbox | 配送待ちデータの永続バッファ。停電・長期断線を跨ぐ。上限と劣化契約を持つ(R17) |
| 制御プレーン | control plane | R12〜R15。Edgeが提供する観測・診断・操作・設定のAPI面 |
| データプレーン | data plane | デバイス→取り込み→保全→出口配送のデータの流れ |
| 操作カタログ | operation catalog | 型付き・dry-run付き・権限段階付き・監査付きの運用操作の一覧(R14) |
| インシデントバンドル | incident bundle | 障害時にEdgeが生成する自己完結の診断パッケージ(症状+直近イベント+設定+ヘルス時系列)(R13) |
| desired / reported | desired/reported | 設定の「あるべき姿」と「実際に適用された姿」の分離(R15)。乖離は検出・報告・収束される |
| host-agent | host-agent | [2]内でアプリ本体から分離された特権操作プロセス。**sudo級特権操作のみ**(再起動・時刻設定・サービス制御等)。ハードウェアI/O(シリアル/I2C)はアダプタのドライバが直接扱い、host-agentは経由しない(レビュー反映 2026-07-02) |
| AIオペレーター | AI operator | [3]または[4]に住み、制御プレーンを叩いて診断・復旧を行うAIエージェント。Edgeの一部ではない |
| runbook | runbook | AIオペレーター/人間が使う、機械可読な対処手順 |
| 有界ジョブ | bounded job | 長時間かかる操作(DFU、スモークテスト等)の明示的なライフサイクル(開始/進捗/完了/失敗)を持つ実行単位 |
| 監督 | supervisor | アプリレベルの監視・再起動・修復理由の記録(R20)。プロセスレベルはsystemd/HWウォッチドッグに委譲 |
| 時刻品質 | time quality | 全測定・イベントに付くタグ: synced / holdover / unsynced(R18) |

## 運用・配送の概念(整合性監査 2026-07-02 で追加)

| 用語 | 英語 | 定義 |
|---|---|---|
| 検疫 | quarantine | データ/デバイスを「保存・可視化(R11)はするが、下流配送(R10)とアクション駆動(R9)には使わない」状態に置くこと。値域外データ・未登録測定キー・登録直後デバイスに適用。解除は時限自動失効またはR14の操作(誰も解除できない誤検知を作らない)。※「隔離」という語は使わない |
| 正本 | source of truth | その情報の権威あるコピー。「その情報なしで動けなくなる箱が持つ」(D2) |
| 保管責任の引き渡し | custody transfer | 測定データを失わず保持する責任が、耐久保存の確認とともにEdgeからSiteへ移ること(D2/D9) |
| アーカイブ責任消費者 | archival consumer | 出口契約の消費者のうち台帳で1つ指定。そのackのみが正本移転=パージ許可を意味する(D2) |
| AIハーネス | AI harness | AIオペレーターの実行環境(エージェントループ・runbook・Edge APIツール・認証情報)。キットの提供物で[3]/[4]に住む。背後のLLMは差し替え可能 |
| operatorトークン | operator token | 制御プレーンを叩く運用主体(AIハーネス/人間)の認証情報。権限段階つき(D3決定5) |
| 劣化契約 | degradation contract | 資源上限到達時に「何をどの順で失うか」の事前合意(R17)。段階の具体(間引き/要約化/最古削除の順序)は設計スペックで確定 |
| spool | spool | 送信側がack受領まで保持する一時バッファ。耐久性は送信側の階級(メモリ/ディスク)に依存し、契約はそれを保証しない(D1) |
| dedup台帳 | dedup ledger | コレクタ側の重複排除記録。TTL+サイズ上限で有界(D1) |
| 衛星アダプタ | satellite adapter | Edgeと別筐体で動かすアダプタランタイム。HTTPバインディングでコレクタに送る(D2 §4) |
| 接続状態機械 | connection state machine | 上流接続のonline/offline/degraded遷移管理。R10(再送)とR12(状態公開)に属する |
| time_source | — | 時刻の**出所**タグ: device_ntp / device_rtc / edge / edge_adjusted(D1)。時刻品質(確度: synced/holdover/unsynced、R18)とは直交する別タグ |
| publication_id | — | 出口契約の**バッチ再送の冪等キー**(消費者側dedup用)。レコード同一性ではない——同一性は `(epoch, seq)`(D7決定4) |
| record family | — | 出口ストリームのレコード種別タグ+スキーマ版。version 1はmeasurement/annotationとoptionalなcommissioning_smoke、予約=文字列観測・時系列ブロック/波形。未知familyの読み飛ばしはoptional familyに限る(D7決定2) |
| publication log | — | 出口ストリームの採番権威。全record familyが共有する単調増加seq((epoch, seq)カーソルの実体)。readingsの内部挿入順とは別——検疫行は解除まで採番されない(D7決定4) |
| publication snapshot | — | 消費者再構築用の現在状態スナップショット(対応するseq水位を刻印)。R22スナップショット(高機密資産・readings非含有)とは**別語・別物**(D7決定8) |
| event_time | — | 出口レコードの正準イベント時刻。導出規則=device_time→age_ms復元(edge_adjusted)→received_at、妥当窓検査は未来方向のみ(D7決定3)。観測時刻であり単調ではない——順序・カーソルには使わない(D7決定4) |
| event_time_source | — | event_timeにどの候補を採用したか+未来方向降格の有無を表す出口レコードのフィールド。time_source(入力の事実)とは別の、導出結果の表示(D7決定3) |
| target / target registry | — | 出口配送先の登録単位とその台帳。配送状態(カーソル・ack)はtarget単位で分離。登録・変更はR14型付き操作(D7決定6) |
| 購読フィルタ | subscription filter | targetが受け取るシリーズの選択(実体化series_keyで照合)。解釈ではなく選択。アーカイブ責任targetには適用しない(D7決定7) |
| 保管対象ポリシー | custody scope policy | アーカイブ責任消費者に託して長期保存するシリーズ範囲の台帳宣言(既定=全量)。対象外シリーズには保管責任の約束自体がなく、retention窓超過で`custody_lost`にならず期限失効する(D7決定7) |
| 配送制御通知 | delivery control notice | 特定targetの配送状態についての通知(gap/cursor_expired等)。ストリームレコードではなくpushバッチのメタデータ(帯域外)で運び、カーソルを消費しない。全target共有のannotation族とは別レイヤ(D7決定2・6) |
| 目撃ステージング | sighting (staging) | 未知hardware_idの観測を有界・パージ可能に保持する登録前状態(hardware_id仮キーでデータ保持)。人間承認の瞬間に採番→検疫→active(D5決定4)。ステージング中のackは `disposition: staged`(D1監査追記) |
| retire(墓標) | retire / tombstone | 台帳エントリの削除に相当する終端状態。行は消さず、system_id再利用は永久禁止(D5決定4) |
| superseded_by | — | retire済みエントリから後継エントリへの参照。replace-hardware確定時に旧候補へ付与(D5ガードレール4) |
| replace-hardware | — | 個体識別型デバイスの交換時、台帳エントリのhardware_idだけを張り替えてseries(履歴)を継続させる明示操作。ガードレールはD5決定4 |
| 台帳エポック(世代番号) | ledger epoch | Edgeの台帳の世代番号。箱交換(R22)を跨ぐ出口カーソル連続性とスプリットブレインのフェンス(D2 §3.5、D5決定3)。※D1の `boot_epoch`(デバイス起動カウンタ)とは**別概念** |
| value_semantics | — | seriesの値の意味クラス: `raw_legacy`(較正前生値)/ `calibrated`。R9較正の二重適用防止(D5) |
| 較正要再確認 | calibration review required | seriesの較正の信頼を保留する状態(交換疑いシグナル・replace確定時)。検疫との違い: データは流れるが較正の信頼が保留されている(D5決定2) |
| 最低保持フロア | minimum retention floor | アーカイブ責任消費者のack後もEdgeに置く最低保持期間(目安72h、設定可)(D1・責務台帳。旧称「パージフロア」は廃止) |

## サイトトポロジ(複数Edge Node。D8 2026-07-07)

| 用語 | 英語 | 定義 |
|---|---|---|
| Standalone | standalone | サイト内のEdge Nodeが1台で、IoTKit Siteや上流接続が任意の構成(D8) |
| Site-managed | site-managed | 複数Edge Nodeを独立した完全なEdge NodeとしてIoTKit Siteへ接続する構成(D8) |
| edge_node_id | — | Edge Nodeの安定した外部同一性。消費者側の大域レコード同一性 `(edge_node_id, epoch, seq)` の先頭成分(D8) |
| Site Aggregator | site aggregator | canonical recordをsite表示やapplication向けに投影する非custodialロール(D8) |
| Archival Store(アーカイブ責任) | archival store / archival consumer | raw canonical recordを耐久保存し保管完了確認を返すSiteロール。この確認だけがEdge purgeを許可(D8/D9) |
| archive_lost | — | Siteが一度保管責任を引き受けた後に失った範囲を表す監査事実。MVP後のhardening対象(D8) |

## 出口MQTTバインディング(D9 2026-07-13改訂)

| 用語 | 英語 | 定義 |
|---|---|---|
| 出口MQTTバインディング | exit MQTT binding | EdgeがMQTT Brokerへ有界batchをQoS 1 publishするR10第一バインディング。Broker PUBACKはtransport受領だけを表す(D9) |
| 保管完了確認 | application custody acknowledgement | Siteがraw recordと連続cursorを同一transactionでcommitした後、`accepted-through` topicへpublishする正式水位。これだけがEdge purgeを許可する(D9) |
| 送信窓 | sending window | 保管完了確認待ちbatch数の上限。MVPは1。PUBACKでは窓を解放しない(D9) |
| Archival Store | archival store | canonical recordを耐久保存して保管完了確認を返すSiteの役割。MQTT Broker自体はArchival Storeではない |
| Broker enrollment | broker enrollment | Edge固有credential、exact topic ACL、接続profileをBroker/Edge hostへ導入し、MQTT通信を許可する操作。Site raw historyへの参加許可ではない(D9/D10) |
| Site activation | site activation | Site adminがdescriptorで発見したexact `(edge_node_id, ledger_epoch)`について、activation後のpublication受理を一度だけ許可する操作。Broker設定変更ではない(D8/D9/D13) |
| 登録前ローカル値 | pre-activation local reading | Edgeへ耐久保存されるがpublication logへ採番されず、Site custody・履歴・後日replayの対象にならないcommissioning確認値(D8/D9) |

## 出口認証(D10 2026-07-13改訂)

| 用語 | 英語 | 定義 |
|---|---|---|
| Edge Node credential | edge node credential | Edge Nodeごとに発行するstatic Broker credential。共有禁止、Git/argv/log非掲載、当該Edge NodeのtopicだけをACLで許可(D10) |
| 管理overlay経路 | managed overlay path | Tailscale等の外部control planeを持つ任意の到達経路。IoTKitの必須要件ではなく、利用時もoverlay identityだけをapplication認証にしない(D10) |
| credential hardening | credential hardening | enrollment、短命化、rotation、無人再発行等の配布前候補。最初の1 Edge Node実機スライスには含めない(D10) |

## 入口認証(D11 2026-07-08)

| 用語 | 英語 | 定義 |
|---|---|---|
| 流量クラス | rate class | デバイス登録時に申告する想定流量の粗い段階(既定クラスあり)。容量設計=現場エンジニアの責任、執行=ソフトの責任、という分担の実体。クラス変更は人間のみ(D11決定4・8) |
| 絞り | throttle | 流量クラス超過分を**非終端**の応答で退けるシステム自動執行。HTTP=429+Retry-After(耐久ackなし)、ack語彙上は `deferred`——終端 `rejected` には決して写像しない(spool持ち送信者のデータ破壊防止)。可逆・ヒステリシス付き自動解除・騒がしく(アラーム+R23+監査)(D11決定4) |
| 対応の階段 | response ladder | 入口の事故対応の順序: 絞る(自動)→検疫(自動・既決)→トークン失効(人間のみ)。自動対応は必ず騒がしく行う(D11決定4) |
| ペアリング窓 / 登録コード | pairing window / registration code | デバイス登録の儀式(D1既決)。登録コードは単回使用・短TTL・窓内のみ有効。窓は自動クローズ、開けっ放し禁止(D11決定6) |
| 入口リスナー既定オフ | ingress listener off-by-default | ネットワーク入口(HTTP/MQTT ingest)は既定で無効。有効化・bind変更・プロトコル追加は独立した工事層操作(device addの暗黙副作用にしない)。インターネット公開は禁止——遠隔地からのデータは別のIoTKit Edge[2]+出口契約で運ぶ(D11決定7) |
| site_local_cidr | — | 入口リスナーのbind先を定義する明示設定(CIDR+許可インターフェース)。「LAN限定」の検証可能な実体。別拠点・第三者WiFi・VPN越しのプライベートアドレスは含めない(D11決定7) |
| capacity_debt | — | 流量クラス申告合計が箱の実測体力を超えたまま、人間の明示承認で `device add`/クラス変更を通した記録。検算はPhase 6と操作のたびの両方で実行(D11決定4) |

## 南向き契約(D12 2026-07-08)

| 用語 | 英語 | 定義 |
|---|---|---|
| 世話 | device care | 南向きの許容範囲: 機器自身の管理面に閉じ、有界時間で自動復帰し、工程機器に電気的に接続されない操作。機器の外の現場状態を変える出力=アクチュエーションは契約対象外(D12決定1、ユーザー裁定) |
| 南向き動詞4分類 | — | 照会(read-onlyスコープ)/構成(desired-state経由のみ)/一過性コマンド(TTL付き・結果報告必須)/有界ジョブ。恒常的なあるべき姿のみdesired、時限で消えるべき状態はコマンド/ジョブ(D12決定2) |
| identify | — | 機体識別のLED点滅(短TTL・自動消灯必須=電池保護)。線引き規則により世話に属する(D12決定1)。中間層(D12決定3) |
| 世話サービサ | care servicer | 取り込みクライアント(北)の対名。南向き契約を受ける側の部品(コマンド受領・desired同期・ジョブ実行報告)(D12決定8) |
| DFUキャンペーン | DFU campaign | ファームウェア更新の有界ジョブ。人間が計画を1回承認(日常層)、実行はカナリア1台成功→続行のシステムジョブ。DFU中は当該デバイス宛の他の南向き動詞を拒否/保留(D12決定6) |
| 呼び鈴 | southbound notify | [3]→[2]の南向き通知。全構成でPi発の購読で受ける(サーバー発インバウンドはどの構成でも作らない)(D12決定7) |
| target_kind / 影響集合 | target kind / impact set | 南向き封筒の宛先種別(子/親/アダプタ)と、親・アダプタ宛先が影響を与える配下デバイス集合。影響範囲提示の実体(D12決定3・5) |
| 受信スコープ | receive scope | 南向き受領の認可: 認証送信者が取得できるのは自分のsubject集合宛のコマンドのみ(subjectスコープ認可の南向き版。他人宛の取得試行は越境として監査)(D12決定7) |

## Edge UI(D13 2026-07-08)

| 用語 | 英語 | 定義 |
|---|---|---|
| 未所有状態(旧setupモード) | unowned / local-recovery-required | admin credentialが無い状態。2026-07-12 Plan 6改訂でネットワークUI開放窓を廃止し、API/UIはbindしない。箱上のlocal `iotkit-edgectl`(物理/SSH root、非echo入力)で所有権を確立後にのみネットワーク管理面を開く。恒久的な認証無視スイッチは作らない(D13決定2) |
| step-up | step-up | ログイン済みセッションでも工事層操作に管理者パスフレーズ再入力を要求する追加確認。共有端末×長セッションの事故面対策(D13決定2) |

## デバイス識別(レガシー用語の置き換え)

| 新用語 | 定義 | 置き換えるレガシー語 |
|---|---|---|
| series(系列) | 測定の連続性の単位。外部表現=series_key、内部FK=series_id の2層(D5決定1・3) | (Node-REDの暗黙概念) |
| series_key | seriesの**外部表現**: `<subject_id>:<measurement_key>:<channel_index\|na>:<series_variant\|primary>`。API・出口契約・ログに限定(D5決定3) | — |
| series_id | 測定行が持つ**内部の整数FK**。seriesの台帳実体化(D5決定3)で発行 | — |
| subject_id | series_keyの先頭成分。= system_id(D5で確定) | — |
| subject_hint | エンベロープでitem単位に申告する解決ヒント(= hardware_id)。多subject送信者(親子束ね)は必須、トークン1:1送信者は省略可(D5決定1) | — |
| measurement_key | 測定種別の文字列キー。測定レジストリ(R6)の語彙 | sensor_type番号 |
| channel_index | 同一measurement内のチャネル番号。'na'はDB内では番兵値-1(D5決定3) | — |
| series_variant | 同一測定の変種(primary / count / pulse_count 等)(D5) | — |
| hardware_id | いま結線されている物理個体**または位置**の正規化識別子。個体識別型/位置識別型の2分類と継続性の意味論はD5決定2 | device_number 等 |
| user_label | 現場の人が付ける表示名 | デバイス名 |
| 短縮表示ID | チェックサム付きのsystem_id先頭短縮+筐体QR。電話サポート用の人間向け規約(D5決定1) | — |
| 親デバイス / 子デバイス | 中継機能を持つデバイス(例: BravePI Mainboard)とその配下の端末(最大20台)。親自身もDFU・パラメータ設定のコマンド宛先=デバイスの資格を持つ。南向きの宛先指定は「デバイス宛・アダプタ経由ルーティング」 | ルーター/端末 |

(識別モデルの正本は [decisions/D5-series-identity.md](decisions/D5-series-identity.md)。3層識別 system_id / hardware_id / user_label+2層series構造として確定。本節はD5反映 2026-07-02 で更新)

## プロダクト名

| 用語 | 定義 |
|---|---|
| IoTKit | IoTKit EdgeとIoTKit Siteからなる、オンプレミス優先のIoTデータ収集基盤 |
| IoTKit Edge | Raspberry Pi側の現場収集ノード。短縮はEdge。収集・正規化・耐久buffer・再送を担う |
| Edge Node | IoTKit Edgeが担うアーキテクチャ上の役割。センサーデータを収集・保全・配送するノード。`Node`単独では呼ばない |
| IoTKit Site | 単一拠点の集約、raw保存、Edge Nodeごとのcursor、query、設定可能なセンサー意味付け、application接続・export境界。短縮はSite |
| MQTT Broker | EdgeとSite間のQoS 1 transportを担う標準MQTT broker。IoTKitはBrokerを自作しない |
| YokaKit | 生産管理アプリ。別プロダクト。出口契約の一消費者。[3]/[4]に住む |
| iotkit-edgectl | 通常コマンドはR14制御プレーンを叩く人間/AI共用操作口。別にlocal-root maintenance系(初期所有権・admin recovery・factory reset)を持ち、これらは箱上の物理/SSH root専用でAPI/UI/AI/R14に公開しない |

## 関連文書

- 責務台帳: [responsibility-ledger.md](responsibility-ledger.md)
- 図解(ブラウザで開く): [diagrams/dataflow.html](diagrams/dataflow.html)(データの一生)、[diagrams/platform-comparison.html](diagrams/platform-comparison.html)(他プラットフォーム比較)、[diagrams/ai-connectivity.html](diagrams/ai-connectivity.html)(AI⇔Edge接続3パターン)
- リポジトリカタログ: [../../rewrite-prep.md](../../rewrite-prep.md)
