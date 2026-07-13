# AGENTS.md

## Project Context

`iotkit-next` は旧 `iotkit` をゼロから作り直す Rust + tokio の IoT ゲートウェイ
(Raspberry Pi 向け)。

レイヤ:

```text
{core/types, core/supervision} <- {core/engine, adapters} <- iotkit-gateway
```

取り込み経路はアダプタ内クライアント (`iotkit-ingest-client`) が正 (D4)。
adapters は `core/engine` に依存しない。`AdapterEvent` は engine/監督専用の frozen
vocabulary であり、新規コードは依存を増やさない。

コードの置き場、crate 地図、層規則の正本は `docs/architecture.md`。依存方向は
`scripts/check-layers` が検査する。新しい crate を作る場合は、同スクリプトの分類と
`docs/architecture.md` を同時に更新する。

正しさの基準は旧実装ではなく `docs/redesign/` の設計正本
(用語集、責務台帳 R1〜R23、決定文書 D1〜D13)。タスク指示と設計正本が矛盾して
見える場合は、勝手に解釈せず作業を止めて報告する。

## Invariants（絶対に破らない）

- 秘密情報（トークン、credential、鍵）を Debug 出力、ログ、エラー、監査記録に載せない。
- データを黙って失わない。ack の意味は D1 に従う。`rejected` は決定的違反専用で、
  ストレージ失敗には `rejected` を返さない（ack なし）。
- 変更系操作は R14 dispatch 経由。SQL 直書きの変更経路を新設しない。

## Development Workflow

開発プロセスは、その作業に該当する標準 Superpowers スキルに従う。基本の流れは次のとおり。

```text
brainstorming
  -> written design + user review
  -> writing-plans
  -> test-driven-development
  -> requesting-code-review / receiving-code-review
  -> verification-before-completion
  -> finishing-a-development-branch
```

設計や計画が不要な単純作業まで形式的に膨らませない。適用条件は各スキルの記述に従う。
`docs/superpowers/specs/` と `docs/superpowers/plans/` は、現在の作業に明示的に選ばれた
文書を除き、設計判断と実行履歴である。履歴中の古い運用指示は現行ルールではない。

## Roles and Authority

- ネイティブな役割 dispatch が利用できる場合、Main と reviewer は Sol/high、
  implementer と executor は Luna/max を意図する。役割選択は実行支援であり、追加の
  台帳や証明状態を作らない。
- worker は指定されたタスクだけを実装し、スコープ外の改善を混ぜず、commit しない。
- Main は承認済み作業の範囲で設計、実装、検証、レビュー、意図的な commit を行える。
- push、PR、merge、release、課金を伴う実行、その他の外部作用は別のユーザー承認を要する。
- 破壊的操作や認証情報の公開は、通常の Codex 権限境界に従う。

## Verification Economy（時間は有限）

- 検証は変更範囲、リスク、現実的な失敗経路に比例させる。検査数を増やすこと自体を
  目的にしない。
- 結果が変更の信頼性を実質的に高めないと明らかに判断できる検査は省略する。
- 通常なら実行する検査を省略した場合、完了報告に省略した検査と、変更へ無関係と
  判断した具体的理由を書く。
- Rust 製品動作、層境界、認証、秘密情報、data loss/custody、並行処理、外部作用に
  関係する検査は、その失敗可能性を除外できない限り省略しない。
- Rust 製品動作へ影響する、または影響を除外できない変更は `scripts/verify.sh`
  （fmt、層規則、workspace tests、Clippy `-D warnings`）を通す。
- 文書のみ、または製品動作に影響しない限定的な設定変更は focused checks に絞れる。
- テスト緑は必要条件であって十分条件ではない。設計正本と不変条件も照合する。
- 影響範囲が不明な場合は検証を広げる。「時間は有限」は未解決の重大リスクを
  受け入れる理由にしない。
