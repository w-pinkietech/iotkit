# CLAUDE.md

オンプレミス優先のIoTデータ収集基盤。収集側のRust + tokio製`IoTKit Edge Node`
（Raspberry Pi向け）と、集約側のGo製`IoTKit Edge`からなる。

```text
{core/types, core/supervision} <- {core/engine, adapters} <- iotkit-edge-node
```

adapters は `core/engine` に依存せず、取り込みは `iotkit-ingest-client` 経由 (D4)。

## Authorities

- 文書の入口と正本順序: `docs/README.md`
- versioned contract: `docs/okf/{en,ja}/contracts/`の対訳文書、machine-readable schemaまたは
  exported wire types、共有fixture、conformance testを一組として扱う
- コード配置、crate 地図、層規則: `docs/okf/ja/architecture/system-overview.md`
- エージェントのプロジェクト規則と不変条件: `AGENTS.md`

`docs/redesign/`の用語集・責務台帳・決定文書は理由と不変条件、inputs/reviews/移行記録と
`docs/superpowers/`は履歴資料であり、現行実装状態や作業指示を上書きしない。
旧実装も正しさの基準ではない。現行正本と依頼が矛盾して見える場合は作業を止めて確認する。

## Product Invariants

- 秘密情報を Debug 出力、ログ、エラー、監査記録へ載せない。
- データを黙って失わない。ストレージ失敗は custody ack を生まない。
- 変更系操作は所有componentの R14 typed dispatch 経由。Edge Nodeは`core/ops`、IoTKit EdgeはGo
  application service内のtyped operation dispatcherを使い、新しいdirect-SQL mutation pathを作らない。

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
