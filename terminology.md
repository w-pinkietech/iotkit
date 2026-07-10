# IoTKit / YokaKit 再設計 用語集

Status: 会話合意済み (living document)
Date: 2026-07-02

この文書は再設計に関わるすべての文書・コード・会話で使う統一語彙を定める。
ここにない語を新しく使うときは、まずこの文書に追加する。

## 配置の4段(「箱」の役割階層)

システムの構成要素は必ず次の4段のどこに住むかを明示する。
段の判定は**役割**で行う(2026-07-08 D10改訂——ホスト型サイトサーバーの導入により物理配置基準を廃止):
**保持する状態が単一の `site_id` に束縛されるなら[3]、site_idを跨ぐなら[4]**。物理配置は既定値
([3]=サイト内LAN)にすぎず、判定基準ではない。

| 段 | 用語 | 英語 | 実体 | 説明 |
|---|---|---|---|---|
| [1] | デバイス | device | BravePI無線センサー端末、直結I2Cセンサー、第三者の自作デバイス(ESP32/PLC等) | 測定・作動する末端。ゲートウェイの配下 |
| [2] | **IoTゲートウェイ**(外向き正式名。「ゲートウェイ」単独は内部略称) | IoT gateway / gateway | 現場に置くRaspberry Pi。IoTKit本体が動く | 責務台帳 R1〜R23 はすべてこの箱の責務。コード・APIの `gateway` / `gateway_identity` は不変 |
| [3] | **サイトサーバー**(旧称「工場サーバー」廃止 2026-07-08) | site server | **単一サイトの責務**(Archival Store/Aggregator/Console)を担う箱。既定は制御室等サイト内LAN。敷地外で運用する**ホスト型**変種あり(D8決定1・D10決定7の条件下で正式)。Standaloneでは不在可、Site-managed(Gateway Pi 2台以上)では必須(D8決定1) | 中の論理ロール: Archival Store(預かり)/Site Aggregator+配り用ブローカー(再publish)/Site Console(D8決定3)。AIオペレーター、オンプレYokaKitもここに住める。1インスタンス=1サイト(相乗り禁止、D10決定7) |
| [4] | クラウド | cloud | **site_idを跨ぐ上位層**(オプション)。商用クラウドに限らず、本社サーバールーム等もここ。※ホスト型[3]と同じデータセンターに同居しうるが、単一site_idに閉じるインスタンスは[3]である(役割基準) | クラウドLLM API、クラウドYokaKit、複数拠点統合・fleet管理・DR複製が住む。**サイト横断は必ずここでやる(サイトサーバー同士に上下関係を作らない)** |

### 禁止・注意語

- **「エッジ」を単独で使わない**。[1]か[2]か曖昧になるため。エッジデバイス/エッジサーバー等の複合語も避け、上の4用語を使う。
- **「サーバー」を単独で使わない**。[3]か[4]かを明示する。
- **「上流」(upstream)** = ゲートウェイから見たデータの届け先([3]または[4]にいる消費者)。「下流」(downstream) = ゲートウェイから見たデバイス側。
- **「ゲートウェイ」は常に[2]を指す。サイトサーバー[3]をゲートウェイと呼ばない**(2026-07-08命名レビュー反映)。
  業界にはIgnitionのように中央サーバーを"Gateway"と呼ぶ流派が実在するため、明示的に禁止する。
  また**BravePIメインボードはデバイス[1](親デバイス)であり、ゲートウェイではない**(BLE受信機のため誤呼称されやすい)。
- **対クラウド境界(「サイトの門」)は箱ではなく出口契約(R10)である**。StandaloneではIoTゲートウェイ1台に
  取り込み契約(入口)と出口契約(出口)の両面が載る([3]は不在)。「第二の門はどの箱か」という問いは立てない。

### 業界対応表(第三者向け)

