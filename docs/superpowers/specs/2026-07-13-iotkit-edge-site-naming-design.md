# IoTKit Edge / Site 命名変更設計

Date: 2026-07-13
Status: 承認済み

## 目的

`Gateway`という名称が単純な中継器を連想させ、実際に担っている現場収集、正規化、耐久保存、再送を十分に表せていない問題を解消する。

IoTKitをRaspberry Pi上の単一プログラム名ではなく、現場側の`IoTKit Edge`と拠点側の`IoTKit Site`からなるオンプレミス優先のIoTデータ収集基盤として再定義する。旧名称を互換名として残さず、公開前の正式なMQTT v1契約も新名称へ統一する。

## 製品と構成要素の名称

| 対象 | 正式名称 | 会話上の短縮 | 日本語での説明 |
|---|---|---|---|
| 製品全体 | `IoTKit` | IoTKit | オンプレミス優先のIoTデータ収集基盤 |
| Raspberry Pi側 | `IoTKit Edge` | Edge | 現場収集ノード |
| Edgeのアーキテクチャ上の役割 | `Edge Node` | Edge Node | センサーデータを収集、保全、配送するノード |
| 拠点側 | `IoTKit Site` | Site | 単一拠点の集約、保管、照会、application接続点 |
| MQTT配送 | `MQTT Broker` | Broker | IoTKitが自作しない標準MQTT broker |
| 外部製品 | `BravePI Mainboard` | Mainboard | 製品正式名。改名しない |
| 外部製品 | `BravePI Transmitter` | Transmitter | 製品正式名。改名しない |

`Node`単独はセンサーノード、brokerクラスタノードなどと衝突するため使わない。`Gateway`は業界カテゴリを説明する一般名の`IoT gateway`では使用できるが、IoTKitの現行構成要素名、識別子、実行ファイル名には使わない。

`Adapter`、`Collector`、`Publisher`、`outbox`、cursorは責務が変わらないため維持する。

## 構成とデータフロー

```text
Sensor
  -> BravePI Transmitter
  -> BLE Long Range
  -> BravePI Mainboard
  -> UART
  -> IoTKit Edge
       - BravePI Adapter
       - Collector
       - SQLite readings + outbox
       - MQTT Publisher
  -> MQTT Broker
  -> IoTKit Site
       - MQTT Consumer
       - raw canonical record storage
       - Edgeごとのcursor
       - query
       - application export boundary
  -> YokaKit / external systems / optional cloud
```

Edgeはセンサー固有信号をcanonical recordへ変換し、Siteが保管責任を引き受けるまでSQLite outboxへ保持する。Siteはraw canonical recordと連続cursorを同一transactionで耐久保存した後にだけ`accepted-through`を返す。MQTT PUBACKはtransport受領だけを表し、Edgeのcursorやpurge eligibilityを進めない。

## IoTKit Siteの責任境界

IoTKit Siteは保存専用サービスではない。次をSiteの責任範囲とする。

- 複数Edge Nodeからのcanonical record受信
- raw canonical recordの耐久保存
- Edge Nodeごとのcursor管理と`accepted-through`
- site-level query
- YokaKitなどのapplication接続点
- 保存済みseriesを外部MQTTや他システムへ届けるapplication export境界
- 将来のEdge接続状態、欠測状況などのsite-level集約

旧IoTKitにあった、センサーと`production`等の意味やMQTT topicを対応付ける機能は、EdgeではなくSiteのapplication exportに属する。Siteは保存済みseriesへ設定可能なセンサー意味を付け、設定済み出力先へのルーティング・投影を担う。品番・工程master、生産実績、OEE、alarm文言などの業務データとロジックはYokaKit等のapplicationが所有する。

今回の命名変更ではapplication exportの責任境界だけを明記し、MQTT Export機能そのものは実装しない。

## コードと実行単位の名称

| 旧名称 | 新名称 |
|---|---|
| `iotkit-gateway/` | `iotkit-edge/` |
| `iotkit-gateway` | `iotkit-edge` |
| `iotkit-gatewayctl/` | `iotkit-edgectl/` |
| `iotkit-gatewayctl` | `iotkit-edgectl` |
| Rust crate identifier `iotkit_gateway` | `iotkit_edge` |
| `iotkit-site-server/` | `iotkit-site/` |
| `iotkit-site-server` | `iotkit-site` |
| `gateway_identity` | `edge_node_id` |
| `GatewayIdentity` | `EdgeNodeID` |
| fixture identity `gateway-01` | `edge-node-01` |
| default hostname `iotkit-gateway.local` | `iotkit-edge.local` |

directory、package、binary、CLI、Go module、Docker image、compose service、script、help、error、log、TLS hostnameを新名称へ揃える。

## MQTT v1 wire contract

公開前の旧形式を破棄し、新名称を最初の正式なv1とする。`schema_version: 1`とtopic内の`/v1/`は維持し、v2や旧形式との二重対応は作らない。

