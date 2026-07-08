# D1: 取り込みモデル(論点1)— レビュー修正版

Status: 確定 (2026-07-02、4レンズ専門レビュー済み・ユーザー承認済み)
レビュー: 配送保証 / Rust実装 / 自作デバイスDX / セキュリティ(いずれも骨格承認+修正指摘)

用語は [../terminology.md](../terminology.md)、責務は [../responsibility-ledger.md](../responsibility-ledger.md) に従う。

## 決定の骨子(4レンズとも承認)

1. **正規化はつなぐ側**(アダプタ内/デバイス作者)。コアは生バイト列を受けない。中央変換エンジンは置かない。
2. **論理契約は一つ**: エンベロープ+確認応答(accepted/duplicate/rejected/deferred)、at-least-once+重複排除。
   MQTT QoS1+アプリ層dedup、Kafka冪等プロデューサと同型の実証済みパターンであり再発明ではない。
3. **バインディング複数**: (a)プロセス内チャネル (b)UDS (c)HTTP。意味論は同一。
4. **障害隔離は第一波ではタスク監督**(panic捕捉+アダプタ単位再起動)。プロセス分離は契約変更なしで後から可能。

## 修正事項(レビューによる契約意味論の精密化)

### ackと配送保証

- **ack = 耐久点**。ackは「コレクタがSQLiteにコミット完了した」ことを意味する。チャネル送信成功・HTTP 200受信開始は
  ackではない。ゲートウェイは書き込み成功後にのみacceptedを返す(書き込み失敗のログ握り潰し禁止)。
- **rejectedは終端**(送信側はspoolから除去し記録)、**deferredは同一エンベロープを不変のまま再試行**。
- **deferredは一時的過負荷専用**。持続的なストレージ逼迫にdeferredを使うと損失が最弱ノード(ESP32のRAM)に
  転嫁され柱2が破綻する。持続逼迫はゲートウェイ側retention(最古削除/間引き=R17劣化契約)で吸収する。
  水位はヒステリシス2閾値。
- **再送はバックオフ+ジッタをMUST**(停電復帰時の全デバイス同時再送ストーム対策。工場では停電復帰は日常)。
- 送信側の再送責任は**SHOULD**(軽量プロファイル: fire-and-forgetの投げっぱなしも一級市民として許容。
  ゲートウェイ側の冪等性=dedupは常に保証)。デバイス側flashへの毎サンプルspool書き込みは禁忌(wear死)。
- **アダプタのcursor規則**: ソース側の消費確定(デバイス内バッファクリア等)はコレクタack受領後のみ。

### envelope_id と重複排除

- **再送時にIDを再生成しない(MUST)**。spoolにID込みで保存。
- **dedupキー = (認証済み送信者アイデンティティ, envelope_id)**。クライアント申告IDを単独キーにしない
  (なりすまし/設定ミスによる影書きで本物のデータがduplicate扱いで消える事故の防止)。
- **時刻ベースID(UUIDv7含む)を主キーにしない**(時計の狂ったデバイスで破綻)。
  推奨プロファイル: `sender_id + boot_epoch(NVS永続起動カウンタ+起動時nonce) + 単調seq`。
  プロセス内アダプタはUUIDv4可。seqは任意フィールドとして公開(順序復元の副産物)。
- **dedup実装**: 物理表現は独立の `ingest_dedup` テーブルに一本化(D5波及 2026-07-02: バッチ対応のため
  測定テーブル直UNIQUEは不成立)。ingest_dedupへの挿入と測定書き込みは**同一トランザクション**
  (耐久化と重複判定の原子性=ack耐久点性質を維持)。TTL付き(目安72h)+サイズ上限つき=DoS対策。
- **契約に明記**: dedup保証はウィンドウW内のみ。Wを超える再送は重複しうる。下流消費者も冪等に受ける
  (耐久ホップごとにdedup)。送信側の再送地平線≦W。

### 時刻

- **デバイス時刻はオプショナル**。「時刻がないからrejected」は禁止。
- **二本立て**: デバイス申告時刻(あれば)+`received_at`(コレクタが必ず付与)。
- `time_source` タグ: device_ntp / device_rtc / gateway / gateway_adjusted。
- RTCなしデバイス向けに `age_ms`(送信時点からの経過ms)を許可し、受信時刻-age_msで復元。

