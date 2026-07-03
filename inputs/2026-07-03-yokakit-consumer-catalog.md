# YokaKit 消費者ニーズ・カタログ（IoTKit 出口契約への入力）

**Status: 抽出物（設計決定ではない）。**
証拠元 = `yokakit-next` @ `main` commit `88b8abaf03eb99e30723ddd03fb97ad58909f8f2`（2026-07-03 時点）、および `YokaKit`（legacy Laravel、パリティ確認のみ・narrow参照）。

本書は設計キュー3（出口契約 = IoTKit が消費者に公開する北向き契約面, R11）の入力である。
**ここでは設計判断を行わない。** 「YokaKit（消費者アプリ）が実際に何を必要としているか」を実コードから証拠付きで棚卸しし、あわせて「YokaKit固有の命名・スキーマがIoTKit出口契約に漏れ込むリスク」を明示的にフラグする。

---

## 0. 調査範囲と方法

- 主機体: `yokakit-next/`（Go単一バイナリ + Vue3、正準リファレンス）。`docs/architecture.md`・`docs/mqtt-protocol.md`・`docs/database-schema.md`・`docs/api.md`・`plan.md` は読了したが、いずれも `TBD` / `<!-- Populated in Phase X -->` のスタブであり実体は無い（`docs/architecture.md:24-26`、`docs/mqtt-protocol.md:15-17`、`docs/database-schema.md:3-5`、`docs/api.md:3-6`）。実際の契約情報は `CLAUDE.md`・`internal/mqtt/*`・`internal/database/migrations/*`・`internal/repository/postgres.go`・`internal/ws/*`・`internal/handler/*`・`cmd/yokakit/main.go`・`testdata/mqtt/*.json` から抽出した。
- 従機体: `YokaKit/`（legacy Laravel）は、yokakit-next のMQTT購読トピック構成が legacy と一致するかのパリティ確認、および `SensorType` 列挙値の意味確認のためだけに参照した（`YokaKit/app/laravel/app/Console/Commands/MqttSubscribeCommand.php`、`YokaKit/app/laravel/app/Enums/SensorType.php`）。
- go未実行・テスト未実行。コード読解のみ。

---

## 1. ドメインエンティティとセンサー観測の対応

証拠: `internal/database/migrations/001_master_data.up.sql`, `002_production_data.up.sql`, `004_changeover_intervals.up.sql`, `005_breakdown_intervals.up.sql`, `006_gantt_chart.up.sql`。

