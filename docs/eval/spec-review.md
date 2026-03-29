# iotkit-next Spec Review Guide

spec/design 評価時に Codex プロンプトへ注入する。
Active Watchpoints を先に読み、次に Baseline Checklist を適用する。

## Active Watchpoints

最近のレビューで観測されたプロジェクト固有の盲点。
max 10 items、デフォルト TTL 3ヶ月。繰り返し出現する項目は Baseline に昇格する。

- Added: 2026-03-27
  Revalidate by: 2026-06-27
  Watchpoint: `biased` in `tokio::select!` creates starvation risk when one channel has sustained traffic — applies to any fan-in or adapter loop.
  Observed in: rpi-local-adapter impl review.

- Added: 2026-03-27
  Revalidate by: 2026-06-27
  Watchpoint: Partial config surfaces (some env vars, some hardcoded) create an awkward middle ground that invites silent misconfiguration. Either fully hardcode or fully expose.
  Observed in: rpi-local-adapter gateway integration.

- Added: 2026-03-28
  Revalidate by: 2026-06-28
  Watchpoint: When a runtime claims "sensor-specific logic only / zero boilerplate," verify that transport-level metadata (ConnectionInfo, bus/address parameters) is constructed by the runtime, not repeated in each driver.
  Observed in: polling-adapter-runtime config rename review.

- Added: 2026-03-29
  Revalidate by: 2026-06-29
  Watchpoint: Stateful components (MQTT connection, command orchestrator, device lifecycle) MUST include a Graphviz state transition diagram in the spec. Each state must list: (1) valid transitions, (2) failure modes, (3) what happens to in-flight data. Without this, implementation produces happy-path-only code that takes 4+ Codex review rounds to fix edge cases.
  Observed in: Phase 1A adapter-runner — 5 rounds of Codex review for MQTT reconnect/disconnect edge cases.

- Added: 2026-03-29
  Revalidate by: 2026-06-29
  Watchpoint: Partial or ambiguous configuration must fail fast, never silently fall back. Applies to: TLS half-config (cert without key), config file path fallback, unknown enum values mapping to defaults. Every config field must be either required or explicitly optional with documented default.
  Observed in: Phase 1A — silent mTLS fallback, config path fallback, thermocouple type silent K default.

## Baseline Checklist

安定的なアーキテクチャレビュー基準。ポリシー変更時のみ更新する。

### 依存と境界

- [ ] `core/types` に adapter、transport、protocol の詳細が逆流していないか。
- [ ] crate 依存が一方向になっているか。
- [ ] `AdapterEvent`、`AdapterCommand`、`SensorIdentity`、`DeviceKey` が汎用契約として保たれているか。
- [ ] adapter 固有の値や enum が core の共有型に漏れていないか。
- [ ] transport は open、read、write などの I/O 責務に留まっているか。
- [ ] codec はフレーム分解の責務に留まり、意味解釈を抱え込みすぎていないか。
- [ ] adapter と runtime composition root が分離されているか。

### 状態遷移と failure mode

- [ ] 状態を持つコンポーネントに Graphviz 状態遷移図があるか。
- [ ] 各状態で「起きうる failure」と「その時のデータの扱い」が定義されているか。
- [ ] 状態遷移の全ペア（正常 + 異常）が網羅されているか。
- [ ] 切断/再接続時にバッファ、retained message、in-flight data がどうなるか明記されているか。
- [ ] graceful shutdown 時に全ての enqueue 済みデータが flush されるか、明記されているか。

各状態遷移に対して以下を強制回答させること:

| Question | 回答が必要 |
|----------|-----------|
| In-flight data はどうなるか？ | buffer / drop / retry のいずれか明記 |
| バッファの上限と溢れ時の挙動は？ | 上限数 + oldest drop / backpressure 明記 |
| retained message は broker 上でどうなるか？ | 残る / 上書き / empty publish で削除 |
| プロセスが crash したら？ | LWT / orphan data / recovery 手段を明記 |
| side-effect (publish/write) が途中で失敗したら？ | retry / tombstone / データ状態を明記 |

### ライフサイクル

- [ ] `discover`、`update`、`lost`、`error` の発火条件が揃っているか。
- [ ] 型だけ存在して実装経路のない概念が残っていないか。
- [ ] デバイスが一度も `DeviceDiscovered` されない経路がないか。
- [ ] 再接続時に state が二重化しないか。
- [ ] shutdown と reader 再試行が競合しても破綻しないか。
- [ ] partial frame、duplicate frame、unknown device で状態管理が崩れないか。

### センサー抽象

- [ ] sensor driver が入力ソース差を吸収できているか。
- [ ] I2C、UART、GPIO の差分が adapter 側の巨大 `match` に漏れていないか。
- [ ] 同じ `sensor_type` を別 adapter から使うとき、sensor module を再利用できるか。
- [ ] sensor 追加時の変更箇所が増えすぎていないか。
- [ ] sensor identity の生成責務が一箇所に寄っているか。

