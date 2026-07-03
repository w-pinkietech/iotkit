# CLAUDE.md

Rust + tokio IoT gateway for Raspberry Pi. `core/types` <- `core/engine` <- adapters <- `iotkit-gateway`.

## Design Authority

- **設計正本は [w-pinkietech/iotkit-redesign](https://github.com/w-pinkietech/iotkit-redesign) の `docs/redesign/`**
  (用語集・責務台帳R1-R23・決定文書D1〜D6)。
- ローカル開発ではこのリポジトリの親ディレクトリに設計リポジトリをチェックアウトする配置を推奨
  (例: `~/dev/iot/docs/redesign/` と `~/dev/iot/iotkit-next/`)。本文書内の設計参照はこの配置を前提とした相対位置。
- monojoh-authorityのADRを参照する前に、必ず設計正本リポジトリの `docs/redesign/adr-inventory.md`
  (生死棚卸し表)を確認する。要改訂21本・廃止1本があり、単体で読むと逆方向(host-agent広域・mTLS等)に実装する危険がある。
- 移行期間中、旧語彙(AdapterEvent)と新契約(Envelope)の変換はゲートウェイ内ブリッジ1ファイルに限定。
  新規コードはAdapterEventへの依存を増やさない(D4)。

## Build & Test

```bash
cargo test --workspace
cargo test -p <crate-name>
```

## Workflow Rules

- **Main agent は実装禁止。** 対話/spec/Codex eval dispatch のみ。実装は agent team (lead → dev subagent)。
- **Codex eval は全段階で必須。** spec → plan → per-task impl → final impl。dev subagent も自分で `codex exec` を実行する。
- Pipeline: brainstorming → codex-eval-spec → writing-plans → codex-eval-plan → agent-team → PR
- **Watchpoint curation は Main agent の責務。** Lead/Reviewer の結果を受けて eval-perspectives-curator で review guide の Active Watchpoints を更新する。
- **計画作成時は設計追補を全掃引する。** 対象決定文書の監査追記・追補節(「実装と同時」等の指示を含む)を計画の Global Constraints に反映してから書く(D1 quarantine_reason 追補の見落とし再発防止)。

## Reference Docs (read on demand)

| Topic | Path |
|---|---|
| Spec review guide | [docs/eval/spec-review.md](docs/eval/spec-review.md) |
| Plan review guide | [docs/eval/plan-review.md](docs/eval/plan-review.md) |
| Impl spec compliance review | [docs/eval/impl-spec-review.md](docs/eval/impl-spec-review.md) |
| Impl quality review | [docs/eval/impl-quality-review.md](docs/eval/impl-quality-review.md) |
| Codex review history | [codex-review.md](codex-review.md) |
| Agent instructions | [AGENTS.md](AGENTS.md) |

## Commit Style

`feat(crate):` / `fix(crate):` / `refactor(crate):` / `docs:` + Co-Authored-By line.
