# AGENTS.md

> **メイン駆動を引き継ぐ場合(2026-07-11〜)**: このリポジトリはメイン駆動を Claude Code から
> codex CLI へ移行中。移行時点の生の作業状態(計画6 の設計ドラフト・配布既定メニューのユーザー裁定
> 待ち・役割逆転の注意)は [docs/superpowers/HANDOFF-2026-07-11-to-codex-driver.md](docs/superpowers/HANDOFF-2026-07-11-to-codex-driver.md)。
> 本 AGENTS.md は「呼ばれる側の per-task ワーカー」向けに書かれている——メイン駆動になると
> コミット・spec/plan・ユーザー対話も自分の責務になる(引き継ぎ §0)。
>
> Codex Cloudを含む新しいcloneからの再開は
> [docs/cloud-development.md](docs/cloud-development.md) を読み、次に永続台帳を実物と照合する。
> Cloud taskは候補ブランチ専用で、task URL/status/diffだけを独立レビューやSETTLEDの証拠にしない。
> 外出中のCloud Mainは未決レビュー債務を台帳へ残し、`master`統合をローカル復帰後まで保留する。

## Project Context

`iotkit-next` は旧 `iotkit` をゼロから作り直す Rust + tokio の IoT ゲートウェイ(Raspberry Pi 向け)。
レイヤ: {`core/types`, `core/supervision`} <- {`core/engine`, adapters} <- `iotkit-gateway`。
取り込み経路はアダプタ内クライアント(`iotkit-ingest-client`)が正(D4)。adapters は `core/engine` に
依存しない — `AdapterEvent` は engine/監督専用の frozen vocabulary で、新規コードは依存を増やさない。

**コードの置き場・crate 地図・層規則の正本は `docs/architecture.md`**(依存方向は `scripts/check-layers`
が機械検査。verify.sh に含まれる)。新しいコードの配置は置き場決定表に従い、新 crate を作るときは
check-layers の分類と architecture.md の地図を同時に更新する。

**正しさの基準は旧実装ではなく、このリポジトリ内の設計正本** — `docs/redesign/`
(用語集・責務台帳 R1〜R23・決定文書 D1〜D13)。
旧実装との互換はゴールではない。タスク指示が設計正本と矛盾して見えるときは、勝手に解釈せず作業を止めて報告する。

## Invariants(絶対に破らない)

- 秘密情報(トークン・credential・鍵)を Debug 出力・ログ・エラー・監査記録に載せない。
- データを黙って失わない。ack の意味は D1 に従う — rejected は決定的違反専用で、ストレージ失敗に rejected を返さない(ack なし)。
- 変更系操作は R14 dispatch 経由。SQL 直書きの変更経路を新設しない。

## Worker mode rules(codex タスク)

- プロンプトで指定されたタスクだけを実装する。スコープ外の「改善」を混ぜない。
- `git commit` しない(コミットは呼び出し側が行う)。
- 完了報告の前に `scripts/verify.sh`(fmt + 層規則 check-layers + `cargo test --workspace` + clippy `-D warnings`)を通す。
- テスト緑は必要条件であって十分条件ではない — データ損失・並行退行・仕様逸脱はテストを素通りしうる。設計正本の不変条件を自分で照合する。

## メイン駆動時の運用(2026-07-11 ユーザー決定)

この節はMain mode専用。上の「commitしない」はworkerにだけ適用し、Mainは承認済みmission内で
意図的にcommitする(push/PRは別承認)。該当フェーズでは `.claude/skills/*/SKILL.md` も読み、
運用正本に反する旧記述は運用正本を優先する。

運用正本は `docs/development-workflow.md`。メイン駆動はDesign Ready、Green/Yellow/Red、
リスク適応パイプライン、永続台帳、停止条件に従う。Plan 6はYellow autonomy試行であり、
Green/Yellowは自律、Redのみ最大3件の判断パケットとしてユーザーへ上げる。

cross-vendor reviewは一時停止中。必須レビューは同一成果物ハッシュをfreshなCodex
read-only sessionへdispatchする:

- codex 側(read-only sandbox、コマンド実行可=`cargo test`・境界プローブができる):
  `REVIEW_MANIFEST=<manifest> scripts/codex.sh review <prompt-file> <label>`
- Claude 側(任意。subscription access復旧時のみ):
  `REVIEW_MANIFEST=<manifest> scripts/claude-review.sh <prompt-file> <label>`
  (復旧後に任意追加するときのモデル/effortは運用正本とその時点の明示判断に従う。
  出力は codex と同じ `/tmp/codex-runs/`)
- Grok 側(任意。quota復旧時のみ):
  `REVIEW_MANIFEST=<manifest> scripts/grok-review.sh <prompt-file> <label>`

degraded modeではCodexが実行/原子性/データ損失/攻撃実行面に加え、正本/意味整合/UX/配布運用も担う。
全員がRed分類、auth/secrets、data loss/custody、外部作用、hash provenance、settlementも確認する。
他観点の指摘は禁止しない。Codexのみ実行可で、Claude/Grokのクリーン通過を実行依存保証に使わない。

完了条件・確認ラウンド・消費ゲート、通常/high-riskモデル行列は運用正本に従う。
