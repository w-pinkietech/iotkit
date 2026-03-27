# iotkit-next Coding Review Checklist

`iotkit-next` の実装レベル、コードレベルのレビュー観点メモ。
Rust 実装、codec、event loop、driver/adapter 周辺を主対象にする。

関連:
- [architecture-review-checklist.md](./architecture-review-checklist.md)

## 使い方

- まず public API と hot path を見る。
- 次に error path、境界条件、並行処理を見る。
- 最後にテストとログで「壊れたときに追えるか」を確認する。
- 指摘は「バグ」「将来壊れやすいコード」「既存方針からの逸脱」の順で出す。

## API と責務

- [ ] public 関数や public 型は、その crate の責務に見合う最小範囲か。
- [ ] private にできる実装詳細を `pub` にしていないか。
- [ ] protocol 固有の数値やバイト列解釈が core 側に漏れていないか。
- [ ] domain 変換が codec や transport に紛れ込んでいないか。
- [ ] 追加変更で `match` や `if` の集中点が肥大化していないか。

## データ検証

- [ ] 長さチェック前に payload を読んでいないか。
- [ ] `data_count`、payload 長、実際の decode 結果の整合が取れているか。
- [ ] continuation や partial frame の状態が異常入力で壊れないか。
- [ ] unknown 値を握りつぶして誤った既知データにしていないか。
- [ ] placeholder 値を本物の `DeviceKey` や identity に変換していないか。

## エラーハンドリング

- [ ] runtime path に `unwrap`、`expect`、panic 前提コードがないか。
- [ ] 返す error が port、device、sensor_type など追跡に必要な文脈を持っているか。
- [ ] 全体障害と個別デバイス障害を同じ error で潰していないか。
- [ ] recoverable error と fatal error が区別されているか。
- [ ] `std::io::ErrorKind::Other` の手組みより `std::io::Error::other` が使える場所で使っているか。

## async / thread / channel

- [ ] bounded channel のサイズに意図があるか。
- [ ] `send().await` の失敗時に task をどう終わらせるか明確か。
- [ ] shutdown 経路で thread、task、channel の終了順が破綻しないか。
- [ ] retry/backoff 中でも shutdown への応答性が確保されているか。
- [ ] event の送信順序に意味がある場合、その順序がコード上で保証されているか。
- [ ] 再接続時に state を引き継ぐべきものと捨てるべきものが整理されているか。

## メモリと割り当て

- [ ] hot path で不要な `clone()`、`to_vec()`、`String` 生成を繰り返していないか。
- [ ] 上限のない buffer growth がないか。
- [ ] 攻撃的または壊れた入力でメモリが増え続けないか。
- [ ] 長寿命 state がデバイス数や入力量に応じて無制限に伸びないか。

## Rust らしさ

- [ ] `Result`、`Option`、enum を使うべきところで文字列フラグや sentinel 値に逃げていないか。
- [ ] 所有権の都合だけで不自然な clone を増やしていないか。
- [ ] 型で表現できる制約をコメントだけに押し込めていないか。
- [ ] `Default`、`From`、`TryFrom`、newtype などを使うと読みやすくなる場面を逃していないか。
- [ ] 命名が役割を正しく表しているか。`reader`、`convert`、`handle` が本当にその責務だけか。

## ログと観測可能性

- [ ] `tracing` の field が文字列連結ではなく構造化されているか。
- [ ] 異常系で必要な field が抜けていないか。
- [ ] 同じ障害が繰り返されたとき、ログだけで時系列を追えるか。
- [ ] warning と error の使い分けに一貫性があるか。

## テスト

- [ ] 変更した分岐に対応するテストが追加されているか。
- [ ] バグ修正なら再発防止テストがあるか。
- [ ] unit test と integration test の置き場が妥当か。
- [ ] 正常系だけでなく malformed input、oversized input、shutdown 競合を見ているか。
- [ ] テストが実装詳細ではなく契約を見ているか。
- [ ] 新しいテスト自体が lint failure を増やしていないか。

## ドキュメントと保守性

- [ ] コメントは必要な理由や制約を書いていて、コードの逐語説明になっていないか。
- [ ] README や設計メモと実装の前提がズレていないか。
- [ ] magic number はプロトコル仕様か、意味のある定数名に切り出せるか。
- [ ] 将来の reviewer が「なぜこうしたか」を追える形になっているか。

## レビュー時の定番質問

- この関数はどこまでが責務で、どこから先は別層の責務か。
- 壊れた入力を 1 発入れたとき、state はきれいに戻るか。
- 新しい sensor type を足したとき、この変更の書き方で重複が増えないか。
- この error は運用中に見たとき、次に何を確認すればよいか。
- clone や allocation は必要最小限か。
- テストは今回の不具合経路を本当に踏んでいるか。

## レビューで特に危ないサイン

- エラー時だけ sentinel 文字列を入れて後段で特別扱いしている。
- decode、convert、state 管理、event 発火が1つの関数に固まっている。
- 境界条件テストがなく、正常系のサンプルだけで安心している。
- ログ文はあるが field が無く、後から機械的に追えない。
- `clone` を増やしてその場をしのいでいる。
