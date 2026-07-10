# AGENTS.md

> **メイン駆動を引き継ぐ場合(2026-07-11〜)**: このリポジトリはメイン駆動を Claude Code から
> codex CLI へ移行中。移行時点の生の作業状態(計画6 の設計ドラフト・配布既定メニューのユーザー裁定
> 待ち・役割逆転の注意)は [docs/superpowers/HANDOFF-2026-07-11-to-codex-driver.md](docs/superpowers/HANDOFF-2026-07-11-to-codex-driver.md)。
> 本 AGENTS.md は「呼ばれる側の per-task ワーカー」向けに書かれている——メイン駆動になると
> コミット・spec/plan・ユーザー対話も自分の責務になる(引き継ぎ §0)。

## Project Context

`iotkit-next` は旧 `iotkit` をゼロから作り直す Rust + tokio の IoT ゲートウェイ(Raspberry Pi 向け)。
レイヤ: {`core/types`, `core/supervision`} <- {`core/engine`, adapters} <- `iotkit-gateway`。
取り込み経路はアダプタ内クライアント(`iotkit-ingest-client`)が正(D4)。adapters は `core/engine` に
依存しない — `AdapterEvent` は engine/監督専用の frozen vocabulary で、新規コードは依存を増やさない。

**コードの置き場・crate 地図・層規則の正本は `docs/architecture.md`**(依存方向は `scripts/check-layers`
が機械検査。verify.sh に含まれる)。新しいコードの配置は置き場決定表に従い、新 crate を作るときは
check-layers の分類と architecture.md の地図を同時に更新する。

**正しさの基準は旧実装ではなく設計正本** — `../docs/redesign/`(用語集・責務台帳 R1〜R23・決定文書 D1〜D13)。
旧実装との互換はゴールではない。タスク指示が設計正本と矛盾して見えるときは、勝手に解釈せず作業を止めて報告する。

## Invariants(絶対に破らない)

- 秘密情報(トークン・credential・鍵)を Debug 出力・ログ・エラー・監査記録に載せない。
- データを黙って失わない。ack の意味は D1 に従う — rejected は決定的違反専用で、ストレージ失敗に rejected を返さない(ack なし)。
- 変更系操作は R14 dispatch 経由。SQL 直書きの変更経路を新設しない。

## Working Rules(codex タスク)

- プロンプトで指定されたタスクだけを実装する。スコープ外の「改善」を混ぜない。
- `git commit` しない(コミットは呼び出し側が行う)。
- 完了報告の前に `scripts/verify.sh`(fmt + 層規則 check-layers + `cargo test --workspace` + clippy `-D warnings`)を通す。
- テスト緑は必要条件であって十分条件ではない — データ損失・並行退行・仕様逸脱はテストを素通りしうる。設計正本の不変条件を自分で照合する。