| うちの箱 | 業界標準図での位置 | 各生態系での呼び名 |
|---|---|---|
| [2] IoTゲートウェイ | 「IoTゲートウェイ/エッジPC」 | AWS: Greengrass core device / SiteWise Edge gateway、Azure: IoT Edge gateway、Sparkplug B: Edge Node |
| [3] サイトサーバー | 「MQTTブローカー or IoT Hub」 | Sparkplug B: MQTT Server + Primary Host Application、ヒストリアン(PI Server)、**Ignitionの"Gateway"はここ(混同注意)** |

## 三本柱(再設計の一級要件)

| 柱 | 名称 | 一行定義 |
|---|---|---|
| 柱1 | 止まらないデータプレーン | 他の箱が全部死んでいても、ゲートウェイ単独でデータ収集・保全が継続する |
| 柱2 | オープンな接続契約 | 入口(デバイスをつなぐ)も出口(データを使う)も公開・版管理された契約。特権的な裏口は存在しない |
| 柱3 | AI運用可能な制御プレーン | 全運用状態が構造化データで読め、全運用操作が型付きカタログで叩ける。判定基準:「専門家がSSHなしで直せるなら、AIにも直せる」 |

## 中核概念

| 用語 | 英語 | 定義 |
|---|---|---|
| プロバイダ | provider | センサー/デバイスのベンダー技術体系(BravePI、BraveJIG等)。コアの語彙に漏れてはならない |
| ドライバ | driver | **物理の世界と話す**コード: transport(シリアル/I2C等の物理I/O)+プロトコルコーデック(南向きencode含む——コーデックは双方向)+センサーデコード(**データシートの数学**: 生値→物理量)。**取り込み契約を知らない**(ドライバSPIへの依存は許容)。「同じICはどこに繋がっていても同じドライバ」原則。iotkit-nextの `rpi4b-transport` / `bravepi-codec` / `iotkit-sensor-drivers` がこの層。※**OSのカーネルドライバとは別概念**。OS側を指すときは必ず「カーネルドライバ」と書く |
| ドライバSPI | driver SPI | ドライバが結果を返す先の型体系(`SensorReading` 等)。ドライバは取り込み契約を知らないが、この**第二の契約**には従う。名前と版管理を持つ(Home Assistantのライブラリ地獄の予防) |
| アダプタランタイム | adapter runtime | ドライバと契約クライアントを実際に走らせる**胴体**: スケジューリング(ポーリング/イベントループ)、検出・接続の状態機械、ドライバのライフサイクルとpanic隔離、**measurement写像**(measurement_key+channelへの写像=正規化の写像段。series解決はコレクタの仕事=D5)、南向きコマンドのディスパッチと有界ジョブ実行。契約もICも知らない。`iotkit-polling-adapter-runtime` が実例(ポーリング型)。BravePIの event_loop/serial_source はイベント駆動型の胴体 |
| 取り込みクライアント | ingest client | **取り込み契約と話す**コード(**北向き専用**): 正規形のエンベロープ化・envelope_id採番・送信・ack処理・(必要なら)spool。バインディング(プロセス内/UDS/HTTP/MQTT)を選ぶ。全アダプタで共有可能な1ライブラリ。ESP32ファームのHTTPクライアントと同種。南向きは別契約・別チャネル(D12。対名=世話サービサ) |
| アダプタ | adapter | **ドライバ+アダプタランタイム+契約クライアント(北=取り込みクライアント、南=世話サービサ、D12)の合成パッケージ**であり、供給者としての**説明責任単位**: 認証済み送信者として識別され、死活観測(R7/R12)の対象になり、南向き参加時は契約の宛先になる。形態4種: ①**公式アダプタ**(ゲートウェイ内プロセス)、②**衛星アダプタ**(同一コード別筐体)、③**外部アダプタ**(第三者製、任意の言語)、④**契約ネイティブデバイス**(ドライバ不要。ランタイム+クライアント=ファームウェアそのもの)。**監督(再起動権限、R20)は形態①のみの性質**——②③④は死活観測+検疫+エスカレーションのみ。②③④は**北向きについて**ゲートウェイから区別不能。南向き能力は能力宣言で申告される(推定しない)。※ネットワーク入口(R2)はアダプタではなくコレクタの玄関。翻訳責任は常に送る側 |
| 能力宣言 | capability declaration | アダプタ/デバイスが「何を測り(measurement_key×channel構成)、何のコマンドを受けられるか」をデバイス台帳(R7)に宣言する仕組み。南向きの宛先解決と、R9/R14の事前条件検証(南向き非対応デバイス宛のアクション設定を**設定時に拒否**)の基準。再アナウンス要求(redescribe)等の最小応答はack応答へのピギーバックで受動参加可能(ただしackを読まないfire-and-forget送信者=形態③④には届かないため、宣言の世代番号 declaration_version のエンベロープ同梱+コレクタ側版不一致検知で補完する=D5)。詳細はD12(未宣言動詞の拒否時点=D12決定2) |
| 北向き / 南向き | northbound / southbound | 北向き=デバイス→ゲートウェイのデータ方向(D1で契約確定)。南向き=ゲートウェイ→デバイスの機器の世話方向(D12で契約確定: 照会・構成・コマンド・有界ジョブ)。公式①・衛星②は双方向市民(②は第一波必須)、外部③は能力宣言による任意参加、契約ネイティブ④の南向き参加は保留(D12決定5) |
| コレクタ | collector | ゲートウェイ側の受理の権威。エンベロープの受理・重複排除・確認応答・逆圧を担う(R8) |
| エンベロープ | envelope | アダプタ/自作デバイス→コレクタ間の配送単位。安定ID(envelope_id)を持ち、再送しても安全 |
| 取り込み契約 | ingest contract | エンベロープをコレクタに届けるための公開ワイヤ契約。バインディングは複数(プロセス内チャネル / UDS / HTTP / MQTT ingest専用リスナー)だが論理契約は一つ |
| 正規化 | normalization | プロバイダ固有の生データ→測定レジストリ準拠の正規形への変換。アダプタ(=つなぐ側)の責任。**3段に分掌**: ①デコード(データシートの数学: 生値→物理量)=ドライバ、②measurement写像(measurement_key+channelへの写像)=アダプタランタイム、③現場較正(オフセット/倍率、現場設定の数学)=R9(コレクタ後段)。**series解決**(送信者アイデンティティ+subject_hint→台帳→system_id→series_id)は写像ではなく**コレクタの責務**(D5)。判定基準:「データシート由来の数学はドライバ、現場設定由来の数学はR9」 |
| 測定レジストリ | measurement registry | 測定種別・単位・型の語彙の定義と版管理(R6)。正規化の目標形を定める権威。**二層構造**: 命名の典拠=標準語彙カタログ、受理の正本=現場レジストリ(D6) |
| 標準語彙カタログ | standard vocabulary catalog | measurement_keyの命名・正準単位(UCUM)・値型・意味論クラス・物理限界値域・チャネル役割を定めるリポジトリ資産(ゲートウェイバイナリに同梱)。契約仕様書の一部として公開=柱2の実体。受理判定には直接使われない(診断・候補提示のみ)(D6決定1) |
| 現場レジストリ | site registry | ゲートウェイDB内の測定レジストリ正本(D2)。カタログから有効化(copy-on-enable=コピーして固定)したエントリ+現場カスタム定義(`custom.`名前空間)+エイリアス表の合成。R8受理判定の唯一の参照先(D6) |
| 出口契約 | egress contract | ゲートウェイ→[3]/[4]の消費者へのデータ公開契約(R10、**上流向き**——「北向き」はD1側の語であり出口には使わない)。複数消費者・消費者別カーソル・at-least-once。本体はD7 |
| 消費者 | consumer | 出口契約でデータを受け取る側。YokaKitは特権なしの一消費者 |
| パブリッシャ | publisher | 出口契約の送信側実装(ゲートウェイ内、outboxから配送する部品) |
| outbox | outbox | 配送待ちデータの永続バッファ。停電・長期断線を跨ぐ。上限と劣化契約を持つ(R17) |
| 制御プレーン | control plane | R12〜R15。ゲートウェイが提供する観測・診断・操作・設定のAPI面 |
| データプレーン | data plane | デバイス→取り込み→保全→出口配送のデータの流れ |
| 操作カタログ | operation catalog | 型付き・dry-run付き・権限段階付き・監査付きの運用操作の一覧(R14) |
| インシデントバンドル | incident bundle | 障害時にゲートウェイが生成する自己完結の診断パッケージ(症状+直近イベント+設定+ヘルス時系列)(R13) |
| desired / reported | desired/reported | 設定の「あるべき姿」と「実際に適用された姿」の分離(R15)。乖離は検出・報告・収束される |
| host-agent | host-agent | [2]内でアプリ本体から分離された特権操作プロセス。**sudo級特権操作のみ**(再起動・時刻設定・サービス制御等)。ハードウェアI/O(シリアル/I2C)はアダプタのドライバが直接扱い、host-agentは経由しない(レビュー反映 2026-07-02) |
| AIオペレーター | AI operator | [3]または[4]に住み、制御プレーンを叩いて診断・復旧を行うAIエージェント。ゲートウェイの一部ではない |
| runbook | runbook | AIオペレーター/人間が使う、機械可読な対処手順 |
| 有界ジョブ | bounded job | 長時間かかる操作(DFU、スモークテスト等)の明示的なライフサイクル(開始/進捗/完了/失敗)を持つ実行単位 |
| 監督 | supervisor | アプリレベルの監視・再起動・修復理由の記録(R20)。プロセスレベルはsystemd/HWウォッチドッグに委譲 |
| 時刻品質 | time quality | 全測定・イベントに付くタグ: synced / holdover / unsynced(R18) |

