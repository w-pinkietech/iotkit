# Implementation Rules

Dev agent が実装前に読むこと。eval guide (spec-review.md, plan-review.md) の詳細版ではなく、根本原則。

## 思考原則

### 1. 変換は可逆でなければならない

identifier、key、topic segment などを変換する場合、元の値を復元できること。
非可逆変換（`:` → `-`）は異なる入力を衝突させる。percent-encoding が標準的な解法。

### 2. Side-effect の前に状態を変えない

collection.remove() → publish のように、side-effect の前に内部状態を変更すると、side-effect の失敗時にデータが消える。
正しくは: peek → side-effect 成功確認 → 状態変更。

### 3. Async enqueue ≠ delivery

rumqttc 等の async MQTT client で `publish().await` が Ok を返しても、ブローカーに届いたとは限らない。
内部キューに入っただけ。eventloop の poll が実行されるまで送信されない。
abort() する前に grace period が必要。

### 4. 設定ミスは即座に死ぬ

不正な設定値をデフォルトにフォールバックしてはならない。
「設定されていない」（Optional で None）と「設定が不正」（値が無効）は別。
前者はデフォルト適用可。後者は error exit。

### 5. 並行コンポーネントのライフサイクルを設計する

複数の tokio::spawn された task がある場合、以下を spec/実装の前に決めること:
- **起動順序:** どの task が先に ready になる必要があるか
- **片方の crash:** 他の task はどう反応するか（検出方法、shutdown 手順）
- **shutdown 順序:** 誰が先に止まるか。データ生産者 → 消費者の順が原則
- **startup failure:** 背景 task の即座の失敗を main が検出できるか

### 6. 一意性の保証は設計時に決める

session ID、client ID、device key 等の一意性は、生成ロジックではなく**入力の性質**から導出する。
ランダムは一意だが再起動で変わる。deterministic だが非可逆な変換は衝突する。
正しくは: 入力を可逆エンコードして一意性を継承する。

### 7. バッファリングは live processing を阻害してはならない

再接続後のバッファ flush を unbounded loop で行うと、live event の処理が止まる。
batched flush（N件ごとに yield）か、live event と interleave すること。

## Library Pitfalls

### rumqttc
- `AsyncClient::publish()` は内部キューに入れるだけ。`EventLoop::poll()` しないと送信されない。
- `EventLoop` を poll しないと keepalive も送信されず、broker が切断する。
- reconnect は `EventLoop` 内部で自動。`ConnAck` の検知は caller 責務。

### tokio
- `select!` はデフォルトでランダム選択。`biased` だと starvation。独立 task 分離が最も安全。
- `signal::ctrl_c()` は SIGINT のみ。SIGTERM は `unix::signal(SignalKind::terminate())`。
- `abort()` は cleanup なしに即停止。grace period → abort の順で使う。
- `spawn` した task の早期失敗を main が検出するには、JoinHandle を select! に含める。

### rusqlite
- `INSERT OR IGNORE` は PK 以外の constraint 違反も無視する。`ON CONFLICT(...) DO NOTHING` で限定。

### serde / JSON
- 外部 JSON の `i64` を `u64` にキャストすると負値で overflow/panic。range check 必須。
- `#[serde(default)]` は missing field 用。invalid value は reject される。混同注意。
