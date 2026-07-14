# D7: 出口契約(R10/R11)— 上流向き公開契約面

Status: 確定 (2026-07-03。2026-07-13 MQTT binding簡素化をD9へ反映)
用語は [../terminology.md](../terminology.md)、責務は [../responsibility-ledger.md](../responsibility-ledger.md) に従う。
設計キュー3への回答。入力 = [../inputs/2026-07-03-yokakit-consumer-catalog.md](../inputs/2026-07-03-yokakit-consumer-catalog.md)
(YokaKit実コードからの消費者ニーズ棚卸し)。保留ADR 0028/0029/0032/0035 および 0030(出口側)を本決定で統合する。

方向語彙の注意: 本契約は**上流向き**(Edge→[3]/[4]の消費者)。「北向き」はデバイス→Edge
(D1側)の語であり、本文書では使わない。

## 決定1: 出口契約の本体 = 生レコードストリーム一本

コア出口(R10)は正本(SQLite)のレコードの**順序付きストリームのみ**を運ぶ。
意味付け(ダウンタイム判定・OEE計算・アンドン状態・段取り替え遷移)は全て消費者側の導出責務であり、
コアは派生イベントを作らない。

根拠:

- **プラットフォーム中立性(第一根拠)**: IoTKitは工業専用システムではなく「IoTで現場を改善したい
  人たちの土台となるプラットフォーム」(ユーザー言明 2026-07-03)。工場語彙(ダウンタイム/OEE)を
  コア契約に入れない理由はドメイン中立性であり、現消費者の実装都合ではない。
- 実在消費者の証拠: YokaKit実物コードは完成イベントを消費していない——設備停止は沈黙検知、
  段取り替え終了はカウント到着による状態遷移で、いずれも生カウント+タイムスタンプから自力導出
  (カタログ§1稼働状態・§5)。「沈黙したら停止」の閾値は業務ルール(YokaKit工程マスタ設定)であり、
  コアに持ち込むと特定アプリの設定をコアが知ることになる。
- オフライン再送(柱1)と完成イベントは相性が悪い: 沈黙が設備停止か回線断かはEdge自身にも
  区別困難。生レコード+タイムスタンプなら遅着でも消費者が正しく再構成できる。
- ADR 0028「YokaKit publisher = 投影アダプタ、投影語彙のコア化禁止」原則と一致。YokaKit語彙
  (gantt-chart/onoff/production等のトピック名=UI機能名・テーブル名の逆引き)はコア契約に
  持ち込まない(カタログ§6のチェックリストを契約設計の禁止例として参照)。

**検疫データの配送規則**(用語集・D5との接続): 検疫中の行は配送しない(検疫は保存・可視化=R11のみ。
用語集どおり)。**検疫解除時、解除された行はpublication log(決定4)に新規採番されて通常のmeasurement
レコードとして流れる**(=D5「解除は新規配送イベント」の実体)。あわせて検疫遷移annotation(決定2)で
「どのseriesのどの範囲が有効化されたか」を通知する。遡及検疫(既配送データの事後隔離)は
annotationのみ(データは既に配送済みのため回収はしない——消費者側の扱いはannotationを見て判断)。

## 決定2: record family枠組み(前方互換の骨格)

ストリームの全レコードに **family識別子+スキーマ版** を付ける。

- **読み飛ばし規則(限定付き)**: 未知familyの読み飛ばしが許されるのは**追加的(optional)な
  familyに限る**。消費者が必ず見るべき情報を後から足す場合は契約major版上げ+購読再交渉が必須。
  「静かな読み飛ばし」と「必須情報」は両立しない——これを契約規則として明文化する。
- **版交渉は購読(target登録)時に確定する**。Edgeは合意したmajor版のレコードのみ配送する
  (「未知majorで停止」の検知点はレコード受信時ではなく登録時の交渉)。minorの未知フィールドは
  optionalフィールドの読み飛ばしのみ許容。
