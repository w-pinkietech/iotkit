# Codex Cloud Development

Status: **Optional operator guide** (2026-07-13)

Codex Cloud は候補実装や調査を別環境で行うための任意ツールであり、通常のローカル開発
パイプラインや `scripts/verify.sh` の一部ではない。Cloud の結果は候補としてローカルへ
戻し、`AGENTS.md`のlaneと検証規則に従って扱う。

## Start or resume

1. `AGENTS.md` でプロジェクト規則、不変条件、権限境界を読む。
2. `docs/README.md`から対象コード、現行契約、architectureを辿る。必要な場合だけ
   `docs/redesign/`の履歴から設計理由を確認する。
3. `git status`, `git log -1`, 対象ファイルで作業状態を実物確認する。

チャット、モデルの記憶、過去の handoff、履歴spec/planは補助情報であり、Git、現在の契約文書、
コード、実行可能テストより優先しない。

## Configure the Cloud environment

Codex environment settings でリポジトリ環境を作り、初期設定とキャッシュ更新に次を使う。

```bash
bash scripts/cloud-setup.sh
```

このスクリプトは `rust-toolchain.toml` と `Cargo.lock` を使い、ローカル plugin や
credential をコピーしない。必要な setup secret は Cloud 環境の secret store に置き、
prompt や repository へ入れない。インターネットアクセスは必要なタスクだけで有効にする。

## Submit and inspect candidate work

Cloud work は uncommitted なローカルファイルを読めない。`cloud/<slug>` などの候補ブランチを
作り、push の承認を得て、ローカル HEAD と remote branch が一致していることを確認してから
送信する。

```bash
export CODEX_CLOUD_ENV=<environment-id>
export CODEX_CLOUD_ALLOW_ARGV_PROMPT=1
scripts/codex-cloud.sh submit impl .review/<task>.md <label> cloud/<slug>

scripts/codex-cloud.sh status <task-id>
scripts/codex-cloud.sh collect <task-id> <label>
scripts/codex-cloud.sh diff <task-id>
scripts/codex-cloud.sh verify-receipt <receipt-path>
```

wrapper は dirty、unpushed、stale、remote と不一致の branch を拒否する。prompt は
`.review/` 配下の secret を含まない通常ファイルとし、100,000 bytes 以下にする。
CLI は query を process argument で運ぶため、`CODEX_CLOUD_ALLOW_ARGV_PROMPT=1` は同一
ホストの process から観測され得ることへの明示的な承認である。secret や個人情報を
prompt に入れない。

Best-of-N は attempt ごとの取得結果を確実に識別できないため無効であり、
`CODEX_CLOUD_ATTEMPTS=1` を維持する。送信が中断した場合は自動再送しない。二重課金を
避けるため、保存された pending receipt と output を使い `list` / `status` で既存 task を
探す。

`codex cloud apply` は wrapper が公開しない。Cloud の status/diff だけでは実際に checkout
された source commit を証明できないため、取得した diff を読み、期待する base と変更範囲を
ローカル Git で確認してから候補ブランチへ取り込む。

## Authority and completion

- Cloud agent は調査、実装、テストを候補ブランチ上で行える。
- Cloud task URL、status、receipt、diff は運用診断と候補評価の材料であり、それだけで
  実装の正しさや merge 可否を証明しない。
- ローカルへ戻した候補は、`AGENTS.md`のlane、review、verification規則に従う。
- push、PR、merge、release、追加 attempt の課金は、それぞれ別のユーザー承認を要する。
- product decision と実装はこの repository に残す。外部 repository や会話だけを
  実装 authority にしない。

セッション終了時は、継続に必要な設計判断を正本文書またはcommitに残す。一時作業手順を
永続文書として追加しない。
一時的な task ID、receipt、診断 output は必要な期間だけ安全に保存する。