| エンティティ | テーブル（yokakit-next固有命名） | 依存するセンサー観測 | 備考 |
|---|---|---|---|
| 工程 process | `processes`（`internal/database/migrations/001_master_data.up.sql:36-44`） | 直接のセンサー依存なし。`raspberry_pi_id`経由で間接的に紐づく | `process_name`がMQTT側の`processName`解決キー（barcode経由、後述） |
| ライン line（工程内の計数チャンネル） | `lines`（`001_master_data.up.sql:94-110`） | `production`トピック（pinNumber経由） | `pin_number`+`raspberry_pi_id`が一意（`UNIQUE (pin_number, raspberry_pi_id)` `001_master_data.up.sql:109`） |
| 品番 part number | `part_numbers`（`001_master_data.up.sql:64-71`） | `barcode`トピック（barcode文字列で解決） | `barcode`列がバーコード値と1:1（`UNIQUE`, `001_master_data.up.sql:67`） |
| サイクルタイム cycle time | `cycle_times`（`001_master_data.up.sql:73-83`、`process_id`+`part_number_id`複合） | 直接のセンサー依存なし（マスタ設定値） | OEE計算のPlanCount算出に必須（`internal/domain/production/oee.go:143-146`） |
| 作業者 worker | `workers`（`001_master_data.up.sql:85-92`）、稼働中は`producers`（`002_production_data.up.sql:84-95`） | センサー観測なし（HTTP `PUT /api/switch/{processId}/worker` で手動割当。`cmd/yokakit/main.go:612`） | IoTKit出口契約には無関係と推定（**未確認**: MQTT経由の作業者ID自動検出が将来的にありうるかは未確認） |
| センサー sensor | `sensors`（`001_master_data.up.sql:112-123`） | `alarm`トピック | `identification_number`+`raspberry_pi_id`で解決（`internal/mqtt/resolver.go:130-157`） |
| ON/OFF設定 on_off | `on_offs`（`001_master_data.up.sql:125-137`） | `onoff`トピック | `pin_number`+`raspberry_pi_id`で解決（`internal/mqtt/resolver.go:161-188`） |
| ガントチャート信号 gantt_chart | `gantt_charts`（`006_gantt_chart.up.sql:2-18`） | `gantt-chart`トピック | `pin_number`+`raspberry_pi_id`で解決（`internal/mqtt/resolver.go:198-217`）。BASE(2)=基準信号(電源ON)、WORK(3)=稼働信号、`docs/superpowers/specs/2026-03-29-gantt-chart-design.md:39-46` |
| 計画停止 planned outage | `planned_outages`（`001_master_data.up.sql:46-53`）、`process_planned_outages`, `production_planned_outages` | センサー観測なし。時刻ベースの管理者設定（`start_time`/`end_time` TIME型） | IoTKitからの観測は不要。純粋にYokaKit側マスタ設定（詳細は§5） |
| 生産数 production count | `production_lines.count`（`002_production_data.up.sql:28-44`）、履歴は`productions`（`002_production_data.up.sql:57-73`） | `production`トピック | 装置は生の累積カウントを送る。差分計算はyokakit-next側で実施（後述§2） |
| 不良数 defective count | `defective_productions`（`002_production_data.up.sql:76-82`） | `production`トピック（`lines.defective=true`のライン） | 良品/不良品は物理的に別ピン（別ライン）として区別。センサー側で良否判定済み |
| 稼働状態 production status | `production_histories.status`（`002_production_data.up.sql:16`, enum 1=RUNNING/2=CHANGEOVER/3=BREAKDOWN/4=COMPLETE, `internal/domain/production/status.go:8-13`） | **直接センサー由来ではない**。`production`トピックの到着間隔から`BreakdownDetector`が推定（`internal/domain/production/breakdown.go:8-55`、`internal/mqtt/handlers/production.go:284-292`） | 重要: 設備停止(BREAKDOWN)はIoT側が明示通知するイベントではなく、「configured `over_time`以内にカウントが来ない」ことの**沈黙検知**。IoTKit出口契約が真似すべきは生カウントストリーム＋タイムスタンプそのものであり、「ダウンタイム」という完成イベントではない可能性が高い |
| 段取り替え changeover | `changeover_intervals`（`004_changeover_intervals.up.sql`） | 間接（`production`トピックのカウント到着でRUNNINGに戻る、`internal/mqtt/handlers/production.go:200-242`）。開始はHTTP手動操作（`switch_handler.go`, `cmd/yokakit/main.go:610-611`） | センサー起点ではなく状態機械+手動操作起点 |
| ブレークダウン breakdown | `breakdown_intervals`（`005_breakdown_intervals.up.sql`） | 上記と同じ沈黙検知ロジック | 同上 |
| アラーム/アンドン状態 alarm | `sensor_events`（`002_production_data.up.sql:97-110`） | `alarm`トピック | `trigger`(bool)と`signal`(bool)の一致で「開始/終了」を判定（`internal/mqtt/handlers/alarm.go:108-116`） |
| CPU/ヘルス heartbeat | `raspberry_pis.cpu_temperature`/`cpu_utilization`（`001_master_data.up.sql:26-34`） | `heartbeat`トピック | デバイス単位（pin無し）。イベント履歴化されず最新値のみ上書き（`internal/mqtt/handlers/heartbeat.go:27-31`） |
| バーコード履歴 barcode history | `barcode_histories`（`002_production_data.up.sql:124-131`） | `barcode`トピック | 品番切替が成功した場合のみ保存（`internal/mqtt/handlers/barcode.go:117-124`） |