## 運用・配送の概念(整合性監査 2026-07-02 で追加)

| 用語 | 英語 | 定義 |
|---|---|---|
| 検疫 | quarantine | データ/デバイスを「保存・可視化(R11)はするが、下流配送(R10)とアクション駆動(R9)には使わない」状態に置くこと。値域外データ・未登録測定キー・登録直後デバイスに適用。解除は時限自動失効またはR14の操作(誰も解除できない誤検知を作らない)。※「隔離」という語は使わない |
| 正本 | source of truth | その情報の権威あるコピー。「その情報なしで動けなくなる箱が持つ」(D2) |
| custody transfer | — | 測定データの正本が配送とともにゲートウェイ→上流へ移転すること(D2) |
| アーカイブ責任消費者 | archival consumer | 出口契約の消費者のうち台帳で1つ指定。そのackのみが正本移転=パージ許可を意味する(D2) |
| AIハーネス | AI harness | AIオペレーターの実行環境(エージェントループ・runbook・ゲートウェイAPIツール・認証情報)。キットの提供物で[3]/[4]に住む。背後のLLMは差し替え可能 |
| operatorトークン | operator token | 制御プレーンを叩く運用主体(AIハーネス/人間)の認証情報。権限段階つき(D3決定5) |
| 劣化契約 | degradation contract | 資源上限到達時に「何をどの順で失うか」の事前合意(R17)。段階の具体(間引き/要約化/最古削除の順序)は設計スペックで確定 |
| spool | spool | 送信側がack受領まで保持する一時バッファ。耐久性は送信側の階級(メモリ/ディスク)に依存し、契約はそれを保証しない(D1) |
| dedup台帳 | dedup ledger | コレクタ側の重複排除記録。TTL+サイズ上限で有界(D1) |
| 衛星アダプタ | satellite adapter | ゲートウェイと別筐体で動かすアダプタランタイム。HTTPバインディングでコレクタに送る(D2 §4) |
| 接続状態機械 | connection state machine | 上流接続のonline/offline/degraded遷移管理。R10(再送)とR12(状態公開)に属する |
| time_source | — | 時刻の**出所**タグ: device_ntp / device_rtc / gateway / gateway_adjusted(D1)。時刻品質(確度: synced/holdover/unsynced、R18)とは直交する別タグ |
| publication_id | — | 出口契約の**バッチ再送の冪等キー**(消費者側dedup用)。レコード同一性ではない——同一性は `(epoch, seq)`(D7決定4) |
| record family | — | 出口ストリームのレコード種別タグ+スキーマ版。初版はmeasurement/annotationの2族、予約=文字列観測・時系列ブロック/波形・合成テスト。未知familyの読み飛ばしはoptional familyに限る(D7決定2) |
| publication log | — | 出口ストリームの採番権威。全record familyが共有する単調増加seq((epoch, seq)カーソルの実体)。readingsの内部挿入順とは別——検疫行は解除まで採番されない(D7決定4) |
| publication snapshot | — | 消費者再構築用の現在状態スナップショット(対応するseq水位を刻印)。R22スナップショット(高機密資産・readings非含有)とは**別語・別物**(D7決定8) |
| event_time | — | 出口レコードの正準イベント時刻。導出規則=device_time→age_ms復元(gateway_adjusted)→received_at、妥当窓検査は未来方向のみ(D7決定3)。観測時刻であり単調ではない——順序・カーソルには使わない(D7決定4) |
| event_time_source | — | event_timeにどの候補を採用したか+未来方向降格の有無を表す出口レコードのフィールド。time_source(入力の事実)とは別の、導出結果の表示(D7決定3) |
| target / target registry | — | 出口配送先の登録単位とその台帳。配送状態(カーソル・ack)はtarget単位で分離。登録・変更はR14型付き操作(D7決定6) |
| 購読フィルタ | subscription filter | targetが受け取るシリーズの選択(実体化series_keyで照合)。解釈ではなく選択。アーカイブ責任targetには適用しない(D7決定7) |
| 保管対象ポリシー | custody scope policy | アーカイブ責任消費者に託して長期保存するシリーズ範囲の台帳宣言(既定=全量)。対象外シリーズはcustodyの約束自体がなく、retention窓超過でcustody_lostにならず期限失効する(D7決定7) |
| 配送制御通知 | delivery control notice | 特定targetの配送状態についての通知(gap/cursor_expired等)。ストリームレコードではなくpushバッチのメタデータ(帯域外)で運び、カーソルを消費しない。全target共有のannotation族とは別レイヤ(D7決定2・6) |
| 目撃ステージング | sighting (staging) | 未知hardware_idの観測を有界・パージ可能に保持する登録前状態(hardware_id仮キーでデータ保持)。人間承認の瞬間に採番→検疫→active(D5決定4)。ステージング中のackは `disposition: staged`(D1監査追記) |
| retire(墓標) | retire / tombstone | 台帳エントリの削除に相当する終端状態。行は消さず、system_id再利用は永久禁止(D5決定4) |
| superseded_by | — | retire済みエントリから後継エントリへの参照。replace-hardware確定時に旧候補へ付与(D5ガードレール4) |
| replace-hardware | — | 個体識別型デバイスの交換時、台帳エントリのhardware_idだけを張り替えてseries(履歴)を継続させる明示操作。ガードレールはD5決定4 |
| 台帳エポック(世代番号) | ledger epoch | ゲートウェイの台帳の世代番号。箱交換(R22)を跨ぐ出口カーソル連続性とスプリットブレインのフェンス(D2 §3.5、D5決定3)。※D1の `boot_epoch`(デバイス起動カウンタ)とは**別概念** |
| value_semantics | — | seriesの値の意味クラス: `raw_legacy`(較正前生値)/ `calibrated`。R9較正の二重適用防止(D5) |
| 較正要再確認 | calibration review required | seriesの較正の信頼を保留する状態(交換疑いシグナル・replace確定時)。検疫との違い: データは流れるが較正の信頼が保留されている(D5決定2) |
| 最低保持フロア | minimum retention floor | アーカイブ責任消費者のack後もゲートウェイに置く最低保持期間(目安72h、設定可)(D1・責務台帳。旧称「パージフロア」は廃止) |

