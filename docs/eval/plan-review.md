# iotkit-next Plan Review Guide

plan 評価時に Codex プロンプトへ注入する。
**plan 著者も書き始める前にこのドキュメント全体を読むこと。**
Active Watchpoints を先に読み、次に Plan Authoring Discipline、最後に Baseline Checklist を適用する。

## Plan Authoring Discipline

plan を書く前に守るべき規律。ここに書いてある粒度を守らないと、実装エージェントが曖昧なステップを自己判断で埋め、Codex review が発散する。

### 1. 1ステップ = 1アクション（2-5分）

以下はそれぞれ**別のステップ**。1つにまとめない。

```
- [ ] Step 1: failing test を書く（コードブロック必須）
- [ ] Step 2: テストを実行して失敗を確認（コマンド + 期待される失敗メッセージ）
- [ ] Step 3: 最小限の実装を書く（コードブロック必須）
- [ ] Step 4: テストを実行して成功を確認（コマンド + PASS）
- [ ] Step 5: commit（git add + git commit コマンド）
```

**粒度の判定基準:** 実装者がそのステップを読んで「何を打てばいいか」が即座にわかるか？ 考える余地があるなら分割が足りない。

### 2. プレースホルダー禁止

以下は全て **plan の欠陥**。1つでもあったら修正する。

| 禁止パターン | なぜダメか |
|-------------|-----------|
| `// TODO: implement` | 実装者が自己判断する余地を作る |
| `適切なエラーハンドリングを追加` | 「適切」が定義されていない |
| `同様に Task N と同じ` | 実装者は Task N を読み返さないかもしれない。コードを繰り返せ |
| `テストを書く`（コードなし） | テストの内容が spec と一致する保証がない |
| `必要に応じて更新` | 必要かどうかの判断を実装者に委ねている |
| `TBD` / `後で決める` | 決めてから plan を書け |

**ルール:** コードを変更するステップには必ずコードブロックを含める。コマンドを実行するステップには必ず実行コマンドと期待出力を含める。

### 3. spec の状態遷移 → plan のタスクへの展開

spec に状態遷移図がある場合、以下のマッピングが必要:

```
spec の各状態 → plan に「その状態に入る実装」+「その状態のテスト」
spec の各遷移 → plan に「遷移トリガーの実装」+「遷移のテスト」
spec の各 failure mode → plan に「failure 処理の実装」+「failure のテスト」
```

「reconnect を実装する」は 1 タスクとしては粗すぎる。正しくは:

```
Task N: Reconnecting 状態の遷移
  Step 1: EventLoop error → Reconnecting のテスト
  Step 2: テスト実行、失敗確認
  Step 3: eventloop_task に Disconnected イベント処理を実装
  Step 4: テスト実行、成功確認
  Step 5: commit

Task N+1: Reconnecting → Online の復帰
  Step 1: ConnAck 受信 → Online + reconcile のテスト
  Step 2: ...
```

### 4. 型名・関数名の一貫性チェック

plan 完成後に以下を自己チェックする:

- Task 3 で定義した `fn clear_layers()` を Task 7 で `fn clear_full_layers()` と書いていないか
- spec の型名と plan のコード内の型名が完全一致しているか
- `pub` / `pub(crate)` の可視性が全タスク通して一貫しているか

**1つでも食い違いがあれば、実装エージェントがどちらを信じるか不定になる。**

### 5. 各タスクの自己完結性

実装者は**そのタスクだけを読んで作業する**前提で書く。

- 前のタスクで定義した型を参照するなら、import パスを明記する
- 前のタスクのコードに依存するなら、何に依存しているか書く（「Task 3 で作った `MqttState` enum を使う」）
- 共通の前提知識（crate 構成、既存の型）はタスクの冒頭に書く

### 6. テストが振る舞いを検証している

**悪い例:**
```rust
#[test]
fn test_config_loads() {
    let config = Config::from_str(TOML);
    assert!(config.is_ok()); // コンパイルが通るだけ
}
```

**良い例:**
```rust
#[test]
fn test_config_rejects_cert_without_key() {
    let toml = r#"
        [mqtt.tls]
        ca_cert = "/path/to/ca.pem"
        client_cert = "/path/to/cert.pem"
        # client_key is missing
    "#;
    let err = Config::from_str(toml).unwrap_err();
    assert!(err.to_string().contains("client_key is required when client_cert is set"));
}
```

テストは「何が起きるべきか」を assert する。「エラーにならない」だけでは振る舞いの検証ではない。

### 7. 中間ステップで cargo test が通る

全てのタスクの commit 時点で `cargo test --workspace` が通らなければならない。
Task 3 の時点でコンパイルエラーが出て、Task 5 で初めて治る、という plan は不可。

これは依存順序の設計問題。inner crate (types) → outer crate (adapter) → binary の順に作る。

## Active Watchpoints

最近のレビューで観測されたプロジェクト固有の盲点。
max 10 items、デフォルト TTL 3ヶ月。繰り返し出現する項目は Baseline に昇格する。

