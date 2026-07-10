# iotkit-next Spec Review Guide

spec/design 評価時に Codex プロンプトへ注入する。
Active Watchpoints を先に読み、次に Baseline Checklist を適用する。

## Active Watchpoints

最近のレビューで観測されたプロジェクト固有の盲点。
max 10 items、デフォルト TTL 3ヶ月。繰り返し出現する項目は Baseline に昇格する。

- Added: 2026-03-27 (renewed 2026-07-08 — untriggered since creation; kept because 計画6 のネットワーク入口は fan-in ループを新設する。次の期限でも未検証なら削除)
  Revalidate by: 2026-10-08
  Watchpoint: `biased` in `tokio::select!` creates starvation risk when one channel has sustained traffic — applies to any fan-in or adapter loop.
  Observed in: rpi-local-adapter impl review.

- Added: 2026-03-27 (renewed 2026-07-08 — untriggered since creation; kept because 計画6 はリスナー・トークン等の新しい設定面を追加する。次の期限でも未検証なら削除)
  Revalidate by: 2026-10-08
  Watchpoint: Partial config surfaces (some env vars, some hardcoded) create an awkward middle ground that invites silent misconfiguration. Either fully hardcode or fully expose.
  Observed in: rpi-local-adapter gateway integration.

## Baseline Checklist

安定的なアーキテクチャレビュー基準。ポリシー変更時のみ更新する。

### 依存と境界

- [ ] `core/types` に adapter、transport、protocol の詳細が逆流していないか。
- [ ] crate 依存が一方向になっているか。
- [ ] 取り込みは Envelope(`iotkit-ingest-contract`)経由か。`AdapterEvent`/`AdapterCommand` は凍結レガシー語彙(`core/types` 在住。監督・旧南向き用、D4/D12)——新規の使用箇所や新規バリアント追加を前提とする設計になっていないか。
- [ ] adapter 固有の値や enum が core の共有型に漏れていないか。
- [ ] transport は open、read、write などの I/O 責務に留まっているか。
- [ ] codec はフレーム分解の責務に留まり、意味解釈を抱え込みすぎていないか。
- [ ] adapter と runtime composition root が分離されているか。

### ライフサイクル

- [ ] `discover`、`update`、`lost`、`error` の発火条件が揃っているか。
- [ ] 型だけ存在して実装経路のない概念が残っていないか。
- [ ] デバイスが一度も `DeviceDiscovered` されない経路がないか。
- [ ] 再接続時に state が二重化しないか。
- [ ] shutdown と reader 再試行が競合しても破綻しないか。
- [ ] partial frame、duplicate frame、unknown device で状態管理が崩れないか。

### センサー抽象

- [ ] sensor driver が入力ソース差を吸収できているか。
- [ ] I2C、UART、GPIO の差分が adapter 側の巨大 `match` に漏れていないか。
- [ ] 同じ `sensor_type` を別 adapter から使うとき、sensor module を再利用できるか。
- [ ] sensor 追加時の変更箇所が増えすぎていないか。
- [ ] sensor identity の生成責務が一箇所に寄っているか。

### 拡張性

- [ ] 新しい sensor type を足すとき、変更箇所を明確に列挙できるか。
- [ ] 新しい adapter を足すとき、共通基盤を再利用できるか。
- [ ] DB、設定、API、監視、起動管理を毎回手書きしなくて済むか。
- [ ] adapter 固有永続化と core 永続化の境界が分離されているか。
- [ ] 将来の pair、scan、DFU、delete-sync をどこに置くか方針があるか。
- [ ] `AdapterCommand` が adapter 固有コマンドの寄せ集めになる兆候がないか。

### 障害と運用

- [ ] 全体障害と個別デバイス障害が区別できるか。
- [ ] `device_key: None` にすべきケースと、特定デバイスに紐づくケースが混ざっていないか。
- [ ] retry 回数、port、device、sensor_type、切断理由がログで追えるか。
- [ ] reader thread 異常終了時の扱いが定義されているか。
- [ ] oversized frame や malformed payload が adapter 全体を汚染しないか。

### 配布と既定値

- [ ] 新しい設定・リスナー・機能の既定値は「何も設定しない設置者」にとって安全側か（bind 既定・認証が効く前の窓・自動有効化）。危険側の既定を選ぶ場合、設置手順に明示の確認ステップが対で定義されているか。
- [ ] その構成で前提となる設定の入れ忘れ（例: custody を期待する構成での archive target 未登録）が起きたとき、黙って劣化せず health・ログで気づけるか。

### UI と API

- [ ] core が adapter 固有 UI を直接知る設計になっていないか。
- [ ] adapter 管理 API の置き場が先に崩れていないか。
- [ ] core の `/adapters` と adapter 固有画面の責務分離が説明できるか。

### 定番質問

- この設計が足す新しい面(リスナー・エンドポイント・常駐タスク)は配置4段([1]デバイス〜[4]クラウド)のどの箱に住み、設置者の手順に何を足すか(docs/architecture.md「Site anatomy」)。
- この変更で新しい sensor type を足すと、どのファイルを何箇所触るか。
- この変更で新しい adapter を足すと、どこまで共通化が効くか。
- 1台のデバイスが無言で消えたとき、core はいつどう知るか。
- reader が落ちて復帰したとき、discover や state は二重化しないか。
- protocol 詳細を消したあとでも、core の型は意味を保てるか。

### 危ないサイン

- 設計メモでは抽象化されているのに、実装の本当の拡張点が別の場所にある。
- adapter が protocol 解釈だけでなく、起動、監視、thread 管理、永続化まで抱え込んでいる。
- event contract にある概念が、正常系の一部でしか流れていない。
- 追加変更のたびに `match` が増え続ける。

## Maintenance

- 期限切れの watchpoint は明示的に更新されない限り削除する。
- 繰り返し出現する watchpoint は Baseline Checklist に昇格する。
- Active Watchpoints が空の場合は `(none currently)` と記載する。