### 拡張性

- [ ] 新しい sensor type を足すとき、変更箇所を明確に列挙できるか。
- [ ] 新しい adapter を足すとき、共通基盤を再利用できるか。
- [ ] DB、設定、API、監視、起動管理を毎回手書きしなくて済むか。
- [ ] adapter 固有永続化と core 永続化の境界が分離されているか。
- [ ] 将来の pair、scan、DFU、delete-sync をどこに置くか方針があるか。
- [ ] `AdapterCommand` が adapter 固有コマンドの寄せ集めになる兆候がないか。

### 障害と運用

- [ ] 全体障害と個別デバイス障害が区別できるか。
- [ ] `device_key: None` にすべきケースと、特定デバイスに紐づくケースが混ざっていないか。
- [ ] retry 回数、port、device、sensor_type、切断理由がログで追えるか。
- [ ] reader thread 異常終了時の扱いが定義されているか。
- [ ] oversized frame や malformed payload が adapter 全体を汚染しないか。

### UI と API

- [ ] core が adapter 固有 UI を直接知る設計になっていないか。
- [ ] adapter 管理 API の置き場が先に崩れていないか。
- [ ] core の `/adapters` と adapter 固有画面の責務分離が説明できるか。

### 定番質問

- この変更で新しい sensor type を足すと、どのファイルを何箇所触るか。
- この変更で新しい adapter を足すと、どこまで共通化が効くか。
- 1台のデバイスが無言で消えたとき、core はいつどう知るか。
- reader が落ちて復帰したとき、discover や state は二重化しないか。
- protocol 詳細を消したあとでも、core の型は意味を保てるか。

### 危ない実装パターン (Anti-Patterns)

以下のパターンが spec/plan に潜んでいたら指摘すること。

**AP-1: Remove before confirm**
collection.remove() してから side-effect (publish/write/delete) する設計。
side-effect が失敗するとデータが永久ロスト。正しくは peek → side-effect 成功 → remove。
例: inventory tombstone を pop_front してから publish → publish 失敗でデータ消失。

**AP-2: Async enqueue ≠ delivery**
async client.publish().await の Ok を「送信完了」と扱う設計。
rumqttc 等の async MQTT client は内部キューに入れるだけで、EventLoop が poll するまでブローカーに届かない。abort/disconnect が先に来ると送信されない。
正しくは flush 用 grace period、または delivery confirmation (PUBACK) を待つ。
例: offline status publish → 即 eventloop abort → ブローカーに届かない。

**AP-3: Silent config fallback**
設定値が無効/欠落時にデフォルトにサイレント fallback する設計。
設定ミスが検出されず、間違った環境で動作する。
正しくは不正な設定は即 error exit。fallback は明示的 default_value のみ許可。
例: unknown thermocouple_type → K、cert だけ設定して key なし → 認証なし。

**AP-4: Lossy encoding in identifiers**
topic/path に使う identifier を非可逆変換 (`:` → `-` 等) でエスケープする設計。
元の値を復元できず、異なる入力が同じ出力に衝突する。
正しくは percent-encoding 等の可逆変換。
例: adapter_id の `:` と `/` を両方 `-` に変換 → 衝突。

### 危ないサイン（構造）

- 設計メモでは抽象化されているのに、実装の本当の拡張点が別の場所にある。
- adapter が protocol 解釈だけでなく、起動、監視、thread 管理、永続化まで抱え込んでいる。
- event contract にある概念が、正常系の一部でしか流れていない。
- 追加変更のたびに `match` が増え続ける。

### Library Pitfalls

プロジェクトで使用しているライブラリの既知の罠。

**rumqttc:**
- `AsyncClient::publish()` は内部キューに入れるだけ。`EventLoop::poll()` しないと送信されない。
- `EventLoop` を poll しないと keepalive も送信されず、broker が切断する。
- reconnect は `EventLoop` 内部で自動処理。`ConnAck` の検知は caller 責務。

**tokio:**
- `select!` はデフォルトでランダム選択。`biased` だと starvation リスク。どちらも問題を起こしうる。独立 task 分離が最も安全。
- `signal::ctrl_c()` は SIGINT のみ。SIGTERM は `unix::signal(SignalKind::terminate())` が必要。
- `abort()` は即座に task を停止。cleanup コードは実行されない。grace period が必要。

**rusqlite:**
- `INSERT OR IGNORE` は PK 衝突以外の constraint 違反も無視する。`ON CONFLICT(...) DO NOTHING` で限定すること。

**serde:**
- `#[serde(default)]` は missing field を受け入れるが、invalid value (型不一致) は reject。混同しやすい。
- `i64` → `u64` キャストは負値で overflow/panic。外部 JSON の decode では必ず range check。

## Maintenance

- 期限切れの watchpoint は明示的に更新されない限り削除する。
- 繰り返し出現する watchpoint は Baseline Checklist に昇格する。
- Active Watchpoints が空の場合は `(none currently)` と記載する。