---

## 2. 現状のMQTT取り込み契約（事実上のワイヤ契約）

証拠: `internal/mqtt/client.go:15-23`（トピック一覧・QoS）, `internal/mqtt/message.go`（ペイロードschema）, `testdata/mqtt/*.json`（実例）, `CLAUDE.md`（トピック表）。

### トピック一覧（フラット・デバイス非名前空間化）

| topic | QoS | 用途 | 証拠 |
|---|---|---|---|
| `heartbeat` | 1 | デバイス死活監視（CPU温度/使用率） | `internal/mqtt/client.go:17` |
| `production` | 2 | 生産数カウント信号 | `internal/mqtt/client.go:18` |
| `barcode` | 2 | 品番バーコード読取 | `internal/mqtt/client.go:19` |
| `alarm` | 2 | センサーアラーム | `internal/mqtt/client.go:20` |
| `onoff` | 2 | ON/OFFイベント | `internal/mqtt/client.go:21` |
| `gantt-chart` | 2 | 設備稼働信号（BASE/WORK） | `internal/mqtt/client.go:22`（yokakit-nextで新規追加。legacy Laravelには存在せず — `YokaKit/app/laravel/app/Console/Commands/MqttSubscribeCommand.php:68-72`に5トピックのみ） |

- 購読はブローカー全体に対しトピック名でフラットに行う。`c.paho.Subscribe(t, qos, ...)`（`internal/mqtt/client.go:105`）。デバイスやテナントによるトピック階層分離は無い（例: `<device>/production`のような構造はない）。
- yokakit-nextは**Publishしない**（`grep -rn "\.Publish("` で該当ゼロ）。デバイスへの制御コマンド送信経路は存在せず、完全な片方向（デバイス→yokakit-next）取り込み。

### ペイロードschema（全てJSON、バイナリなし）

証拠: `internal/mqtt/message.go:39-150`、実例は`testdata/mqtt/*.json`。

- `HeartbeatMessage`: `{ipAddress: string, cpuTemperature: float64, cpuUtilization: float32}`（`message.go:40-44`, 実例 `testdata/mqtt/heartbeat.json`）。単位は未指定（推定: ℃, %。**未確認**）。
- `ProductionMessage`: `{ipAddress: string, pinNumber: FlexInt, count: int}`（`message.go:47-51`, 実例 `testdata/mqtt/production.json`）。`count`は累積値。yokakit-next側で前回値との差分（オフセット）を計算（`internal/mqtt/handlers/production.go:143-153`）。
- `BarcodeMessage`: `{ipAddress: string, macAddress: string, barcode: string}`（`message.go:54-58`, 実例 `testdata/mqtt/barcode.json`）。
- `AlarmMessage`: `{ipAddress: string, pinNumber: FlexInt, sensorType: int, signal: bool, value: float64}`（`message.go:61-67`, 実例 `testdata/mqtt/alarm.json`）。`sensorType`はBraveJIG側の数値コード（legacy `YokaKit/app/laravel/app/Enums/SensorType.php:26-35`: UNKNOWN=0, GPIO_INPUT=0x0101, GPIO_OUTPUT=0x0102, AMMETER=0x0103, DISTANCE=0x0104, THERMOCOUPLE=0x0105, ACCELERATION=0x0106, DIFFERENCE_PRESSURE=0x0107, ILLUMINANCE=0x0108, OTHER=0xFFFF）。`value`の単位はセンサー種別ごとに暗黙（**未確認**: 単位変換ロジック・単位フィールドはコード上に存在せず、生の数値がそのままDB保存される）。
- `OnOffMessage`: `{ipAddress: string, pinNumber: FlexInt, onOff: bool}`（`message.go:70-74`, 実例 `testdata/mqtt/onoff.json`）。
- `GanttChartMessage`: `{ipAddress: string, pinNumber: FlexInt, signal: bool}`（`message.go:135-139`）。
- `FlexInt`型: `pinNumber`はJSON数値でも文字列("17")でも受理する互換レイヤー（`message.go:9-37`、legacy互換コメントあり: "Legacy IoT devices may send pinNumber as either \"17\" or 17"）。

