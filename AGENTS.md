# AGENTS.md

## Project Context

`iotkit-next` は旧 `iotkit` をゼロから作り直すオンプレミス優先のIoTデータ収集基盤。
収集側のRust + tokio製`IoTKit Edge Node`（Raspberry Pi向け）と、集約側のGo製
`IoTKit Edge`からなる。

レイヤ:

```text
{core/types, core/supervision} <- {core/engine, adapters} <- iotkit-edge-node
```

取り込み経路はアダプタ内クライアント (`iotkit-ingest-client`) が正 (D4)。
adapters は `core/engine` に依存しない。`AdapterEvent` は engine/監督専用の frozen
vocabulary であり、新規コードは依存を増やさない。

文書の入口と正本の構成は `docs/README.md`。機械表現・共有fixture/conformance test・
現行契約文書を一つの契約成果物として扱い、不一致時は一方へ自動追従させない。
コードの置き場、crate 地図、層規則の正本は
`docs/okf/ja/architecture/system-overview.md` であり、依存方向は `scripts/check-layers` が検査する。
新しい crate を作る場合は、同スクリプトの分類と同文書を同時に更新する。

`docs/redesign/`の用語集・責務台帳・決定文書は、現行文書から参照される理由と不変条件を保持する。
同directoryのinputs/reviews/移行記録と`docs/superpowers/`は履歴であり、現行実装状態や
作業指示を上書きしない。タスク指示と現行契約成果物が矛盾して見える場合は、
勝手に解釈せず作業を止めて報告する。旧実装も正しさの基準にはしない。

## Invariants（絶対に破らない）

- 秘密情報（トークン、credential、鍵）を Debug 出力、ログ、エラー、監査記録に載せない。
- データを黙って失わない。ack の意味は D1 に従う。`rejected` は決定的違反専用で、
  ストレージ失敗には `rejected` を返さない（ack なし）。
- 変更系操作は所有componentの R14 typed dispatch 経由。Edge Nodeは`core/ops`、IoTKit EdgeはGo
  application service内のtyped operation dispatcherを使う。API/UI/CLIからSQLへ直書きする
  変更経路を新設しない。

## Experimental Raspberry Pi

- 実験用PiのSSH接続先は `iotkit@iotkit`。
- Codexの制限環境では、root所有の `/etc/ssh` 設定が `nobody:nobody` に見えることがあり、通常の
  `ssh iotkit@iotkit` が次のエラーで開始前に失敗する。

  ```text
  Bad owner or permissions on /etc/ssh/ssh_config.d/20-systemd-ssh-proxy.conf
  ```

- このエラーをPi側または開発PCの実際の権限不正と断定しない。`/etc/ssh`への`chown`、`chmod`、削除を
  行わない。Codexからはsystem SSH configを読まない次の形を使う。

  ```bash
  ssh -F /dev/null iotkit@iotkit
  ```

- 自動調査では必要に応じて `BatchMode=yes`、短い`ConnectTimeout`、`/tmp`内の一時known-hosts fileを使う。
  sandbox内で名前解決が拒否された場合だけ、同じ読み取り専用SSH commandを承認付きで再実行する。
- 初回接続・診断は読み取り専用とし、Pi上のservice、package、設定、Docker container、UARTを変更または
  openする前に、現在のタスクがその変更を許可しているか確認する。
- 確認済みの基準状態: Debian 13 / arm64、`/dev/serial0 -> /dev/ttyAMA0`、`iotkit`は`dialout`所属。
  BravePI信号は通常停止中なので、ユーザーが再開するまではUART frameが来ないことを異常扱いしない。

## Development Workflow

Superpowers skills は任意の作業支援ツールであり、全変更に一律適用する必須パイプラインではない。
ユーザーの直接指示と本ファイルのプロジェクト規則は、plugin skill の一般的な trigger や成果物要件に
優先する。着手前に現実的なリスクへ応じて次の lane を選び、必要な場合だけ skill を使う。

### Fast lane

局所的な bug、refactor、文書、設定、小機能で、公開契約・認証・custody・data loss・restore・
migration・外部作用へ影響しない作業に使う。

1. 目的と完了条件を短く確認する。
2. 製品挙動が変わる場合は focused test を先に追加または更新する。
3. 実装する。
4. リスクに比例した focused verification を実行する。
5. 結果を報告する。

ユーザーが明示しない限り、design spec、永続 implementation plan、worktree、subagent team、
独立レビューを作らない。

### Standard lane

複数 crate にまたがる変更、新しい内部境界、または有力な実装案が複数ある作業に使う。

1. 原則1ページ以内の簡潔な設計を示す。
2. 重要な選択が残る場合だけ方向を確認する。
3. 永続 plan 文書でなく一時 checklist を使う。
4. 適切なテストとともに実装する。
5. 1回のレビューとリスクに比例した verification を行う。

新しい spec を作るより既存の正本文書を更新する。完了した一時 plan はリポジトリへ残さない。

### Full lane

次に影響する変更だけに使う。

- 公開 ingest / egress wire contract
- 認証・認可・秘密情報
- custody・purge権威・data loss
- DB migration・backup・restore・rollback
- 外部に見える破壊的または不可逆な作用
- 後から変更すると高価な互換性保証

この lane では brainstorming、written design、implementation plan、TDD、独立レビュー、広い検証を
使ってよい。ただし既存 ADR・契約文書へ収まる判断を新しい spec へ重複記載しない。

### 共通規則

- 現実的なリスクを覆う最も軽い lane を既定にする。
- 動くコードと実行可能テストを、重複する説明文より優先する。
- 既存決定の再記述だけを目的に spec や ADR を作らない。
- 過去のspec/planはGit履歴にあり、現行指示ではない。現在の正本と実装を優先する。
- 通常は1レビューで終える。実質的な設計・実装変更があった場合だけ再レビューする。
- 外部モデルレビューはユーザーの明示依頼時だけ行う。
- プロセス成果物を、それが導く実装より大きくしない。
- 完了報告には変更リスクに見合う fresh verification evidence を必ず添える。

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