```text
iotkit/v1/edge-nodes/{edge_node_id}/records
iotkit/v1/edge-nodes/{edge_node_id}/accepted-through
```

Record batch:

```json
{
  "schema_version": 1,
  "edge_node_id": "edge-node-01",
  "ledger_epoch": "epoch-01",
  "publication_id": "edge-node-01:epoch-01:1:1",
  "cursor_start": 1,
  "cursor_end": 1,
  "records": []
}
```

Application acknowledgement:

```json
{
  "schema_version": 1,
  "edge_node_id": "edge-node-01",
  "ledger_epoch": "epoch-01",
  "publication_id": "edge-node-01:epoch-01:1:1",
  "accepted_through": 1
}
```

`through`は指定番号を含む連続範囲まで処理済みであることを表すcursor表現として維持する。`accepted`は現在のSQLite実装のcommit操作ではなく、Siteが耐久保存後に保管責任を引き受けた契約上の状態を表す。このため`committed-through`や`archived-through`へ変更しない。

`ledger_epoch`はEdge台帳の世代UUIDである。通常再起動では変わらず、snapshot復元では新規採番する。同じ`edge_node_id`を名乗る旧世代を拒否するフェンスであり、Site側の大域レコード同一性は次とする。

```text
(edge_node_id, ledger_epoch, pub_seq)
```

MQTT usernameは`edge_node_id`へ束縛し、ACLは当該Edge Nodeのrecords publishとaccepted-through subscribeだけを許可する。Siteは逆の権限を持つ。

## DBと旧形式の扱い

Edge DBのidentity metadataとSite DBの列・主キーを`edge_node_id`へ変更する。旧DBからのmigration、旧JSON field alias、旧topic subscribe、旧binary aliasは実装しない。

製品コードが旧DBを自動削除してはならない。旧schemaを指定して起動した場合は、対応していないpre-release schemaであることを明示して失敗する。実験用PiのDB、設定、MQTT資格情報は、ローカルとDockerで新構成を検証した後、今回の明示的な実験作業として削除・再作成する。

## 文書の扱い

次の現行資料は新名称へ更新する。

- `README.md`
- `AGENTS.md`
- `CLAUDE.md`
- `docs/architecture.md`
- `docs/exit-contract.md`
- `docs/redesign/terminology.md`
- `docs/redesign/responsibility-ledger.md`
- `docs/redesign/decisions/`の現行決定
- 現行の構成図
- build、deploy、operationに関する現行資料

`docs/redesign/reviews/`等の過去のレビュー記録は当時の判断を保存するため書き換えない。現行資料から参照するときだけ、当時の旧称であることを必要に応じて注記する。

## 一斉切り替え

変更は一つのbranch・PRで完結させ、意味のある単位でcommitを分ける。masterへmergeされる最終状態では旧名と新名を混在させない。

実装順は次を基本とする。

1. 用語集と設計正本
2. wire contractと共有fixture
3. Rust Edge本体、CLI、DB
4. Go Site、DB
5. MQTT ACL、compose、Docker、script
6. README、architecture、現行構成図
7. 実験環境の再構築と確認

## テスト方針

作業途中は、変更箇所に直接関係するfocused testだけを実行する。Rust workspace全体test、workspace全体Clippy、Go全package test、Docker end-to-end、実機試験を作業中に繰り返さない。

コード修正がすべて終わり、PRを作成する直前に次の最終検証を一度まとめて実行する。

1. `scripts/verify.sh`
2. Go全package test
3. `scripts/test-site-mqtt.sh`
4. 履歴資料等の許容箇所を除いた旧名称の残存検査
5. 実験用Piの再初期化と、BravePI MainboardからIoTKit Siteまでの実機smoke test

最終検証で失敗した場合はPRを作らない。原因箇所をfocused testで修正し、成功証拠が必要な検査を再実行する。検査回数を守るために失敗を未確認のまま進めない。

## 対象外

- 旧DB、旧wire、旧binaryとの互換処理
- DB migration
- `production`等へのMQTT Export実装
- YokaKit側の変更
- AWS連携
- Adapter、Collector、custody、cursorの機能変更
- `ledger_epoch`、`accepted-through`の意味変更
- BravePI製品名の変更
- 過去のレビュー資料の書き換え
- リポジトリ名`iotkit-next`の変更

## 完了条件

- 現行コードと正本文書が`IoTKit Edge`、`IoTKit Site`へ統一されている
- `gateway_identity`と`/gateways/`が現行実装に残っていない
- 履歴資料と一般カテゴリの`IoT gateway`だけが旧語の許容箇所である
- 新しいMQTT v1契約でEdgeとSiteが相互運用できる
- Siteの耐久保存後に返る`accepted-through`だけがEdge cursorを進める
- 旧形式が暗黙に受理されない
- 最終検証が成功する
- 実験用PiでBravePIからSiteまでの経路を再現できる
- 秘密情報がログ、fixture、commitへ入らない