- **初版で確定する2族**:
  - **measurement族**: series識別(D5のseries_key)+event_time(決定3)+値。**一時点=1レコード**。
    values配列は**単一seriesの1観測の値**(値型が `array<scalar,N>` の場合の固定長ベクトル。
    bool/intはf64正規化=D6決定10)であり、**多チャネル束ねでも時間方向ブロックでもない**
    (多軸はチャネル=別series。D5/D6・取り込み契約の忠実な反映)。
  - **annotation族(ストリームannotation)**: データについての構造化通知で、**共有seqを持ち
    全targetに配送される**(購読フィルタ不可)。最低限: D2が配送必須と確定済みの
    **custody_lost欠落annotation**、**検疫遷移annotation**(決定1)、**epoch開始annotation**(決定8)。
    予約(D12波及 2026-07-08): **device_maintenance**(親再起動等の保守イベント)/**counter_discontinuity**
    (カウンタ非連続)——スキーマ詳細はWave 1出口spec。
    購読外シリーズを参照するannotationは情報として無視してよい(消費者に契約上の義務を生まない)。
- `series_definition`同期、legacy metadata snapshot、commissioning smokeは最初の実機縦切りには
  含めない。Siteはraw canonical recordを保存し、Edge Nodeごとのcursor、site-level query、application
  export境界を持つ。series解釈はEdgeのR11または後続のversion付きmetadata契約で追加する。
  Siteのapplication exportは保存済みseriesのrouting・projectionに加え、`production`等の設定可能な
  センサー意味付けを担う。追加時もYokaKit固有のbusiness masterやOEE等の業務ロジックは含めない。
- **配送制御通知(annotationとは別レイヤ)**: gap/cursor_expired(決定6)等、**特定targetの配送状態
  についての通知**はストリームレコードではなく、pushバッチのメタデータ(帯域外)で運ぶ。
  **カーソルを消費しない**。全target共有のストリームにtarget固有の事実を混ぜない。
- **予約family(名前のみ確保、実装は宿題)**:
  - 文字列/離散観測(バーコード等): 取り込み契約が `Vec<f64>`(数値のみ)である現状では出口だけ
    決めても運べない。**D1改訂(文字列観測の取り込み)とセットで後日決定**。YokaKitの完全移行
    (品番切替=バーコード)はこの宿題の消化に従属する。
  - 時系列ブロック/波形(高レート加速度・振動スペクトラム): D6の予約と接続。サンプル間隔・
    ブロック時刻規則を含む別契約として設計する。

## 決定3: 時刻 = 正準event_time+出自併載

- コアが機械的導出規則を一本定義し、導出済み `event_time` を出口レコードに載せる:
  1. デバイス申告時刻(time_source=device_ntp/device_rtc)があればそれ。
  2. なければ `age_ms` 復元時刻(received_at − age_ms、time_source=edge_adjusted。D1)を
     **device_time相当として**採用。
  3. どちらもなければ `received_at`。
- **妥当窓検査は未来方向のみ**: device_time(復元時刻含む)がreceived_atより許容ズレを超えて未来の
  場合は採用せず `event_time = received_at` に**降格**し、降格の事実を `event_time_source` で表示する。
  **過去方向の窓は出口には存在しない**——D1の鮮度ウィンドウ(例24h、設定可能)超の遅着は
  **取り込み時に終端拒否済み**であり(D1不変)、正本に入った行の過去方向はすべて採用できる。
  **【実装状況 2026-07-03、ユーザー裁定】**: このD1鮮度ウィンドウ拒否は**Wave 1実装**(外部送信者=
  HTTP/MQTT取り込みと同時)。Wave 0の実アダプタ(BravePI/rpi-local)は `device_time=None` **かつ
  `age_ms=None`** 決め打ちで、遅着device_timeも `received_at − age_ms` による過去復元(候補2)も
  送らないため、この前提は未到達でも実害なし(両ingest_mapで確認済み。D1同注記と対)。event_time導出(計画4 T2で実装済み)は
  この前提の上に立つが、Wave 0では該当データが到来しない。計画4レビュー(Sonnet/codex)が
  「前提の未実装」を検出し、Wave 1繰り延べで裁定。
  「拒否される遅着(取り込み=D1)」と「降格される遅着(出口=未来方向のみ)」の境界はこの一文で確定。
- 出自フィールド(time_source / time_quality / received_at / device_time)も併載する。
  疑う消費者は出自を見て自分で判断できる。
- **`event_time_source` と `time_source` の関係**: `time_source`(D1)は取り込み時の申告出自
  (device_ntp/device_rtc/edge/edge_adjusted)をそのまま運ぶ。`event_time_source` は
  **出口レコードの新フィールド**で、「event_timeにどの候補を採用したか(device / edge_adjusted /
  received_at)と、未来方向降格が起きたか」を表す導出結果の表示。前者は入力の事実、後者は
  導出の結果——別フィールドとして両方載せる。
- 根拠: 全消費者が同じ時間軸を見ることの保証。受信時刻単独を正とすると電池駆動センサーの
  バックログ再送(柱1中核ケース)で全サンプルが「今」に潰れる。生フィールドのみ(選ばない)だと
  消費者ごとに時間軸解釈が分裂する。導出規則は機械的であり業務ルールではないためコア中立性は
  保たれる。現行YokaKitワイヤにタイムスタンプが一切なく受信時刻を代用している事実(カタログ§2)は
  「時刻を契約で供給すべき」証拠であって「受信時刻で足りる」証拠ではない。
- 未来方向許容ズレの既定値は設計スペック段階で確定(分オーダーを想定)。

## 決定4: 順序とカーソル = 挿入順(epoch, seq)。event_timeは順序ではない

- ストリームの順序とカーソルは**挿入順 `(epoch, seq)`**(D5決定3)。
- **seqの実体は出口publication log**(全family——measurement・annotation——が同一の採番空間を共有する
  単調増加番号)。`readings.seq` は内部の挿入順であって出口seqそのものではない(検疫行は解除まで
  publication logに採番されないため。実装形態——専用outboxテーブル等——はWave 1実装の宿題)。
- **レコード同一性 = `(epoch, seq)`**(単一Edge Node局所)。backfill・再送・復旧のどの経路で届いても
  同一レコードは同一の `(epoch, seq)` を保つ。消費者はレコード単位で冪等upsertする。`publication_id` は
  **バッチ再送の冪等キーのみ**であり、レコード同一性ではない(同一readingが通常再送とbackfillジョブの別バッチで
  二重到達しても、(epoch, seq)で必ず捕まる)。
  **(D8波及 2026-07-07)** 複数Edge Nodeの現場(Site-managed)では、消費者が保持する**global**なレコード同一性・
  cursor・dedup・ack水位・batch冪等キーを `edge_node_id` でスコープする——
  `global_record_identity = (edge_node_id, epoch, seq)`、batch dedup = `(edge_node_id, target_id, publication_id)`。
  `epoch/seq`(や `publication_id`)を単独で消費者DBの主キー・再開位置に使ってはならない。詳細はD8。
- `event_time` は観測時刻であり、**単調ではなく、カーソルでもない**。ackと購読再開はカーソルのみで
  行う。event_timeでack/再開すると遅着バックログを取りこぼす——契約で明示的に禁止する。
- 消費者が表示・集計で時間軸に並べるのはevent_time(それが用途)。配送の完全性はカーソルが担う。
  二つの軸の役割分離が本決定の本体。

## 決定5: 契約はトランスポート非依存。第一波バインディング = MQTT QoS1 (D9改訂 2026-07-13)

- **契約の語彙(record family・(epoch,seq)カーソル・ack・publication_id)はトランスポート非依存に
  定義する。** バインディングは差し替え・追加が可能(D1「バインディング複数」原則の出口版)。
- 第一波(D9改訂): EdgeがMQTT Brokerへ有界batchをQoS 1 publishする。
  broker PUBACKはtransport受領だけを表し、custodyはSiteが耐久commit後に別topicへpublishする
  application-level `accepted-through`で移転する。**at-least-once + 冪等 `publication_id`**とし、
  再送権威はEdge側outboxである。詳細は[D9](D9-exit-mqtt-binding.md)。
  **HTTP push(旧第一波: POST+同期レスポンスack)は追加バインディング候補に降格**——契約語彙は不変のため、
  必要とする消費者が現れれば再設計なしで追加できる。
- 接続の向きが常に外向き(Edge→Broker)なのは不変。MQTT QoSだけではSiteの耐久保存を
  表現しないため、正式purge水位をapplication ackとして分離する。
- 将来のストリーミングバインディング(WebSocket/SSE等)は**追加**であって再設計にならない。
- HTTP圧縮(gzip/zstd)の交渉(Accept-Encoding)はHTTPバインディング固有の規定。MQTT側のペイロード圧縮は
  Wave 1の出口設計specで扱う。
- MQTT再publishは**custody ackの代替にしない**(D8/D9)。レガシーMQTT互換payloadへの変換は
  YokaKit投影アダプタ(ADR 0028)の
  仕事であってコアの仕事ではない(不変)。

## 決定6: target registryとfan-out(ADR 0035統合)

- 第一波はsingle-target。multi-target化の際は配送状態を **`target_id + publication_id` 粒度で分離**。
- **(epoch, seq)カーソルはtarget単位で保持**。あるtargetのackが別targetの未配送状態を消すことは禁止。
- **アーカイブ責任消費者フラグはtarget registryの属性**(D2: 台帳で1消費者を指定)。
  アーカイブ責任に指定できるtargetは、Site applicationが耐久保存後に正式な`accepted-through`を
  返せる構成に限る。broker PUBACKだけを返すtargetはアーカイブ責任に指定できない。
- **cursor_expired規律と復帰状態機械**: 非アーカイブtargetの遅延はパージを阻害しない(D2)。
  pushモデルではEdgeが各targetのカーソルを知っているため、targetのカーソルがパージ済み
  地平より古くなったことは**Edgeが検知**し、次回push時に**gap通知(配送制御通知=決定2、
  「seq S以前は利用不可」)**を添えて**利用可能地平から配送を再開**する。
  - 単純な消費者(ロガー等)はgapを受容してそのまま続行できる(snapshotは**必須ではない**)。
  - 状態志向の消費者はpublication snapshot(決定8)で基準を作り直せる。snapshotには**対応する
    seq水位を刻印**し、消費者はその水位+1から差分を継ぐ。
  - **非アーカイブtargetへの配送保証は「ローカル保持窓の範囲内」**であることを契約に明記。
- **target登録の認証・認可(R19の出口面の骨子)**: target登録・購読フィルタ変更・アーカイブ責任
  フラグ操作は**R14の型付き操作**(権限段階+全操作監査+登録時の疎通スモークテスト必須)。
  target登録・アーカイブフラグ付け替えは正本移転先の変更なので人間の工事操作とする。MVPの接続は
  MQTT over TLS、Edge Nodeごとのstatic credential、topic ACLを使う(D10)。
  Edgeは登録されたtargetへ全測定データをpublishする以上、登録が認可なしなら1回の誤設定が
  全データ流出になる——無人現場で誰も気づかないため、契約定義v1から骨子を持つ(D3読み替え規則)。
- target固有のpayload整形・transport詳細は投影境界の上(コア語彙に入れない)。

## 決定7: 購読フィルタと保管対象ポリシーの分離(データ量への回答)

背景: 「IoTはデータ量が多くなりがち」(ユーザー懸念 2026-07-03)。高レートシリーズを全targetに
全量配送する義務を避けつつ、custodyの意味論を壊さない。

- **購読フィルタ(配送の選択)**: 各targetはシリーズ単位で受け取る対象を宣言できる。
  選択であって解釈ではないためコア中立性(決定1)を壊さない。
  - **照合は実体化済みseries_keyに対して行う**(D6: series_keyは確立後不変)。canonical
    measurement keyでの購読が、過去に別キーで実体化されエイリアス解決されたseriesを自動的に
    含むことは**ない**——alias対応はR11のレジストリメタデータ面(決定9)で読めるので、
    包含したい消費者は自分でフィルタを更新する。
  - **アーカイブ責任targetには購読フィルタを適用しない**(保管対象ポリシーがそのまま配送範囲)。
    フィルタ⊂保管対象の構成を許すと、その差分が「配送されず・ackされず・水位到達で
    custody_lost」という無音のデータ損失製造機になるため、構成として禁止する。
  - **フィルタ変更の遷移**: 変更は変更時点以降のみ有効(拡大しても過去分は黙って届かない)。
    過去分が要る場合はbounded backfill(決定8)で取得する——保管対象ポリシー変更(下記)と対称。
  - annotation族(ストリームannotation)はフィルタ不可(決定2)。
- **保管対象ポリシー(custodyの範囲)**: アーカイブ責任消費者に配送=保管されるシリーズの範囲は
  台帳上の**明示的な宣言**(既定=全量)。**保管対象外と宣言されたシリーズはcustodyの約束自体が
  存在しない**——ローカルretention窓で生き(R11で読める)、窓超過で**custody_lostにならずに**
  期限失効する(宣言時に一度監査。パージごとのannotationは出さない)。
  これにより①除外シリーズが未ackのまま水位まで溜まる矛盾がなく、②custody_lostは
  「保管を約束したのに果たせなかった」例外事象のまま保たれる。
- **R17パージ順序の改訂(D2 §1への波及、本決定で明示改訂)**: 保管対象外シリーズという新しい
  行クラスの導入に伴い、劣化契約のパージ順序を4クラスに更新する:
  **①アーカイブ責任消費者ack済み(最低保持フロア超過分)→ ②保管対象外シリーズ(窓超過分は
  常時削除。圧力時は窓内も監査イベント付きで削除可=劣化)→ ③解決もretireもされない検疫滞留行
  → ④未ackの正本(custody_lost監査+欠落annotation必須)**。
  検疫かつ保管対象外の行は②に帰属する(custody約束がない方が支配する)。
- **ポリシー変更の遷移**: 対象追加=custodyは追加時点から(過去分はbounded backfillの範囲で
  ベストエフォート)。対象除外=**明示的な保管放棄操作**として監査必須(R14の型付き操作)。
- 出口での間引き/ダウンサンプリングは第一波では作らない(アーカイブ忠実性と衝突。間引きは
  消費者側かR11の範囲集計で)。将来必要なら集約ストリームを別familyとして追加できる(決定2の枠組み)。
  ※R9側の**時間集約派生series**(D13決定5予約 2026-07-08)はこの禁止に抵触しない——R9が新しい
  物理seriesを生成し、出口は生成済みseriesを普通に運ぶだけ(標準構成: 生=保管対象外+集約を出口へ)。
- バイナリエンコーディングはJSON開始(オープン契約の検査可能性優先)、実測で問題化したら
  追加バインディング(決定5の枠組み)。

## 決定8: コールドスタート回復(ADR 0032統合)

- 上流(消費者)の再構築時は **publication snapshot**(現在状態スナップショット。**対応するseq水位を
  刻印**)+ optional **bounded backfill** で復旧。有界ジョブとしてモデル化し、無制限履歴エクスポートは
  提供しない(「バッファであって倉庫ではない」D2原則)。
- **backfillはレコードの元の `(epoch, seq)` を保って再配送する**(決定4のレコード同一性)。
  通常ストリーム再送との二重到達は消費者の(epoch, seq)冪等upsertで吸収される。
- **語の分離**: publication snapshotはR22スナップショット(高機密資産・専用チャネル・readings非含有)
  と**別語・別物**。混同禁止(adr-inventory処置案どおり)。
- **検疫遷移はbounded backfillの対象に含める**(検疫解除で有効化された過去データ——決定1の
  新規採番済みレコード——を消費者が取り直せる経路)。
- **エポック2ケースの分離**:
  - (A) 消費者側再構築(Edge epoch不変)= publication snapshot+ローカル履歴からの
    bounded backfill。
  - (B) Edge R22復元(新epoch)= R22スナップショットはreadings本体を含まないため
    **復元前データのbackfillは約束できない**。**新epoch開始annotation(ストリームannotation)には
    旧epoch IDのみを記載**する——新しい箱は旧epochの未配送範囲を知り得ないため、欠落範囲の特定は
    消費者側が自分のカーソルとの突合で行う。消費者はpublication snapshotで新基準に載り替える
    (D5決定3・R22最小契約のエポックフェンスと接続)。

## 決定9: R11読み出し面の範囲

- 基本面(台帳確定済み、D3のWave 0 R11行「範囲クエリ+CSV」を本決定で精緻化): クエリ+範囲集計+
  CSVエクスポート。**時間範囲指定は必須**(無制限全表スキャン禁止。RPiのメモリ現実。YokaKit
  カタログ§4の実運用制約とも一致)。**範囲クエリの時間軸はevent_time基準を正とする**(表示・集計の
  軸=決定4)。received_at基準の照会は出自調査用の副次面として提供してよい。
  event_timeの実体化列+インデックス(readings v3改訂)は実装宿題。
- **レジストリ/台帳メタデータ公開面**: 消費者が値と対象を解釈するための読み出し面。
  - series/measurementメタデータ: 単位・値型・チャネル役割・カタログ/現場レジストリrevision・
    **エイリアス対応表**(D6監査追記がキュー3へ送った宿題の回収。決定7のフィルタ照合の前提)。
  - **subject(デバイス)メタデータ**: user_label・hardware_id・親子関係。消費者がseriesを業務対象に
    対応付けるにはUUIDだけでは足りない(カタログ§6-4の解決キー構造が証拠)。
  - **現行サンプリング間隔**(D12決定4 2026-07-08): 沈黙検知する消費者の参照先。間隔変更が
    「偽の設備停止」として解釈されないための公開メタデータ。
  measurement族レコード自体は値の解釈情報を重複して運ばない。Edgeローカル/対話照会の正本は
  このR11面である。Siteへのversion付きmetadata同期は実機縦切り後の別契約とする。
- R11最小実装は**readings v3+seriesモデル+レジストリ**を読む(旧sensor_readingsベースの
  既存query_readingsは対象外——計画4の旧テーブル削除で消える)。
- Edge自身の健全性(CPU温度/使用率等、旧heartbeatトピック相当)は**R12の面**から取得する
  (R10/R11には入れない)。YokaKit投影アダプタはR12を照会する——出口契約外の依存として明示。
  **R12の照会は読み取り専用の権限段階で足りる**(operatorトークン全権は不要——一消費者に特権を
  与えない)。readings行のrssi/battery_pctはセンサー付帯メタデータであり、Edge健全性とは別物。
- **per-target配送状態(カーソル遅延・再送失敗・exhaustion)はR12の観測面に公開する**——
  フィルタ誤設定・消費者死亡の検出性(柱3)の実体。台帳監査反映「接続状態機械=R10+R12」の出口側。

## 境界の明確化(非目標)

- 派生イベント生成(stale通知・ダウンタイム・アンドン)はコアの仕事ではない(決定1)。
  汎用の派生が本当に必要になったらコアの外(共有ライブラリ/別サービス)で。
- Slack通知等のファンアウトは消費者内部の副作用(カタログ§3補足)。
- デバイスへの制御コマンド送信(南向き)は本契約の範囲外(キュー5)。yokakit-nextは
  MQTTをPublishしない=現消費者に双方向要求は存在しない(カタログ§2)。

## Waveとの対応

- D3のWave 0スコープにR10は含まれない(R11のみ)。計画4(Wave 0最終)に効くのは決定9
  (R11面: クエリ+範囲集計+CSV+レジストリ/台帳メタデータ)。
- R10 push配送・fan-out・publication snapshot・publication logの実装はWave 1以降。
  本決定は契約を先に固定する(実装Waveと契約確定は独立)。

## ADR統合処置(0028/0029/0030(出口側)/0032/0035)

adr-inventory.mdの処置案を本決定で具体化した。各ADR本文への反映(edge語→配置4段語彙置換、
本文書への参照追記)は本決定の確定コミットと同時に行う:

- **0028**: record family「edge-runtime-status」は**コア出口familyとしては存在しない**(Edge
  健全性はR12照会=決定9)。投影層(YokaKit側)が独自に持つ場合の改称は投影アダプタの語彙であり
  コア契約外。検疫遷移annotation配送(決定1・2)・エポック複合カーソル(決定4)・アーカイブ責任
  消費者ack(決定6)・最低保持フロア(D2)の組み込み、複数消費者共有の明記(決定6)。
- **0029**: 「本ADRは出口(R10)でありD1の上流取り込みバインディングとは別方向」の明記、
  アーカイブ責任消費者・R17劣化契約・0035 fan-outとの接続(決定5・6)。
- **0030(出口側)**: 再送単位=有界バッチ(publication_id)、no silent drop=D2 custody_lost規律で
  回収、retry exhaustionのoperator可視化=per-target配送状態のR12公開(決定9)。取り込み側の
  ack語彙(accepted/duplicate/rejected/deferred)はD1で処置済み。
- **0032**: snapshot→publication snapshot改称、seq水位刻印、検疫遷移のbounded backfill対象化、
  エポック2ケース(決定8)。
- **0035**: (epoch, seq)複合カーソルのtarget単位保持、アーカイブ責任消費者フラグの持ち方、
  cursor_expired規律(決定6)。

## 宿題(本決定が送る先)

| 宿題 | 送り先 |
|---|---|
| 文字列/離散観測の取り込み(バーコード等)——YokaKit完全移行の前提 | D1改訂(取り込み契約v2) |
| 時系列ブロック/波形family(サンプル間隔・ブロック時刻規則) | 将来決定(D6予約と合流) |
| publication logの実装形態(outboxテーブル・採番・readings/annotationとの関係) | Wave 1設計スペック |
| publication snapshotの内容定義(series毎最新値か、検疫行の扱い、決定1「派生を作らない」との整理) | Wave 1設計スペック |
| publication_idの採番規則・バッチ組成固定規則 | Wave 1設計スペック |
| event_time実体化列+インデックス(readings v3改訂)——R11範囲クエリ(event_time基準)の前提 | **Wave 0/計画4**の設計スペック |
| 未来方向許容ズレの既定値 | 設計スペック段階 |
| annotation族・配送制御通知・合成テストfamilyの具体スキーマ、契約表現形式(JSONスキーマ置き場) | 設計スペック段階 |
| topic名前空間・ACLの詳細(認証認可の本体はD10で確定 2026-07-08) | Wave 1出口設計スペック |
| 保管対象ポリシー・フィルタ・アーカイブフラグの操作面(宣言・放棄のCLI/UI) | R14操作カタログ(計画4以降) |
| per-target配送状態のR12公開形式 | R12設計スペック(Wave 1) |

## 証拠と経緯

- 消費者ニーズの証拠: [../inputs/2026-07-03-yokakit-consumer-catalog.md](../inputs/2026-07-03-yokakit-consumer-catalog.md)
  (yokakit-next @ 88b8abaf の実コード棚卸し)。
- codex(gpt-5.5/xhigh)中間レビュー2回(2026-07-03): 計11件(高3/中7/低1)全採用。
  主要成果: 保管対象ポリシー分離・順序とカーソルの分離・レジストリメタデータ面・
  一時点=1レコード訂正・cursor_expired・エポック2ケース。
- Fable+codex並行最終レビュー(2026-07-03): 統合21論点(BLOCKER 2/高3含む)全採用。
  主要成果: D1鮮度窓との継ぎ目確定(過去方向降格分岐は死んでいた)・アーカイブtargetの
  フィルタ不適用・publication log採番・レコード同一性=(epoch,seq)・annotation/配送制御通知の
  分離・R19出口面骨子・R17パージ順序4クラス改訂。