- Added: 2026-03-29
  Revalidate by: 2026-06-29
  Watchpoint: If the spec contains a state transition diagram, verify that every state + failure mode has a corresponding task/step and test. Missing states in the plan = missing implementation = Codex review will catch it later at higher cost.
  Observed in: Phase 1A — reconnect/disconnect states were in spec prose but not decomposed into plan tasks, leading to 5 rounds of implementation fixes.

- Added: 2026-03-29
  Revalidate by: 2026-06-29
  Watchpoint: Config validation must be tested for every reject path (empty values, partial TLS, invalid URLs, unknown enum variants). "Validate config" as a single step is too coarse — each reject case needs its own test assertion.
  Observed in: Phase 1A — silent fallbacks and half-configured mTLS found repeatedly in review.

## Baseline Checklist

安定的な plan レビュー基準。ポリシー変更時のみ更新する。

### タスク分解

- [ ] 各タスクが独立してコンパイル・テストできる単位になっているか。
- [ ] タスク間の依存が明示されているか（A が終わらないと B が始められない等）。
- [ ] 1タスクの変更ファイル数が多すぎないか（目安: 3-5ファイル以内）。
- [ ] spec の全要件がいずれかのタスクでカバーされているか（漏れがないか）。
- [ ] spec にない作業がタスクに紛れ込んでいないか（スコープ超過）。

### 依存と順序

- [ ] 内側（core/types）から外側（adapter、gateway）への順序になっているか。
- [ ] 型変更が先、利用側の更新が後、という順序が守られているか。
- [ ] 並列実行可能なタスクが明示されているか。
- [ ] Cargo.toml の依存追加が必要なタスクで漏れていないか。
- [ ] `mod` 宣言の追加タイミングがテスト実行より前にあるか。

### 各タスクの完成条件

- [ ] 各タスクに具体的なテストコマンドと期待結果があるか。
- [ ] テストが「コンパイル通過」だけでなく、振る舞いを検証しているか。
- [ ] commit メッセージの粒度が適切か（1タスク1コミットが原則）。
- [ ] 変更対象ファイルのパスが正確か（存在するファイルか、新規作成か）。

### 状態遷移カバレッジ

- [ ] spec に状態遷移図がある場合、全状態が plan のいずれかのタスクで実装されているか。
- [ ] 各状態の failure mode に対応するテストがタスク内に存在するか。
- [ ] 切断/再接続/shutdown の各パスが独立したステップとして分解されているか（「reconnect を実装する」は粒度不足）。
- [ ] spec の failure mode テンプレート（in-flight data、バッファ溢れ、crash recovery 等）の各回答に対応するコードとテストがあるか。

### Anti-Pattern チェック

plan のコードスニペットに以下のパターンが含まれていないか確認:

- [ ] **AP-1 (Remove before confirm):** collection から remove してから side-effect を実行していないか。peek → 成功 → remove の順か。
- [ ] **AP-2 (Async enqueue ≠ delivery):** async publish/write の Ok を「完了」と扱っていないか。flush/grace period/confirmation が必要か。
- [ ] **AP-3 (Silent config fallback):** 不正な設定値をデフォルトに fallback していないか。error exit すべきか。
- [ ] **AP-4 (Lossy identifier encoding):** identifier のエスケープが非可逆でないか。衝突リスクがないか。

### テスト方針

- [ ] 既存テストの更新箇所が列挙されているか。
- [ ] 新規テストが正常系だけでなく異常系もカバーしているか。
- [ ] stub/mock の構築が具体的に書かれているか（「適切なテストを書く」ではない）。
- [ ] ワークスペース全体の `cargo test --workspace` が最終タスクに含まれているか。

### 実装の正確性

- [ ] コードスニペットの型名・関数名が spec と一致しているか。
- [ ] 前のタスクで定義した型を後のタスクで別名で参照していないか。
- [ ] pattern match の網羅性を考慮しているか（新フィールド追加時の既存 match 更新）。
- [ ] `pub` / `pub(crate)` の可視性が意図通りか。

### リスクと見落とし

- [ ] 変更の影響範囲をワークスペース全体で確認しているか（grep 漏れ）。
- [ ] PoC バイナリ、integration test など見落としやすい箇所が含まれているか。
- [ ] feature flag や Cargo.toml の features 追加が必要な箇所を見逃していないか。
- [ ] 既存の re-export（`pub use`）への影響を考慮しているか。

### 定番質問

- このタスク順序で、途中のどの時点でも `cargo test` が通るか。
- 実装者がタスク N だけを読んで、前後のタスクを読まずに作業できるか。
- spec の要件 X は、どのタスクのどのステップで実現されるか。
- テストが実装の正しさを本当に検証しているか、コンパイル通過だけではないか。

### 危ないサイン

- 「既存テストを更新」とだけ書いてあり、具体的な変更内容がない。
- タスク間で型名やフィールド名が微妙に食い違っている。
- 最後のタスクにだけ「全体テスト」があり、途中タスクにテストがない。
- spec にある要件がどのタスクにも対応していない。
- 「同様に」「前タスクと同じ」で具体的なコードが省略されている。

## Maintenance

- 期限切れの watchpoint は明示的に更新されない限り削除する。
- 繰り返し出現する watchpoint は Baseline Checklist に昇格する。
- Active Watchpoints が空の場合は `(none currently)` と記載する。