### フィールド名・単位・タイムスタンプの扱い

- フィールド名はキャメルケース（`ipAddress`, `pinNumber`, `macAddress`, `cpuTemperature`など）。これはBravePI/BraveJIGデバイス側プロトコルの命名であり、YokaKitのテーブル/カラム名（スネークケース: `ip_address`, `pin_number`）とは別物。**ここは正しく分離されている**（§6参照）。
- **タイムスタンプはペイロードに一切含まれない。** 全メッセージともサーバー受信時刻（`time.Now()`）をイベント時刻として採用（`internal/mqtt/handlers/production.go:122`、`barcode.go:74`、`gantt_chart.go:60`）。デバイス側にRTC/NTPクロックがある前提を置いていない、または実装されていない。IoTKit出口契約がタイムスタンプを供給する場合、「デバイス側時刻」と「受信時刻」のどちらを正とするかは**未確認・要決定事項**。
- デバイス識別は「トピック名」ではなく「ペイロード内の`ipAddress`」で行う。`ipAddress → raspberry_pi_id`解決はDBルックアップ＋インメモリキャッシュ（`internal/mqtt/resolver.go:103-126`）。センサー/ON-OFF/ガントチャートはさらに`(raspberry_pi_id, pinNumber)`または`(raspberry_pi_id, identification_number)`の複合キーで二段解決する（`resolver.go:130-217`）。マスタ変更時は`InvalidateAll()`でキャッシュ全クリア（`resolver.go:221-228`）。
- 認証/TLSはオプトインだが既定無効。デバイス側がMQTT認証/TLSに未対応のため（`CLAUDE.md`「Constraints」節、`docker/mosquitto/config/mosquitto.conf:7-8`: `allow_anonymous true`）。

---

## 3. 消費者が必要とするレコード/イベント種別

証拠: `internal/ws/bridge.go`, `internal/mqtt/handlers/*.go`, `internal/domain/production/oee.go`。

| イベント/レコード種別 | 発生契機 | 粒度 | 証拠 |
|---|---|---|---|
| 生産数サマリ (`ProductionSummary`) | `production`/`barcode`トピックの処理成功後、`indicator`ラインのカウント変化時 | 工程単位、都度（イベント駆動、ポーリングなし） | `internal/ws/bridge.go:16-33`, `internal/mqtt/handlers/composite.go:63-73` |
| 良品数/不良品数 (`good_count`/`defect_count`) | 上記に同梱。`count_switch`設定により「カウンタが総数か良品のみか」の解釈が変わる | 工程単位・累積値のスナップショット | `internal/repository/postgres.go:2076-2100`(finalizeProductionSummary) |
| OEE指標一式（`good_rate`, `time_operating_rate`, `performance_operating_rate`, `oee`, `achievement_rate`, `cycle_time`） | 生産数変化のたびにサーバー側で都度再計算（クライアントには計算済み値のみ配信） | 工程単位、リアルタイム | `internal/domain/production/oee.go:110-186`, `internal/ws/bridge.go:16-33` |
| センサーアラーム/アンドン状態 (`AlarmEvent`) | `alarm`トピック受信、センサー解決成功時 | センサー単位、状態変化(`trigger==signal`)の開始/終了で1件ずつ | `internal/mqtt/handlers/alarm.go:104-131`, `internal/ws/bridge.go:36-43` |
| ON/OFFイベント (`OnOffEvent`) | `onoff`トピック受信。`off_message`がNULLの場合はブロードキャスト抑制（BR-018） | on_off設定単位、都度 | `internal/mqtt/handlers/onoff.go:92-104`, `internal/ws/bridge.go:46-51` |
| 設備稼働（ガントチャート）イベント | `gantt-chart`トピック。信号変化時のみ記録（変化なしはスキップ） | ピン単位（BASE/WORK）、状態変化ごと | `internal/mqtt/handlers/gantt_chart.go:45-98`, `internal/ws/bridge.go:78-90` |
| ハートビート/デバイス死活 | `heartbeat`トピック | デバイス単位、最新値のみ（履歴化なし） | `internal/mqtt/handlers/heartbeat.go:27-31` |
| バーコード履歴 | `barcode`トピック（品番切替成功時のみ） | 都度 | `internal/mqtt/handlers/barcode.go:121-124` |
| 生産履歴一覧・詳細（過去分） | HTTP GETリクエスト時にDBから集計 | 工程単位・期間指定なし（1品番切替=1履歴行） | `internal/handler/history_handler.go:78-97` |
| 生産推移タイムポイント（チャート用） | HTTP GET、`productions`テーブルの時系列そのまま返却 | ラインごとの時刻+カウント+稼働時間 | `internal/handler/history_handler.go:66-75, 236-250` |
| ガントチャート履歴（Excel/期間） | HTTP GET、開始/終了イベントの境界拡張クエリ | チャートごとON/OFF区間、日付範囲指定 | `internal/repository/postgres.go:2502-2527` |

