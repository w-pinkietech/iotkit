# CLAUDE.md

Rust + tokio の Raspberry Pi 向け IoT ゲートウェイ。

```text
{core/types, core/supervision} <- {core/engine, adapters} <- iotkit-gateway
```

adapters は `core/engine` に依存せず、取り込みは `iotkit-ingest-client` 経由 (D4)。

## Authorities

- 製品の設計正本: `docs/redesign/`（用語集、責務台帳 R1〜R23、決定文書 D1〜D13）
- コード配置、crate 地図、層規則: `docs/architecture.md`
- エージェントのプロジェクト規則と不変条件: `AGENTS.md`
- 開発プロセス: 作業に該当する標準 Superpowers スキル

旧実装は正しさの基準ではない。設計正本と依頼が矛盾して見える場合は作業を止めて確認する。

## Product Invariants

- 秘密情報を Debug 出力、ログ、エラー、監査記録へ載せない。
- データを黙って失わない。ストレージ失敗は custody ack を生まない。
- 変更系操作は R14 dispatch 経由。新しい direct-SQL mutation path を作らない。

## Workflow

開発laneの選択、skill利用、レビュー、成果物の規則は `AGENTS.md` だけを正本とし、
ここへ重複記載しない。Main は承認済み作業を実装・commitできる。worker は指定スコープだけを
扱い、commitしない。push、PR、merge、releaseなどの外部作用は別承認。

## Build and Verification

```bash
cargo test --workspace
cargo test -p <crate-name>
scripts/verify.sh
scripts/check-layers
```

`scripts/verify.sh` は fmt、層規則、workspace tests、Clippy `-D warnings` の製品ゲート。
検証は変更範囲と現実的な失敗経路に比例させ、明らかに無関係な検査は省略できる。
省略した通常検査と具体的理由は完了報告へ書く。Rust 製品動作、認証、秘密情報、
data loss/custody、並行処理、外部作用への影響を除外できない場合は検証を広げる。

## Commit Style

`feat(crate):` / `fix(crate):` / `refactor(crate):` / `docs:` / `chore:`。
Co-Authored-By は assistant identity が確認できる場合だけ付ける。