### バッチと順序

- **バッチは初日からワイヤ形式の一級市民**(電池駆動デバイスの現実)。単発=長さ1のバッチ。
- **ackはエンベロープ単位のステータス配列**(部分受理。all-or-nothingは毒バッチ=無限再送ループを生むので禁止)。
- バッチサイズ上限(件数・バイト)+超過用reject理由コード。
- **到着順序は未規定**と明文化。順序が必要な消費者は(source, timestamp, seq)で並べ直す。
- 順序依存の判定(閾値エッジ検出)はアダプタ内(ソース順が見える場所)で行うか、
  timestamp+seq並べ直し+小さな遅着許容窓で行う。**遅着タイムスタンプを既定で拒否しない**
  (省電力センサーのバックログは常に「古い」)。ただし鮮度ウィンドウ(例24h、設定可能)超は拒否。
  **【実装状況 2026-07-03、ユーザー裁定】鮮度ウィンドウ超の拒否はWave 1実装**(外部送信者導入と同時)。
  Wave 0の実アダプタは `device_time=None` **かつ `age_ms=None`** 決め打ちで、遅着した過去時刻を一切送らない
  (`bravepi-mainboard-adapter` と `iotkit-polling-adapter-runtime` の両ingest_mapで確認済み)ため未実装でも実害なし。
  event_time導出の候補2(=`received_at − age_ms` による過去復元)も `age_ms=None` で到達不能。
  D7決定3(event_timeの過去方向に妥当窓なし)はこの拒否を前提とする——Wave 1で両者を同時に満たす。

### セキュリティ(第一波必須)

- **per-deviceトークン**(共有キー禁止)。ゲートウェイはハッシュ保存、失効はデバイス単位、
  スマホからワンタップで失効+再発行(トークンは漏れる前提の設計)。
- **subjectスコープ認可**(D5波及 2026-07-02、旧称seriesスコープ認可): トークンに書き込み可能な
  **subject集合(system_id集合)**をバインド。series粒度はsubjectから導出(D5)。
  越境試行(解決先が他送信者の既知subject)は拒否+侵害シグナルとして監査(受理判別表はD5決定4)。
- **TLSデフォルト**: ゲートウェイ自己署名証明書+オンボーディング時フィンガープリント配布(ピン留め)。
  プライベートCA基盤は作らない。平文HTTPは明示opt-in+警告ログでのみ許可(**プライベートアドレス限定**——
  D11決定2追記 2026-07-08。WAN越えの平文入口は構成として拒否)。per-device mTLSは採用しない。
- **DoSハードニング**: 認証検証をボディ読み込み前に(未認証は確保ゼロで即401)、ボディ上限(例64KB)、
  タイムアウト、同時接続上限、per-deviceレート制限+グローバル上限(429+Retry-After)、
  ingress→コアは有界キュー(満杯は503シェッド)、dedup台帳のTTL+サイズ上限。
- **物理アクション前の検証ゲート**: 接点出力等は「検証済み・値域内・鮮度OK」データのみ+N連続/持続時間条件。
- **値域検証**: series契約でmin/max/単位を宣言。範囲外は破棄せず検疫フラグ付き保存(下流に流さない)。
  検疫は時限自動失効またはAI/人間による解決を必須に(誰も解除できない誤検知を作らない)。
- **オンボーディング**: 時間制限付きペアリングウィンドウ+デバイス上のワンタイム登録コード+人間のタップ承認
  (AIは提案まで)。登録直後は検疫ステート(データは見えるがアクションを駆動しない)。
- 契約だけ予約: トークンkey_id(ローテーション)、HMACリクエスト署名、per-deviceクォータ、単調seqの厳格検証、失効理由コード。
  (per-deviceクォータ=流量クラスの申告制と絞りの階段、失効理由コードはD11決定3・4で具体化 2026-07-08。
  key_id/HMAC/単調seqは高脅威現場向けopt-in予約のまま)

### 開発者体験(第一波必須)