補足: アラームは同時にSlack通知にもファンアウトされる（`internal/mqtt/handlers/alarm.go:118-121`）が、これはYokaKit内部の副作用であり、IoTKit出口契約が直接関与すべき事項ではないと考えられる（**設計判断ではないため断定は避ける**）。

---

## 4. データの適時性と粒度の要件

| 用途 | 経路 | レイテンシ要件 | 粒度 | 保持期間 | 証拠 |
|---|---|---|---|---|---|
| アンドンボード（全工程の稼働状況） | MQTT→Go→WebSocket `summary`/`alarm`/`onoff`チャンネル | イベント駆動即時配信（ポーリングなし）。WS再接続は指数バックオフ最大30秒、10回試行で断念 | 工程単位の最新スナップショット | 該当なし（ライブのみ） | `web/frontend/src/composables/useWebSocket.ts:46-57`, `internal/ws/hub.go:159-165`（送信不可なクライアントは即Evict、ブロッキングしない） |
| ガントチャート（当日ダッシュボード） | WebSocket `gantt-chart`チャンネル＋初期ロードは`GET /api/gantt-charts/all`（当日スコープ限定） | リアルタイム | ピン単位のON/OFF区間 | 当日のみ（無制限フルスキャン回避、`plan.md:32,90`「scoped to today, not unbounded」） | `cmd/yokakit/main.go:588`, `internal/repository/postgres.go:2529-2549` |
| ガントチャート履歴（期間指定） | `GET /api/processes/{id}/gantt-charts/history` | オンデマンド（バッチ的） | チャートごとの区間、日付範囲は必ずクエリで境界指定 | 保持期間はRETENTION_DAYSに従う（既定90日、後述） | `internal/repository/postgres.go:2502-2527`（"always time-bounded", `plan.md:31`） |
| 生産履歴・OEEレポート | `GET /api/processes/{id}/histories`（ページネーション） | オンデマンド | 品番切替1回=履歴1行、ページング（デフォルト10件/ページ、最大100件） | RETENTION_DAYS | `internal/handler/history_handler.go:294-321` |
| データ保持全般 | 日次パージ処理 | N/A | テーブルごとに`cutoff = now - retentionDays`で削除、子→親の順（`productions`等→`production_histories`） | 既定**90日**（`RETENTION_DAYS`環境変数、最小7日にクランプ） | `internal/config/config.go:125`, `internal/retention/purge.go:33-76`, `README.md:69`(`RETENTION_DAYS` デフォルト値表) |
| メモリ制約 | 全体 | Raspberry Pi 4上でRSS目標 < 200MB | 全時系列クエリは時間範囲必須（無制限全表スキャン禁止） | — | `CLAUDE.md`, `plan.md:7`「Memory constraint」 |

---

## 5. 消費者が必要としないもの（過剰公開回避の材料）

以下はコード上「YokaKitが使っていない/生成していない」ことが確認できた、またはIoT側からの直接観測に依存しないと判断できる領域。**断定できない箇所は明記する。**

