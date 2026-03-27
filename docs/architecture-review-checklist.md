# iotkit-next Architecture Review Checklist

`iotkit-next` をアーキテクチャレベルでレビューするときの観点メモ。
PoC、日常レビュー、PRレビューで共通利用する想定。

関連:
- [coding-review-checklist.md](./coding-review-checklist.md)

## 使い方

- まず依存方向と公開契約を見る。
- 次にデバイスライフサイクルと障害系を見る。
- 最後に拡張時の変更点を数える。
- 1項目でも「説明できない」があれば、その変更は構造上の負債候補。

## 依存と境界

- [ ] `core/types` に adapter、transport、protocol の詳細が逆流していないか。
- [ ] crate 依存が一方向になっているか。
- [ ] `AdapterEvent`、`AdapterCommand`、`SensorIdentity`、`DeviceKey` が汎用契約として保たれているか。
- [ ] adapter 固有の値や enum が core の共有型に漏れていないか。
- [ ] transport は open、read、write などの I/O 責務に留まっているか。
- [ ] codec はフレーム分解の責務に留まり、意味解釈を抱え込みすぎていないか。
- [ ] adapter と runtime composition root が分離されているか。

## ライフサイクル

- [ ] `discover`、`update`、`lost`、`error` の発火条件が揃っているか。
- [ ] 型だけ存在して実装経路のない概念が残っていないか。
- [ ] デバイスが一度も `DeviceDiscovered` されない経路がないか。
- [ ] 再接続時に state が二重化しないか。
- [ ] shutdown と reader 再試行が競合しても破綻しないか。
- [ ] partial frame、duplicate frame、unknown device で状態管理が崩れないか。

## センサー抽象

- [ ] sensor driver が入力ソース差を吸収できているか。
- [ ] I2C、UART、GPIO の差分が adapter 側の巨大 `match` に漏れていないか。
- [ ] 同じ `sensor_type` を別 adapter から使うとき、sensor module を再利用できるか。
- [ ] sensor 追加時の変更箇所が増えすぎていないか。
- [ ] sensor identity の生成責務が一箇所に寄っているか。

## 拡張性

- [ ] 新しい sensor type を足すとき、変更箇所を明確に列挙できるか。
- [ ] 新しい adapter を足すとき、共通基盤を再利用できるか。
- [ ] DB、設定、API、監視、起動管理を毎回手書きしなくて済むか。
- [ ] adapter 固有永続化と core 永続化の境界が分離されているか。
- [ ] 将来の pair、scan、DFU、delete-sync をどこに置くか方針があるか。
- [ ] `AdapterCommand` が adapter 固有コマンドの寄せ集めになる兆候がないか。

## 障害と運用

- [ ] 全体障害と個別デバイス障害が区別できるか。
- [ ] `device_key: None` にすべきケースと、特定デバイスに紐づくケースが混ざっていないか。
- [ ] retry 回数、port、device、sensor_type、切断理由がログで追えるか。
- [ ] reader thread 異常終了時の扱いが定義されているか。
- [ ] oversized frame や malformed payload が adapter 全体を汚染しないか。

## UI と API

- [ ] core が adapter 固有 UI を直接知る設計になっていないか。
- [ ] adapter 管理 API の置き場が先に崩れていないか。
- [ ] core の `/adapters` と adapter 固有画面の責務分離が説明できるか。

## レビュー時の定番質問

- この変更で新しい sensor type を足すと、どのファイルを何箇所触るか。
- この変更で新しい adapter を足すと、どこまで共通化が効くか。
- 1台のデバイスが無言で消えたとき、core はいつどう知るか。
- reader が落ちて復帰したとき、discover や state は二重化しないか。
- protocol 詳細を消したあとでも、core の型は意味を保てるか。

## レビューで特に危ないサイン

- 設計メモでは抽象化されているのに、実装の本当の拡張点が別の場所にある。
- adapter が protocol 解釈だけでなく、起動、監視、thread 管理、永続化まで抱え込んでいる。
- event contract にある概念が、正常系の一部でしか流れていない。
- 追加変更のたびに `match` が増え続ける。