## サイトトポロジ(複数ゲートウェイ。D8 2026-07-07)

| 用語 | 英語 | 定義 |
|---|---|---|
| Standalone | standalone | サイト内のGateway Piがちょうど1台の構成。YokaKit同梱可、上流接続・site server任意(D8決定1) |
| Site-managed | site-managed | Gateway Piが2台以上でsite server[3]を必須とする構成。各PiはいずれもローカルSQLite/collector/出口を持つ完全ゲートウェイ(D8決定1・2) |
| gateway_identity | — | Gateway Piの安定した外部同一性。初回自己構成で1回だけ生成し、共有イメージには焼き込まない。消費者側の大域レコード同一性 `(gateway_identity, epoch, seq)` の先頭成分(D8決定5)。台帳エポックとは別概念 |
| Site Aggregator | site aggregator | site server内の**非権威**ロール。各PiのR10を読み投影・統合表示・運用管理する。custody transferしない(D8決定3) |
| Archival Store(アーカイブ責任) | archival store / archival consumer | site server内の**custody受け手**ロール。各PiからR10 raw streamを受け耐久保存し、archival ackを返す。このackだけがPiのpurgeを許可(D8決定3。D2のアーカイブ責任消費者——ゲートウェイの台帳で指定する上流の預かり先——をsite server[3]に置いた形) |
| archive_repair_hold | — | site archive損失/修復の検知中、対象 `gateway_identity`・範囲のGateway Pi purgeを修復完了まで止める保留フラグ。backfillで送り直すべき範囲を先に消さないため(D8決定4) |
| active epoch台帳 | active epoch registry | site serverがGateway Piごとの現行epochを永続保持する台帳。stale epochは大小比較でなくこの台帳との一致で判定(RTCなし前提。D8決定5) |
| archive_lost | — | Pi purge済みかつsite archive損失かつbackupなしの範囲に付す監査イベント。Gateway Piの `custody_lost` ではなくsite側の責務損失として区別(D8決定4) |
| Site Console | site console | site server上の統合運用UI。gateway enrollment・alarm集約・snapshot vault・update orchestrationの操作面(D8決定1・8) |

