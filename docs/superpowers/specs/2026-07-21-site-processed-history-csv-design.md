# Site加工後履歴CSV設計

## 目的

Site Consoleの利用者が、Edgeから受信した生データではなく、Siteで補正・意味付けされた確定済み観測をCSVとして持ち出せるようにする。CSVは特定の外部アプリケーション向けpayloadではなく、IoTKitが保持する汎用的な加工後履歴である。

## 採用方針

- 標準の「CSVをダウンロード」は`semantic_observations_v3`を正本とする加工後CSVを出力する。
- 既存の`raw_records`を出すCSVは「受信した生データ」と明示し、調査用途の副導線として残す。
- 加工後CSVの生成時にルールを再評価しない。保存済みの`rule_revision`、`calibration_revision`、`value_json`をそのまま用いる。これにより過去の設定変更後も、当時確定した結果が変わらない。
- MQTT output adapterが生成するtopicやpayloadはCSVへ含めない。外部サービス固有の変換と、Siteの汎用履歴を混ぜない。

## API

`GET /api/v1/semantic-history.csv`を追加する。認証済みの全ロールが利用できる。

検索条件は既存履歴と同じく、`from`、`to`を必須とし、`signal_ref`、`edge_node_id`を任意とする。期間上限は31日、出力上限は100,000行とする。上限を超えた場合はCSVを途中まで返さず、HTTP 422とする。

CSVはUTF-8 BOM付きで、次の列を持つ。

1. `observed_at`
2. `processed_at`
3. `edge_node_id`
4. `signal_ref`
5. `sensor_name`
6. `rule_name`
7. `kind`
8. `value`
9. `unit`
10. `series_id`
11. `sequence`
12. `observation_id`
13. `rule_revision`
14. `calibration_revision`
15. `source_pub_seq`

`numeric`の単位はセンサー表示設定を反映する。`boolean`、`alarm`、`cumulative_counter`は単位なしとする。値は保存済みJSON scalarの表現を使用する。表計算ソフトの数式注入を避けるため、文字列列には既存のCSV安全化処理を適用する。

## Console

履歴画面の主ボタンを「加工後CSV」にする。その近くに、このCSVが補正・判定・累積ルールの適用後であることを説明する。副ボタンとして「受信した生データCSV」を置き、通信調査向けであることを明示する。

画面内のグラフと「受信した値」表は今回変更しない。加工後CSVと画面内の生データ表示が同一だと誤認されないよう、見出しと補足文で境界を示す。

## 性能と整合性

`semantic_observations_v3`へ観測時刻順の検索インデックスを追加する。CSVレスポンスを書き始める前に上限超過を判定し、途中で切れたCSVを正常応答として返さない。

同じセンサーに通常値、カウント、アラームなど複数ルールがある場合は、該当する全ルールの観測を別行で出力する。ルール未設定または期間内に観測がない場合は、ヘッダーだけのCSVを返す。

## 検証

- Store: 時間・Edge・センサーの絞り込み、複数ルール、単位、並び順、100,000行境界を検証する。
- HTTP: 認証、CSVヘッダー、BOM、列、数式注入対策、422を検証する。
- Console: 加工後CSVを主導線、生データCSVを副導線として表示することを検証する。
- OpenAPI/TypeScript: 新APIを契約へ追加し、生成型の差分検査を通す。
- Browser E2E: 履歴画面に2種類のCSV導線が表示されることを検証する。