- **curl 3行クイックスタート**を成立させる(デバイス登録1コマンド→トークン即発行→curlで送れる)。
- **rejectedの理由を届ける**: reason_code+人間可読message+field_path+期待スキーマ断片(近い候補の提示)。
- **dry_run/validateエンドポイント**(認証込み・副作用なしの試行)+ゲートウェイUIのライブインスペクタ
  (直近の受信とreject理由の生ログ。試行錯誤の速度が体験を決める)。
- **未登録測定キーは検疫として受信・保管**し、UIで後付けマッピング(「'temp'を'temperature_c'に対応付け」)。
  デバイス側正規化は理想として維持しつつ逃げ道を用意。
- **リファレンス実装の配布**: envelope_id生成レシピ、再送・バックオフ、時刻タグを吸収するクライアントライブラリ
  (Arduino/ESP-IDF/MicroPython)。Tasmota/ESPHome/Node-RED向けの接続ガイド。

### MQTTバインディング(方針決定)

- 契約のack意味論はMQTT QoS1のPUBACKと構造的に同型 → **バインディング(d)としてMQTTリスナー内蔵を正式採用**。
  ただしフルブローカーではなく**ingest専用リスナー**(デバイス間pub/subは責務外)。
  認証はHTTP側と同一クレデンティャル(username=device_id, password=トークン)。
- 位置づけ: 第一波の契約定義にMQTTマッピングを含める。実装はHTTP ingress完成直後の最初の追加バインディング。
  Tasmota/ESPHome等の既存ファームウェアがファーム書き換えゼロで繋がることが「持ち込み第一級」の証明になる。
- 「なぜ内蔵フルブローカーにしないか」は設計判断として記録(運用複雑性の回避、ingestに限定)。

### Rust実装の地雷(実装計画に織り込む)

- `SensorReading.labels: Vec<&'static str>` → `Vec<String>` へ変更必須(serdeデシリアライズ不可のため
  ネットワークバインディング導入前に。全ドライバに波及するが機械的)。
- プロセス内バインディングは `mpsc<IngestRequest{envelope, ack_tx: oneshot}>`(actor request-reply定石)。
  trait統一はしない。契約意味論の同一性は**共有適合テストスイート**(同一シナリオを全バインディングに流して
  ack列を検証)で担保。UDSとHTTPは同一axum Routerを2リスナーでserve(axum 0.8以降)。
- `Deferred`はプロセス内では返さない(mpscのawaitが逆圧)。HTTP/UDS専用の意味論。
- プロセス内クライアントは**「ackなし」と「チャネル閉鎖」を型で区別**する(2026-07-03 実装還流):
  ack用oneshotのドロップ=未耐久・再送対象(コレクタは生存)、mpsc閉鎖=コレクタ死亡・監督対象。
  同一エラーに潰すと一過性ストレージ失敗がプロセス再起動に化ける(計画2で実証、
  `SubmitError::NoAck/Closed` で修正済み)。
- **adapter_idの出所をチャネルキーからエンベロープ自身(`source`)に移す**(HTTP ingressは1チャネルに
  多sourceを多重化するため。エンベロープは自己記述に)。
- タスク監督: async側はJoinSet+join_next(catch_unwind不要)、`panic="abort"`禁止(CIチェック)、
  グローバルpanic hookでbacktraceログ。DbHandleのMutex毒性を解消(parking_lot or into_inner回復)。
  blocking I/O(serial/I2C)は必ずread timeout付きループ(スレッドは殺せない。exclusive openのEBUSY対策も兼ねる)。
  再起動は指数バックオフ+jitter+上限、N回超で永続degraded+イベント発行。
- **SQLiteの耐久設定(D8波及 2026-07-07)**: custody-criticalなトランザクション——readings正本を書くコレクタ、
  およびarchival ack水位・publication logを書く出口側——は `WAL + synchronous=FULL` を**MUST**とする。
  `synchronous=NORMAL` は電源断でWAL最終同期以降のコミット群が巻き戻りうるため、
  **再構成可能なderived/retryメタデータ(表示キャッシュ・health cache等)に限る**。
  `FULL` のRPi上での書込量・group commit要否はD8保留の実測項目。
- ackタイムアウトをHTTPハンドラに必須(コレクタ詰まり→ハンドラ滞留→メモリ膨張の防止)。

