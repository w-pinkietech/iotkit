# IoTKit

[English](README.md) | 日本語

[開発参加ガイド](CONTRIBUTING.ja.md) | [Contributing in English](CONTRIBUTING.md)

オンプレミスを優先し、端末の中で動くIoT観測基盤です。IoTKitはInput Adapterでセンサーを読み、端末内のpipeline（校正、ヒステリシス付き閾値、デバウンス、累積カウント）でObservationへ変換し、固定した公開契約で標準のMQTT Brokerへ公開します。Pinkietなどの業務アプリケーションはBrokerを購読し、Observationを自分のドメインへ対応付けます。

> **現在の製品バージョン: 0.4.0（pre-1.0）。** IoTKitは早期source releaseとして
> 公開しています。0.xの間はAPI、ディスク上のschema、wire contractが変更される可能性があります。
> [GitHub Releases](https://github.com/w-pinkietech/iotkit/releases)と
> [ロードマップ](#ロードマップ)を参照してください。
> [v1互換性方針](docs/product/ja/contracts/compatibility-policy-v1.md)は製品1.0.0から適用し、
> このpre-1.0の状態を変更しません。

現行の製品知識はOKF v0.2形式でも提供しています: [日本語](docs/product/ja/index.md) / [英語](docs/product/en/index.md)。

## このPCでまず試す

Git、Python 3.14以降、Docker Composeを使用できるLinux hostで、repositoryにある
2行の[`iotkit.toml`](iotkit.toml)からloopback限定の試用環境を起動できます。
`trial-sample` Input Adapterと3本のpipelineを持つEdge Nodeと、Mosquitto Brokerが動きます。

```bash
./scripts/iotkit trial validate
./scripts/iotkit trial up
./scripts/iotkit trial watch
```

`watch`は、端末が公開するObservationとstatusを独立したconsumerの視点で1行ずつ表示します。
確認する項目と停止・初期化の手順は[試用profileの手順](docs/product/ja/operations/trial-profile.md)を
参照してください。試用環境は現場導入ではありません。

## なぜ作るのか

現場では、種類の異なるセンサーを、そのたびに信頼性を作り直さずに接続する必要があります。Input Adapterが持つのはセンサーとの通信、設定、読み取り、計測値の対応付けだけで、SQLite、MQTT、再送、認証は持ちません。IoTKitがそれらを一度だけ提供し、Brokerの停止中も収集を続け、「送った」と「どこかに保存された」を混同しません。MQTTのPUBACKが送信責任の境界であり、再送の正はIoTKit自身のoutboxだけです。

IoTKitはこの点で意図的に地味です。端末1台につき**Rustバイナリ1つ + SQLite + systemd**、その間に標準のBrokerがあるだけで、中央の装置は要りません。

## 現在できること

```text
sensor -> Input Adapter -> pipeline -> MQTT Output Adapter -> MQTT Broker -> consumer
          |<---------------- IoTKit Edge Node（端末1台に1つ）------------------>|
```

- **Input Adapter**：BravePI Mainboard（UART）、Raspberry Piの直結I2Cセンサー、評価用の`trial-sample`。汎用のhostがbackoff付きで再起動を管理し、ハードウェアのインタフェースを開けないadapterは致命ではなく報告として扱います。
- **pipeline**（`measurement` / `state` / `accumulated-count`）はSQLiteに保存し、typed operation（今は`nodectl pipeline`、後にConsole）で編集します。評価は状態とoutbox行を保存するトランザクションの中で行い、調整項目の変更ではseriesを保ち、構造項目の変更と明示的なリセットで新しいseriesを始めます。
- **MQTT Output Adapter契約 v1**：`iotkit/v1/edge-node/{edge-node-id}/observation/{pipeline-id}/{kind}`へQoS 1、retain有効、in-flight 1件で公開し、PUBACK後にoutboxから削除します。`status` topicは`online` / `degraded` / `offline`の3値、`faults`の一覧、Will、正常終了時の`offline`を持ちます。payloadは`uptime_ms`（単調時計）と`unix_epoch_ms`（端末が実時計を信頼できるときだけ）の2つの時刻を持ちます。`testdata/observation/v1/`のschemaとfixtureがproducerとconsumerの共通の正です。
- **operator CLI**（`iotkit-edge-nodectl`）：pipelineのimport / export / update / reset、passphraseとtoken、health。

中央のIoTKit Edge、そのcustody契約、業務アプリケーション向けのOutput Adapterは再設計（[#232](https://github.com/w-pinkietech/iotkit/issues/232)）で廃止し、残るEdge Node側の旧経路は[#250](https://github.com/w-pinkietech/iotkit/issues/250)で削除中です。

## 現場への導入

端末にEdge Nodeのバイナリをsystemdで配置し、`[output.mqtt]`を現場のBroker（TLSは`system_roots`または`bundle_only`）に向け、`nodectl pipeline import`でpipeline定義を入れます。手順は[導入と復旧](docs/product/ja/operations/installation-and-recovery.md)、Broker証明書の道具は`scripts/iotkit-broker-cert`です。

## Buildとテスト

toolchainは[mise](https://mise.jdx.dev/)（`mise.toml`）で固定しています。systemパッケージとして`pkg-config`、`libudev-dev`、一気通貫テスト用に`mosquitto`と`mosquitto-clients`が必要です。

```bash
mise install
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check

# MQTT Output Adapter v1契約のfixture（schema、正規形、受信側）
node scripts/check-observation-fixtures.mjs
scripts/test-observation-consumer.sh

# 一気通貫テスト：trial-sample -> Edge Node -> Mosquitto -> 独立したconsumer
scripts/test-journey.sh
```

CIは各PRで、軽量なrepositoryチェック、Rust workspace全体（fmt、clippy、テスト）、
一気通貫テストの3つのlaneを実行し、安定した`required CI` aggregateを公開します
（[`.github/workflows/ci.yml`](.github/workflows/ci.yml)）。一気通貫テストのlane
（`scripts/test-journey.sh`）が再設計の受け入れ証拠で、最小ループに続けて障害注入
（Broker停止、`kill -9`、調整項目の変更、削除、保存失敗、正常終了）を確認します。
テスト方針は[`.agents/testing.md`](.agents/testing.md)にあります。localでは
`scripts/verify.sh --workspace`を明示的な診断に使います。

## Repository構成

| Path | 内容 |
|------|------|
| `edge-node/apps/` | RustのEdge Node daemonとoperator CLIのcomposition root |
| `edge-node/core/` | 端末内のドメイン。crateごとに1責務（`pipeline`、`collector`、`ops`、`storage`、`types`など） |
| `edge-node/ingest/` | Envelope / Ack契約と、Input Adapterが使うプロセス内binding |
| `edge-node/input/` | Adapter host API、conformance testkit、polling runtime、transport、再利用可能なsensor driver |
| `edge-node/adapters/` | BravePI MainboardやRaspberry Pi直結I2Cなど具体的なsensor family統合 |
| `testdata/observation/v1/` | MQTT Output Adapter v1のschemaとfixture。producerとconsumerのテストが共有 |
| `docs/`、`deploy/`、`scripts/`、`review/` | 製品文書、配置資材、自動化、[review suite](review/README.md)の観点 |

crate map全体、層の規則、「新しいcodeをどこに置くか」の表は
[architecture文書](docs/product/ja/architecture/system-overview.md)にあります。収集側の作業は
[`edge-node/README.ja.md`](edge-node/README.ja.md)、具体的なadapterの作業は
[`edge-node/adapters/README.ja.md`](edge-node/adapters/README.ja.md)から始めてください。

開発はCodex Cloudを含め1つのcloneから再開できます。再開の順序とcontext authorityの規則は
[docs/cloud-development.md](docs/cloud-development.md)を参照してください。

## Architectureと契約

- [文書index](docs/README.md) — 読む順序と正本の優先順位。
- [製品モデル](docs/product/ja/concepts/product-model.md) — IoTKitが所有するもの、Observationモデル、設定の所有、pipeline定義。
- [Architecture](docs/product/ja/architecture/system-overview.md) — crate map、配置規則、データの流れ、concurrency。
- [契約](docs/product/ja/index.md#公開契約) — MQTT Output Adapter v1、Input Adapter v1、v1互換性方針。

過去の再設計の決定と完了した実装計画は根拠と追跡のためにrepositoryに残していますが、現行の実行可能な契約と文書indexを上書きしません。

## ロードマップ

- **端末完結の基盤への再設計（[#232](https://github.com/w-pinkietech/iotkit/issues/232)）**：契約、TOML設定、pipeline core、MQTT Output Adapterと、CIゲートとしての一気通貫テスト。**完了**（子Issue 1〜4）。
- **中央層の削除（[#250](https://github.com/w-pinkietech/iotkit/issues/250)）**：旧component、旧契約、旧文書、migrationの新基線。**進行中。**
- **Edge Node上のConsole（#232 子Issue 6）**：pipelineの編集、リセット、障害の表示、短期の入力バッファ。
- **配布**：端末完結の製品が安定した後の導入image、更新、client library。

## License

[Apache License, Version 2.0](LICENSE)で公開しています。