## 出口MQTTバインディング(D9 2026-07-08)

| 用語 | 英語 | 定義 |
|---|---|---|
| 出口MQTTバインディング | exit MQTT binding | 出口契約(R10)の第一波バインディング。IoTゲートウェイがtargetのMQTTエンドポイントへ有界バッチをpublish(QoS1)し、ackを「しまってから返すPUBACK」+補助topic明細で受ける(D9) |
| 送信窓 | sending window | 未ack(未PUBACK)のin-flightバッチ数の上限。窓が埋まったら新規publishを止めoutboxに滞留(D9決定7) |
| 補助topic(ack明細) | ack detail topic | `ack/{gateway_identity}` 上でArchival Storeが返す明細。`accepted_through` 水位(正式なpurge水位)・終端通知を運ぶ。target_id/publication_id/epoch相関必須、retained禁止、接続時に再同期(D9決定2・3) |
| 終端通知 | terminal notice | 再送しても結果が変わらない失敗(決定的契約違反・custody_conflict)をゲートウェイへ伝える補助topicメッセージ。受けたバッチはoutbox隔離+operator解決(D9決定3)。一時的ストレージ失敗には使わない |
| 非預かりターゲット | non-custodial target | 市販ブローカー等、custodyを移転しない出口先。PUBACKは配達確認どまり、逆圧・カーソル・gap通知は構造的に失われるベストエフォートの配り(D9決定5) |
| 一級target / 購読者 | first-class target / subscriber | 消費者の二層。一級target=契約対応リスナー(store-then-ack)を実装しカーソル・完全性保証を受ける消費者。購読者=配り用ブローカーをsubscribeするだけのベストエフォート消費者(D9決定8) |