## 移行フェーズ(iotkit-nextから)

1. labels型変更+Envelope/Ack型導入+envelope_id列+hardware_id/measurement_key構造化分離(D5)
   (ackはダミー全accepted。dedupの物理表現は1.5のingest_dedupに一本化)
1.5. 台帳・seriesテーブル(devices/series/ingest_dedup+sensor_readings v3)+コレクタ内台帳解決
   (D5「Wave 0実装への接続」参照。D5波及 2026-07-02で挿入)
2. チャネル型切替+ack順序修正(書き込み後ack)+JoinSet監督
3. axum ingressクレート追加(擬似アダプタとしてfan-inに1ストリーム登録)+セキュリティ最小セット
4. MQTTリスナー(バインディングd)

各フェーズでworkspaceテストが通る粒度に切る。engine/projection、polling runtimeの状態機械、
AdapterHost/StreamMapの骨格は温存。

## 第一波で凍結すべきもの(後から変えると契約破壊)

ackの意味(耐久点)と4語彙、rejected/deferredの送信側義務、envelope_id不変性とdedupキーのスコープ、
dedupウィンドウの存在、バッチのワイヤ形式と部分ack、順序無保証、時刻二本立て+time_source、
subjectスコープ認可(トークンに書き込み可能なsubject集合をバインド。series粒度はsubjectから導出=D5)、
エンベロープ必須フィールドの命名(CloudEventsに寄せるか検討)、
measurement_keyの文法(文字集合・ドット名前空間・**コロン禁止**・長さ上限=D6決定2)。

## 監査追記(2026-07-02 厳格レビュー反映)

- **「凍結」→「安定意図(stable-intent)」に格下げ**(D3決定2): 最初の外部消費者/デバイス作者が現れるまで、
  破壊的変更は移行ノート付きで可。
- **AIオペレーター経由のプロンプトインジェクション対策**(セキュリティ節に追加):
  - operatorトークン(AI用)は物理アクション権限段階に**昇格不可**。物理アクションを伴う操作は人間承認必須。
  - インシデントバンドル・診断出力・reject理由に含まれる**攻撃者可制御文字列**(user_label、ペイロード断片等)は
    データとして明示的にタグ付け/引用符化し、指示文として解釈されない構造にする。
- **最低保持フロア**(用語統一 2026-07-02: 旧「パージフロア」表記を廃止、台帳と統一):
  アーカイブ責任消費者のack後も最低保持期間(目安72h、設定可)を置く。
- **ハードウェア最低要件**: 高耐久SD(産業用)/eMMC/USB SSDを導入文書に明記。
  Wave 1受け入れ基準に**電源断反復試験**(RPi+スマートプラグの自動リグ)を入れる。
  R16「電源断は正常系」は実測なしに主張しない。
- **deferredの精密化**(骨子3の注記): 意味論は同一だが、deferredは逆圧をチャネルawaitで表現できない
  バインディング(HTTP/UDS)でのみ顕在化する。
- **MQTT 3.1.1の制約**: PUBACKはreject理由を運べない。rejected詳細は `ack/{device_id}` トピックで補完
  (fire-and-forget派は購読不要)。「QoS1と同型」はaccepted経路について真。
- **非有限値は決定的契約違反**(2026-07-03 実装還流): ワイヤのf64値は有限必須(NaN/±Inf禁止、
  D6決定10の精密化と対)。非有限は `value_type_mismatch` で終端拒否する。ackなし扱いにすると
  耐久点直前で毎回失敗し、悪意なきデバイス1台が恒久再送ループを作る(計画2で実証・修正済み)。
- **Wave分割の読み替え**(D3決定1): 本文書の「第一波必須」は「契約定義v1に含める」の意味。
  Wave 0で実装するのはプロセス内バインディング+dedup+ack耐久点+監督のみ
  (一号現場に第三者デバイスは存在しないため、トークン/TLS/DoS/オンボーディングUIの実装はWave 1)。
