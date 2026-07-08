# AGENTS.md

## Project Context

`iotkit-next` は旧 `iotkit` をゼロから作り直す Rust + tokio の IoT ゲートウェイ(Raspberry Pi 向け)。
レイヤ: `core/types` <- `core/engine` <- adapters <- `iotkit-gateway`。

**正しさの基準は旧実装ではなく設計正本** — `../docs/redesign/`(用語集・責務台帳 R1〜R23・決定文書 D1〜D13)。
旧実装との互換はゴールではない。タスク指示が設計正本と矛盾して見えるときは、勝手に解釈せず作業を止めて報告する。

## Invariants(絶対に破らない)

- 秘密情報(トークン・credential・鍵)を Debug 出力・ログ・エラー・監査記録に載せない。
- データを黙って失わない。ack の意味は D1 に従う — rejected は決定的違反専用で、ストレージ失敗に rejected を返さない(ack なし)。
- 変更系操作は R14 dispatch 経由。SQL 直書きの変更経路を新設しない。

## Working Rules(codex タスク)

- プロンプトで指定されたタスクだけを実装する。スコープ外の「改善」を混ぜない。
- `git commit` しない(コミットは呼び出し側が行う)。
- 完了報告の前に `scripts/verify.sh`(fmt + `cargo test --workspace` + clippy `-D warnings`)を通す。
- テスト緑は必要条件であって十分条件ではない — データ損失・並行退行・仕様逸脱はテストを素通りしうる。設計正本の不変条件を自分で照合する。