## 出口認証(D10 2026-07-08)

| 用語 | 英語 | 定義 |
|---|---|---|
| enrollment台帳(名簿) | enrollment ledger | site server[3]が保持する登録台帳: `gateway_identity`・鍵/証明書fingerprint・ledger epoch・site所属・credential束縛レコード。D8決定5の一意性検証・active epoch台帳と同居(D10決定1) |
| 登録券 | enrollment ticket | 新Gateway Piを名簿に載せる単回使用のprovisioning束 {接続endpoint, サーバー公開鍵ピン, site_id, 単回使用秘密, 短TTL}。共有イメージ焼き込み禁止、人間承認必須(D10決定2) |
| 束縛credential | bound credential | Pi・targetごとに1つの資格情報。`gateway_identity + target_id + target_endpoint_id + pinset + scope` へ束縛(URL文字列には束縛しない)。共有credential禁止(D10決定1) |
| 2スロット(make-before-break) | two-slot rotation | targetごとにcredentialを2枠持ち、「新発行→疎通スモーク成功→旧失効」の順で更新する方式。スモーク成功が旧失効の事前条件(D10決定3) |
| 無人再発行 | unattended re-issuance | 期限切れcredentialの非常口。箱から出ない鍵(登録済みfingerprintの鍵ペア/トンネル鍵)で認証→名簿照合→自動再発行+監査。人間の関与不要(D10決定3) |
| 中間層 / 日常層 / 工事層 | routine / daily-tap / construction tier | 変更操作の権限3分類(D12決定3で正式化。照会はread-onlyスコープとして別軸)。**中間層**=AI可・必須条件つき(出口credential rotation・失効・無人再発行・トンネル鍵rotation=D10決定5、南向きの世話の一部=D12決定3。一括操作は昇格)。**日常層**=人間のタップ承認・AIは提案まで(device add・ペアリング窓・デバイストークン失効=D1/D11、較正確定・アラーム意味論・親再起動・DFU承認・流量クラス変更=D12決定3/D11決定8)。**工事層**=構造・経路の変更(target追加・削除、cloud target登録、archive designation変更、平文opt-in、enrollment承認、入口リスナー有効化/bind変更。人間のみ、AIトークンには構造的に発行不可)(動詞集合の正本=D10決定5・D11決定8・D12決定3) |
| ホスト型サイトサーバー | hosted site server | site server[3]のソフトウェア一式を敷地外(クラウド/VPS)で運用する正式変種。成立条件(トンネルMUST・WAN断耐久宣言・purge自動保留・VPS外DR・相乗り禁止)はD10決定7 |
| 経路クラス規則 | path-class rule | 守りの強度を箱の設置場所でなく経路で決める規則。[2]↔[3]の全プレーンは、LAN内なら「廊下」ルール、インターネットを渡るならピン留め静的鍵トンネル内MUST(D10決定7) |
| トンネル鍵 | tunnel key | ホスト型の[2]↔[3]トンネル(WireGuard等)のピア鍵。Pi上で生成し公開鍵のみ名簿登録、期限なし(有効性=名簿照合)、rotation=中間層2スロット、失効=peer除去+MQTTセッション切断連動(D10決定7) |
| credential_health | — | アラーム(旧称 `certificate_expiry`)。証明書・target資格情報・operator tokenの期限・rotation失敗・ピン不一致・2スロット片肺(D10) |
| site_unreachable | — | ホスト型のアラーム。archival storeへの全Pi一斉不達(WAN断/トンネル断/VPS障害)。LAN内部分分断の `partial_partition` とは別事象(D10) |

