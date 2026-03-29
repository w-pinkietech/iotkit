# iotkit-next Plan Review Guide

plan 評価時に Codex プロンプトへ注入する。
Active Watchpoints を先に読み、次に Baseline Checklist を適用する。

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