- **ackのdisposition**(D5波及 2026-07-02): acceptedに `disposition: durable | staged` を導入。
  stagedは**未承認subject宛**(目撃ステージング中)のaccepted: 耐久保存済みだが、承認されなければ
  有界期限後にパージされうる(期限はackで通知可能)。パージ後もdedup台帳エントリはTTLまで保持する
  (再送でデータが復活しない)。durableのack=耐久点の意味は不変。
- **dedupの物理表現の一本化**(D5波及 2026-07-02): `ingest_dedup` テーブルに一本化
  (バッチ対応のため測定テーブル直UNIQUEは不成立)。ingest_dedup挿入と測定書き込みは
  同一トランザクション(耐久点性質を維持)。本文「envelope_idと重複排除」節は修正済み。
- **ack構造の2階層明記(2026-07-02、外部レビュー指摘反映)**: 「ackはエンベロープ単位のステータス配列」の
  精密化——ackは**2階層**である。(1) バッチ(=エンベロープの配列)へのackは**エンベロープ単位**の
  ステータス配列(dedup・終端判定の単位=エンベロープ)。(2) acceptedされたエンベロープの内部は
  **item単位**のステータス配列で部分受理を表現し、**入力itemsと同数・同順(位置整列)**とする。
  明示的な `item_index` フィールドは採らない(同数・同順の契約で十分、冗長フィールドは不整合の温床)。
  規範表現は `iotkit-ingest-contract` クレートの `EnvelopeAck`/`AckStatus::Accepted{items}`/`ItemStatus`。
- **D6波及(2026-07-02)**: ack dispositionを `durable | staged | quarantined` の3値に拡張
  (quarantined=未知measurement_key等の検疫受理。受理判別表はD6決定6)。reason_codeに
  `value_type_mismatch` / `malformed_measurement_key` を追加。measurement_key文法(D6決定2)を
  上記安定意図リストに追加済み。
- **送信者アイデンティティの正準化(2026-07-03、外部レビュー第2回反映)**: dedup・subjectスコープ認可の
  送信者IDは常に**認証主体**から導出する(HTTP/MQTT=トークンが指すdevice_id、プロセス内=source申告が主体)。
  `Envelope.source` は診断用の自己記述。認証付きバインディングで認証主体とsourceが不一致の場合は
  **エンベロープ単位でrejected+侵害シグナル監査**(なりすまし・設定ミスの早期可視化。実装はWave 1のHTTP ingress)。
- **dispositionの直列性(2026-07-03、同上)**: `staged` と `quarantined` は**同時に成立しない**——
  subject解決が常に先であり、未知subjectのitemはレジストリ判定前にstagedへ入る(レジストリ判定は
  承認時の本流化で実施)。優先順位: staged(subject未承認)> quarantined(series/行検疫)> durable。
  検疫理由の可視化として `ItemStatus::Stored` に任意フィールド `quarantine_reason`
  (out_of_range / unknown_key / undeclared_channel / device_quarantined。D6判別表と1:1)を追加する
  (契約へはadditive。実装はレジストリ実装と同時)。
- **Rejectedの適用範囲の精密化(2026-07-03、実装レビュー反映)**: `rejected` は**決定的な契約違反**
  (文法違反・値型不一致・認可違反・バッチ上限超過等、再送しても結果が変わらないもの)にのみ使う。
  **ストレージ起因の失敗(コミット失敗を含む)ではackを一切返さない**——ackなし=未耐久のシグナルであり、
  送信側はタイムアウト後の再送で回復する(rejectedは終端=spool除去のため、未耐久データに使うと無音損失になる)。
- **time_qualityはエンベロープに載せない(2026-07-03、同上)**: R18の時刻品質(synced/holdover/unsynced)は
  **受信側(ゲートウェイ)が自分の時計状態を評価して刻む**受信時メタデータであり、送信者は主張できない。
  readings v3に `time_quality` 列を持つ(Wave 0は固定値 `unsynced`=品質を過大主張しない保守既定。
  NTP状態評価の実装はWave 1)。デバイス側の時刻申告は既存の time_source / age_ms / device_time で表現する。

## 後回しでよいもの

watermark式dedup最適化、TTL/水位の具体値、プロセス分離、Sparkplug B、HMAC署名の実装、
per-deviceクォータの実装、ESP32のflash spool(非推奨のまま)。