## 入口認証(D11 2026-07-08)

| 用語 | 英語 | 定義 |
|---|---|---|
| 流量クラス | rate class | デバイス登録時に申告する想定流量の粗い段階(既定クラスあり)。容量設計=現場エンジニアの責任、執行=ソフトの責任、という分担の実体。クラス変更は人間のみ(D11決定4・8) |
| 絞り | throttle | 流量クラス超過分を**非終端**の応答で退けるシステム自動執行。HTTP=429+Retry-After(耐久ackなし)、ack語彙上は `deferred`——終端 `rejected` には決して写像しない(spool持ち送信者のデータ破壊防止)。可逆・ヒステリシス付き自動解除・騒がしく(アラーム+R23+監査)(D11決定4) |
| 対応の階段 | response ladder | 入口の事故対応の順序: 絞る(自動)→検疫(自動・既決)→トークン失効(人間のみ)。自動対応は必ず騒がしく行う(D11決定4) |
| ペアリング窓 / 登録コード | pairing window / registration code | デバイス登録の儀式(D1既決)。登録コードはD10登録券の縮小版(単回使用・短TTL・窓内のみ有効)。窓は自動クローズ、開けっ放し禁止(D11決定6) |
| 入口リスナー既定オフ | ingress listener off-by-default | ネットワーク入口(HTTP/MQTT ingest)は既定で無効。有効化・bind変更・プロトコル追加は独立した工事層操作(device addの暗黙副作用にしない)。インターネット公開は禁止——遠隔地からのデータは別のIoTゲートウェイ[2]+出口契約で運ぶ(D11決定7) |
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