- **計画停止（planned_outages）はセンサー観測に依存しない。** 開始/終了時刻は管理者がマスタ画面で設定するTIME型（`internal/database/migrations/001_master_data.up.sql:46-53`）。IoTKitからのイベント供給は不要。
- **段取り替え(changeover)の開始トリガーはMQTT起点ではなくHTTP起点。** `PUT /api/switch/{processId}/changeover/start`（`cmd/yokakit/main.go:610`）。終了のみ`production`カウント到着で自動遷移。つまりIoTKitの出口契約に「changeover開始」という専用イベントは不要（生カウントストリームで十分）。
- **ブレークダウン(breakdown)も同様に、IoTKit側の明示イベントではなく沈黙検知（タイマー）による導出。** `internal/domain/production/breakdown.go:8-55`。出口契約に「ダウンタイム発生イベント」を持たせる必要は必ずしもない（生カウント+タイムスタンプがあれば消費者側で導出可能）。ただし`gantt-chart`のBASE信号（電源ON/OFF）は生の設備稼働シグナルであり、これは直接必要（§1参照）。
- **作業者(worker)の自動検出はMQTT経由ではない。** HTTPで手動アサイン（`cmd/yokakit/main.go:612`）。IoT側からの作業者ID通知は見当たらない（**未確認**: 将来デバイス側でRFID等による自動検出があるかは未確認、現コードには存在しない）。
- **センサー`value`の単位変換・正規化はコード側で一切行っていない。** 生の`float64`をそのままDB保存（`internal/mqtt/handlers/alarm.go:92-102`）。つまりIoTKit出口契約に「単位付き値」を要求する根拠はyokakit-next側の実装からは出てこない（現状は単位非依存の生値パススルー）。
- **yokakit-nextはMQTTをPublishしない。** デバイスへの制御コマンド（例: 起動/停止指示）を送る経路はコード上存在しない。出口契約は片方向（IoTKit→YokaKit）で足りると考えられる（**未確認**: 将来的な双方向要求の有無は本調査の範囲外）。
- **バイナリペイロードは使われていない。** 全トピックJSON（`internal/mqtt/message.go`全体、`encoding/json`のみ使用）。

---

## 6. 結合リスク（YokaKit固有語彙がIoTKit出口契約に漏れ込む危険箇所）

これは提案ではなく、**現状コードにおいてYokaKit固有の命名がどこまで各層に浸透しているか**の事実整理。IoTKit出口契約の設計者が「うっかりこの語彙をコアに持ち込まないための」チェックリストとして使うことを想定。

1. **MQTTトピック名がYokaKitのユースケース/テーブル名と一致している。**
   `gantt-chart`トピック（`internal/mqtt/client.go:22`）は、YokaKitのUI機能名「ガントチャート」およびDBテーブル`gantt_charts`/`gantt_chart_events`（`internal/database/migrations/006_gantt_chart.up.sql`）と同一語彙。`onoff`トピックもテーブル`on_offs`と同一。`barcode`トピックもテーブル`barcode_histories`と同一。`production`トピックもテーブル`productions`と同一。
   → これらはデバイス種別/観測種別の命名ではなく「YokaKitが何に使うか」で名付けられたトピックである。IoTKit出口契約がこれをそのまま踏襲すると、コアの北向き契約面がYokaKit専用語彙で汚染される。

2. **WebSocketチャンネル名がMQTTトピック名・DB概念と直結。**
   `internal/ws/hub.go:14-17`の`ChannelSummary`("summary")・`ChannelAlarm`("alarm")・`ChannelOnOff`("onoff")・`ChannelGanttChart`("gantt-chart")は、MQTTトピック名（"alarm", "onoff", "gantt-chart"）とほぼ1:1。フロントエンドも`useWebSocket(['summary', 'alarm', 'onoff'])`（`web/frontend/src/components/domain/AndonBoard.vue:76`）とベタ書き。
   → MQTT→DB→WS→UIの全層が同一語彙で貫通しており、「装置が送る生シグナル種別」と「YokaKitが表示する画面要素」が命名上分離されていない。IoTKit側でこの語彙を継承すると、コアが「YokaKitの画面構成」を暗黙に知っていることになる。

