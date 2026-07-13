# Superpowers design and execution records

開発プロセスの正本は、現在の実行環境にインストールされた標準 Superpowers スキルである。
作業内容に応じて `brainstorming`、`writing-plans`、`test-driven-development`、
レビュー、検証、ブランチ完了の各スキルを適用する。

このディレクトリの `specs/`、`plans/`、`migrations/` は、設計判断と実行計画をリポジトリに残す場所。
現在の作業で明示的に選ばれた文書以外は履歴資料であり、書かれた当時の運用手順を
現行指示として扱わない。履歴資料は、現在の設計正本と矛盾しない範囲で判断理由や
実装経緯を調べるために使う。

現在の正本:

- 製品の what/why: [`../redesign/`](../redesign/)
- コード配置、crate 地図、層規則: [`../architecture.md`](../architecture.md)
- エージェント規則、安全不変条件、検証方針: [`../../AGENTS.md`](../../AGENTS.md)

過去の計画に残る廃止済み vocabulary、設定値、レビュー手続きより、上記の正本と
現在選択中の Superpowers スキルを優先する。
