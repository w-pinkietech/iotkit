# iotkit-next Implementation Spec Compliance Review Guide

実装がタスク仕様に合っているかを評価する。
Codex プロンプトへ注入し、spec compliance reviewer subagent が使用する。

Active Watchpoints を先に読み、次に Baseline Checklist を適用する。

## Active Watchpoints

max 10 items、デフォルト TTL 3ヶ月。繰り返し出現する項目は Baseline に昇格する。

(none currently)

## Baseline Checklist

### API と責務

- [ ] public 関数や public 型は、その crate の責務に見合う最小範囲か。
- [ ] private にできる実装詳細を `pub` にしていないか。
- [ ] protocol 固有の数値やバイト列解釈が core 側に漏れていないか。
- [ ] domain 変換が codec や transport に紛れ込んでいないか。
- [ ] 追加変更で `match` や `if` の集中点が肥大化していないか。

### 仕様カバレッジ

- [ ] plan task の全ステップが実装されているか。
- [ ] plan task にない変更が追加されていないか（スコープ超過）。
- [ ] 型名、フィールド名、関数シグネチャが plan と一致しているか。
- [ ] `pub` / `pub(crate)` の可視性が plan の意図通りか。
- [ ] pattern match が網羅的に更新されているか（新フィールド追加時）。

### テスト仕様適合

- [ ] plan で指定されたテストが実装されているか。
- [ ] テストが plan の期待結果と一致しているか。
- [ ] plan で指定されたテストコマンドが通るか。

### 定番質問

- plan task の要件 X は、コードのどの箇所で実現されているか。
- plan にない追加変更はないか。あるなら、なぜ必要だったか。
- 変更の影響範囲は plan が想定した範囲に収まっているか。

### 危ないサイン

- plan と実装で型名やフィールド名が微妙に食い違っている。
- plan のステップが飛ばされている。
- plan にない「改善」が混入している。
- テストがコンパイル通過のみで振る舞いを検証していない。

## Maintenance

- 期限切れの watchpoint は明示的に更新されない限り削除する。
- 繰り返し出現する watchpoint は Baseline Checklist に昇格する。
- Active Watchpoints が空の場合は `(none currently)` と記載する。