3. **`ProductionSummary`構造体がYokaKit固有ビジネスルールをフィールドとして直接埋め込んでいる。**
   `count_switch`（良品のみカウントか総数カウントか、`internal/repository/postgres.go:2076-2089`）は、YokaKit工程マスタの設定フラグそのもの。OEE式の変数名（`good_rate`, `time_operating_rate`, `performance_operating_rate`, `achievement_rate`）もJIS/TPM用語ではあるものの、`plan_count`（目標数、YokaKitの`goal`概念）等はYokaKit運用ルール由来。
   → IoTKit出口契約がこれらの「計算済み指標」を直接運ぶ設計にすると、コアがYokaKitの計算ロジック（count_switchの解釈、goal概念）を代行することになり、provider中立の原則に反する。

4. **センサー解決キーがYokaKitのマスタテーブル構造（`sensors`, `on_offs`, `gantt_charts`の各`(raspberry_pi_id, pin/identification_number)`複合キー）に強く依存。**
   `internal/mqtt/resolver.go`全体。IoT側は`ipAddress`+`pinNumber`（またはidentification_number）しか送らず、それを「何の意味か」に変換する責務が完全にYokaKit側マスタに委譲されている。これ自体はデバイス⇔業務意味のマッピングとして自然だが、IoTKitの出口契約がこの「解決後の意味（process_name, alarm_text, event_name等の日本語ラベル）」まで運ぶ設計になると、コアがYokaKitのラベル管理を肩代わりすることになる。
   → 出口契約は「どのデバイス／どのピンで何が起きたか（デバイス座標系）」までに留め、「それが業務的に何を意味するか（YokaKit座標系）」への変換はYokaKit側アダプタの責務として残すべきという論点が浮かぶ（**これは設計論点として記録するのみで、本書では裁定しない**）。

5. **`sensorType`の数値コード（0x0101等）はBraveJIGデバイス側語彙であり、YokaKit語彙ではない。** これは逆にコアに残してよい可能性がある語彙の例（§2参照）。ただし現状コードでは`SensorTypeUnknown`/`SensorTypeOther`という命名・フォールバック規則（`internal/mqtt/handlers/alarm.go:14-19`）がYokaKit側実装内に存在しており、コード上の所在は要整理。

6. **`gantt-chart`機能自体がlegacy Laravelに存在せずyokakit-next独自追加。** つまりこの機能は「IoTKit側が今後も安定して提供すべき普遍的な出口契約要素」なのか、「YokaKitというアプリの一機能要求」に過ぎないのかの切り分けが必要（**設計判断は本書の範囲外**）。少なくとも「BASE/WORK信号」という汎用的な設備稼働シグナルの必要性自体は普遍的だが、"gantt-chart"という命名・テーブル構造は明らかにYokaKit UI機能名からの逆引き設計であり、そのままコア語彙に採用するのは避けるべき候補の筆頭。

---

## 未確認事項一覧（推測で埋めなかった箇所）

- `heartbeat`ペイロードの`cpuTemperature`/`cpuUtilization`の単位（℃/%と推定するが明示的なコード上の記述なし）。
- `alarm`ペイロードの`value`フィールドの単位・スケール（センサー種別ごとに異なるはずだが、コード上に変換・注記なし）。
- デバイス側にRTC/NTPがあるか、将来ペイロードにタイムスタンプが追加される計画があるか。
- 作業者(worker)の将来的なMQTT経由自動検出（RFID等）の有無。
- Slack通知（アラーム）がIoTKit出口契約の関心事に含まれるべきかどうか（現状はYokaKit内部の副作用として実装されているのみ）。
- `gantt-chart`機能がIoTKit出口契約で普遍的に必要とされる「設備稼働シグナル」の代表例なのか、YokaKit固有機能に留まるのかの位置づけ。