## ゲートウェイUI(D13 2026-07-08)

| 用語 | 英語 | 定義 |
|---|---|---|
| setupモード | setup mode | 未コミッショニング箱のUI開放窓(家庭用ルーター同型)。可能な操作は閉集合(初期設定・デバイス登録・リスナー有効化のTLS/LAN限定例外・検疫解決・ライブ値閲覧)、工事層と上流接続は開かない。管理者パスフレーズ設定で閉じる。監査actor=`setup_mode`。閉じ圧力=常時警告+Phase 6不合格+出口target登録不可(D13決定2) |
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
| 親デバイス / 子デバイス | 中継機能を持つデバイス(例: BravePIメインボード)とその配下の端末(最大20台)。親自身もDFU・パラメータ設定のコマンド宛先=デバイスの資格を持つ。南向きの宛先指定は「デバイス宛・アダプタ経由ルーティング」 | ルーター/端末 |

(識別モデルの正本は [decisions/D5-series-identity.md](decisions/D5-series-identity.md)。3層識別 system_id / hardware_id / user_label+2層series構造として確定。本節はD5反映 2026-07-02 で更新)

## プロダクト名

| 用語 | 定義 |
|---|---|
| IoTKit | ゲートウェイ[2]で動くOSSプラットフォーム本体(本再設計の主対象) |
| YokaKit | 生産管理アプリ。別プロダクト。出口契約の一消費者。[3]/[4]に住む |
| gatewayctl | 制御プレーンを叩くCLI。人間とAIの共用操作口 |

## 関連文書

- 責務台帳: [responsibility-ledger.md](responsibility-ledger.md)
- 図解(ブラウザで開く): [diagrams/dataflow.html](diagrams/dataflow.html)(データの一生)、[diagrams/platform-comparison.html](diagrams/platform-comparison.html)(他プラットフォーム比較)、[diagrams/ai-connectivity.html](diagrams/ai-connectivity.html)(AI⇔ゲートウェイ接続3パターン)
- リポジトリカタログ: [../../rewrite-prep.md](../../rewrite-prep.md)
