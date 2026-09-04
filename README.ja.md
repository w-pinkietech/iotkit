# IoTKit

[English](README.md) | 日本語

[開発参加ガイド](CONTRIBUTING.ja.md) | [Contributing in English](CONTRIBUTING.md)

オンプレミスを優先し、データの完全性を重視するIoT収集基盤です。IoTKit Edge Nodeへ対象を絞ったセンサーAdapterを追加すると、IoTKitが耐久収集、再送、IoTKit Edgeへの明示的な保管責任移転を提供します。IoTKit Edgeが耐久保存するまで、データは削除可能になりません。

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

```bash
./scripts/iotkit trial validate
./scripts/iotkit trial up
```

表示に従って試用管理者のpasswordを決め、`http://127.0.0.1:8080`を開いて
`admin`でログインします。変化する照度（三角波）と接点状態（矩形波）のsampleは
DBやConsoleへ直接seedされず、Input Adapter、Edge Nodeの保管責任、標準MQTT Broker、
IoTKit Edgeの通常経路を通ります。確認方法と片付け方は
[試用profileガイド](docs/product/ja/operations/trial-profile.md)を参照してください。
試用環境は現場導入には使用できません。

## なぜ作るのか

製造現場では、センサーを追加するたびに信頼性の仕組みを作り直さず、さまざまなセンサーを接続する必要があります。Adapterが担当するのは、センサーとの通信、設定、読取り、測定値への写像だけです。SQLite、MQTT、再送、保持、認証は所有しません。IoTKitがこれらを一度提供し、通信断の間も収集を継続し、停電後もデータを黙って失わず安全に復旧します。

IoTKit Edge Nodeは意図的に単純です。**1つのRust binary + SQLite + systemd**で構成し、Node上にcontainer orchestration、ML基盤、中央rule engineを置きません。Edge Nodeは倉庫ではなくbufferであり、IoTKit Edgeが耐久保存を確認するまでデータを保持します。MQTT PUBACKだけでは、その確認になりません。IoTKit Edgeはrecordを耐久保存し、commit後にだけ各Edge Nodeの`accepted-through`を進めます。また、raw query、archive query、センサーの意味付け、application出力のIoTKit側境界を提供します。

IoTKit Edgeは保存済みsignalを汎用的な`numeric`、`boolean`、`cumulative_value`、`alarm`へ写像し、別のOutput Adapterがapplication向けMQTT contractへ変換します。製品、工程、OEE、業務アラーム、業務UI、通知はPinikietなどのapplicationが所有します。

## 現在できること

```text
 vendor/protocol device ──▶ Input Adapter ──┐
 contract-native device ──▶ HTTPS ingest ───┴─▶ IoTKit Edge Node
                                                   │ SQLite readings + outbox
                                                   ▼
                                            internal MQTT Broker
                                                   │
                                                   ▼
                                              IoTKit Edge
                                                   ├─ durable raw records and Edge Node cursors
                                                   ├─ generic semantics
                                                   └─ Output Adapter ──▶ external Broker ──▶ application
```

- 停電を通常の事象として扱う、crash-consistentな**耐久取り込み**。
- 機器名の変更やhardware交換でも履歴を分断しない**series identity**。
- 標準語彙と導入環境ごとのoverrideを持つ**measurement registry**、未知・範囲外データのrow/series quarantine。
- **Edge Node保管責任契約:** 標準Brokerを介したIoTKit Edgeへのat-least-once MQTT配送とtarget別cursor。IoTKit Edgeの耐久`accepted-through`だけが保持処理による削除を許可し、未ackのoriginalは古くても保護されます。[Edge Node保管責任契約](docs/product/ja/contracts/edge-node-custody-v1.md)を参照してください。
- **認証付きHTTP取り込み:** 既定offのLAN向けTLS listenerが、device別bearer credential、上限制御、位置対応item結果、重複再送、副作用のないvalidationを持つJSON Envelopeを受理します。[認証付きingest契約](docs/product/ja/contracts/ingest-v1.md)を参照してください。
- device ledger、measurement registry、snapshot/restore、IoTKit Edge targetを操作する**operator CLI**（`iotkit-edge-nodectl`）。
- 範囲指定history/CSV、storage診断、暗号化backup、新規pathへのrestoreを提供する**IoTKit Edge運用機能**。
- 新規または復元済み状態はlocal ownership/recoveryを必要とし、network setup routeは公開しません。復旧後はdevice tokenとoperator権限を再検査します。

Edge Node host故障またはhardware交換では、
[Edge Node hardware復旧クイックガイド](docs/product/ja/operations/edge-node-hardware-recovery.md)から開始してください。
- control-plane APIはprivate LANからの到達を前提とします。private routed pathがない環境ではSSH port forwardingを使用します。

### Edge Nodeの初期化

新しいEdge Node DBを作り、生成されたidentityを表示します。既存DBがある場合は変更せず拒否します。

```bash
iotkit-edge-nodectl --db edge-node.db init
iotkit-edge-nodectl --db edge-node.db identity
iotkit-edge-nodectl --db edge-node.db mqtt-binding
```

後二つはread-onlyです。`mqtt-binding`はEdge Nodeが使うusername、client ID、topic、QoS、retainを表示しますが、credentialの生成や表示は行いません。

Broker、IoTKit Edge、Edge Nodeの起動後、合成commissioning recordをenqueueし、どちらのSQLite schemaも直接読まずにIoTKit Edgeの耐久ackを確認できます。

```bash
smoke=$(iotkit-edge-nodectl --db edge-node.db smoke enqueue)
iotkit-edge-nodectl --db edge-node.db smoke status \
  --ledger-epoch "$(jq -r .ledger_epoch <<<"$smoke")" \
  --pub-seq "$(jq -r .pub_seq <<<"$smoke")"
```

`delivered`は、通常のMQTT recordがIoTKit Edgeの耐久raw storageへ到達し、対応する`accepted-through`がEdge Node cursorを進めたことを示します。smoke recordはセンサー測定値ではなく、semantic projectionから除外されます。

### IoTKit Edgeの導入bootstrap

運用を想定した基準構成では、Raspberry Pi上でEdge Nodeをnative実行し、LinuxのIoTKit Edge host上で標準BrokerとIoTKit EdgeをDocker Composeにより実行します。既存のfull-chain server certificate、private key、root trust bundleを準備してください。証明書発行、DNS、firewall、任意のVPNはIoTKit Edge operatorの責務です。Broker hostnameはIoTKit Edge host上で明示したbind addressへ解決でき、certificateがそのhostnameを対象に含む必要があります。

最初に、初期化済みEdge Nodeからsecretを含まないbindingを出力し、そのJSONをIoTKit Edge operatorへ渡します。

```bash
iotkit-edge-nodectl --db /var/lib/iotkit/edge-node.db mqtt-binding > edge-node-mqtt-binding.json
```

IoTKit Edge host上のrepository cloneで、Composeを実行する非root accountからbootstrapを実行します。出力directoryはGit repository外にある新規pathで、既存のoperator所有directoryの配下でなければなりません。

```bash
install_root="$HOME/.local/share/iotkit/edge-01"
mkdir -p "$(dirname "$install_root")"
scripts/bootstrap-edge.sh \
  --binding ./edge-node-mqtt-binding.json \
  --output-dir "$install_root" \
  --broker-host mqtt.edge.example \
  --broker-bind 192.0.2.10 \
  --tls-cert /secure/path/server-fullchain.pem \
  --tls-key /secure/path/server.key \
  --tls-ca /secure/path/broker-ca.pem \
  --edge-publish-topic 'pinikiet/v1/sources/iotkit-01/sensors/press-sensor/observations' \
  --edge-publish-topic 'pinikiet/v1/sources/iotkit-01/status'
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml up --build --detach
```

上記は小規模・standalone向けの`embedded` profile（SQLite）です。同じConsoleとMQTT契約を維持しながら、より大きな常設環境で管理対象PostgreSQL profileを使う場合は、bootstrapへ`--storage-profile postgres`を追加し、起動時にもoverlayを追加します。

```bash
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml -f deploy/compose.edge-postgres.yaml \
  up --build --detach
```

profileは導入directoryの`storage-profile.json`へ固定され、起動flagと異なる場合は停止します。接続失敗時にSQLiteへfallbackせず、両DBへの二重書込みもしません。SQLiteからのoffline移行は[IoTKit Edgeの導入と復旧](docs/product/ja/operations/installation-and-recovery.md)を参照してください。

generatorは、anonymousを無効にしたBroker設定、Edge Node専用ACLとhash済みpassword DB、IoTKit Edge secret、`edge-handoff/`を作ります。三つのhandoff fileをEdge Nodeへ安全に転送してください。`mqtt-password`と`broker-ca.pem`を`edge-mqtt.toml`が指定するpathへ配置し、Edge Node service account所有、mode `0600`とします。Edge Node再起動前にTOML fragmentを設定へmergeします。転送成功後はIoTKit Edge host上の`edge-handoff/`を削除してください。credentialをargv、environment variable、log、Gitへ置いてはいけません。

IoTKit Edge host上で、ownerだけが読める一時fileを使って最初のIoTKit Edge ownerを作ります。

```bash
install -m 600 /dev/null "$install_root/secrets/initial-admin-password"
# shell historyへ残さない方法で初期passwordを書き込む。
docker compose --env-file "$install_root/edge.env" -f deploy/compose.edge.yaml run --rm \
  -v "$install_root/secrets/initial-admin-password:/run/iotkit/admin-password:ro" \
  edge account bootstrap --storage-profile "$(sed -n 's/^IOTKIT_STORAGE_PROFILE=//p' "$install_root/edge.env")" \
  --db /data/edge.db \
  --postgres-config "$(sed -n 's/^IOTKIT_POSTGRES_CONFIG=//p' "$install_root/edge.env")" \
  --storage-metadata /run/iotkit/storage-profile.json --login-id admin \
  --display-name 'システム管理者' --password-file /run/iotkit/admin-password
rm "$install_root/secrets/initial-admin-password"
```

Windows browserから`IOTKIT_EDGE_ORIGIN`を開きます。LANへ公開するHTTPS endpointはCaddyだけであり、IoTKit EdgeのHTTP listenerは`127.0.0.1`に留めます。全画面でloginが必要です。`viewer`は閲覧、`admin`は機器・信号・意味・出力の設定、`system_admin`はさらにaccount発行ができます。

起動後は上記のcommissioning smoke commandを実行してください。bootstrapを再実行しても既存の出力directoryは置換しません。診断、password recovery、certificate renewal、rollbackは[IoTKit Edgeの導入と復旧](docs/product/ja/operations/installation-and-recovery.md)を参照してください。

### IoTKit Edgeの意味付けとapplication出力

Consoleの**信号**画面で、補正、threshold/hysteresis、boolean state、累積値count、alarm behaviorを設定します。5分間のlive previewはpreview開始後に受け取った観測だけを使い、mappingやoutput eventを書き込みません。保存すると将来分だけに適用する新しいrevisionが始まり、過去のraw recordを黙って再計算しません。

**出力**画面で、IoTKit Edgeが発行した`source-id`とセンサー単位の`sensor-id`を使い、用途（`production`、`onoff`、`gantt_chart`、`alarm`）をPinikietへ出力します。同じセンサーの全用途は一つのtopicを共有し、Pinikietへの登録も一度だけです。用途ごとの`series-id`と`sequence`は独立します。IoTKitの累積値がPinikietの`kind=production`になるのは、このAdapter内だけです。出力はQoS 1で、Broker PUBACKまでIoTKit Edge outboxに残ります。Pinikiet statusはretained source statusとして別にpublishします。

Edge NodeとIoTKit Edgeを結ぶ内部Brokerと、外部application Brokerは別にできます。それぞれのendpoint、trust bundle、client ID、credentialをdeployment設定として配置し、external profileを`serve`の`--output-*` flagで渡します。Consoleは状態を表示しますが、Broker credentialは変更できません。

## Buildとテスト

[`rust-toolchain.toml`](rust-toolchain.toml)で固定したtoolchain（Rust 1.98.0）が必要です。`rustup`が自動的に導入します。

```bash
# 対象crateのRust feedback
cargo test -p <owning-crate>
cargo clippy -p <owning-crate> --all-targets -- -D warnings
cargo fmt --all --check

# 明示した場合だけのworkspace全体診断。通常のPR sweepではない
scripts/verify.sh --workspace

# Docker Mosquittoによる外部Output Adapter、PUBACK、再接続ゲート
scripts/test-edge-output.sh
IOTKIT_TEST_STORAGE_PROFILE=postgres scripts/test-edge-output.sh

# SQLite/PostgreSQL共通契約と短時間capacity回帰smoke
scripts/test-edge-postgres.sh
scripts/test-edge-capacity.sh

# OpenAPI生成Console型、TypeScript、埋め込みJavaScriptの同期
npm ci --prefix edge/frontend
scripts/test-edge-console-frontend.sh

# Chromiumによるlogin、Edge Node登録、センサー設定、意味付け、外部出力、権限導線
scripts/test-edge-console-e2e.sh
IOTKIT_TEST_STORAGE_PROFILE=postgres scripts/test-edge-console-e2e.sh

# v1 host統合ゲート。新しいreport directoryを指定する
scripts/test-edge-host-release-gate.sh /secure/report/iotkit-v1-YYYYMMDD
```

IoTKit ConsoleはRustの型付きserver-side renderingを使い、browser動作を`edge/frontend/src/`のTypeScriptで実装します。JSON API型は`edge/openapi/edge-console-v1.yaml`から生成します。配布物にはesbuild済みの`static/console.js`を埋め込むため、IoTKit Edgeの実行環境にNode.jsは不要です。

CIは各PRで、軽量なrepositoryチェック、Rust workspace全体（fmt、clippy、テスト）、
一気通貫テストの3つのlaneを実行し、安定した`required CI` aggregateを公開します
（[`.github/workflows/ci.yml`](.github/workflows/ci.yml)）。一気通貫テストのlaneは現在、
MQTT Output Adapter v1のfixtureを実際のMosquittoへ流して購読側で照合します。
[#232](https://github.com/w-pinkietech/iotkit/issues/232)の再設計がpublishできる段階で、
sample Input AdapterからBrokerを経て独立したconsumerまで製品を動かすテストへ育てます。
テスト方針は[`.agents/testing.md`](.agents/testing.md)にあります。
localでは`scripts/verify.sh --workspace`を明示的な診断に使い、現行製品のrelease前には
`test-edge-host-release-gate.sh`を一度実行します。

## Repository構成

| Path | 役割 |
|------|------|
| `edge-node/apps/` | Rust製Edge Node daemonとoperator CLIのcomposition root |
| `edge-node/core/` | 耐久収集domain。1 crate 1責務 |
| `edge-node/ingest/` | Envelope/Ack contract、in-process binding、認証付きHTTP binding |
| `edge-node/input/` | Adapter host API、conformance testkit、polling runtime、transport、共有sensor driver |
| `edge-node/adapters/` | BravePI MainboardやRaspberry Pi直結I2Cなどの具体的sensor family統合 |
| `edge/` | Rust製IoTKit Edge、Console、raw/semantic store、cursor管理、application出力 |
| `docs/`, `deploy/`, `scripts/`, `testdata/`, `review/` | 共有contract、導入、automation、component横断fixture、[レビュースイート](review/README.ja.md) の perspective |

crate全体図、layer rule、新しいcodeの配置表は[Architecture](docs/product/ja/architecture/system-overview.md)にあります。
収集側は[`edge-node/README.ja.md`](edge-node/README.ja.md)、具体的Adapterは
[`edge-node/adapters/README.ja.md`](edge-node/adapters/README.ja.md)、Edge/Consoleは
[`edge/README.ja.md`](edge/README.ja.md)から読み始めてください。

Codex Cloudを含め、単一cloneから開発を再開できます。再開順序とcontext authorityは[docs/cloud-development.md](docs/cloud-development.md)を参照してください。

## Architectureと契約

- [ドキュメント入口](docs/README.md) — 読む順序と正本の優先関係。
- [製品モデル](docs/product/ja/concepts/product-model.md) — IoTKitの所有範囲、component境界、外部applicationに残すもの。
- [Architecture](docs/product/ja/architecture/system-overview.md) — crate map、配置rule、data flow、custody、concurrency。
- [契約](docs/product/ja/index.md#contracts) — device ingest、Input Adapter、Edge Node保管責任、Output Adapterの境界。

過去のredesign決定と完了済みimplementation planは、理由と追跡可能性のためrepositoryに残します。ただし、現行の実行可能契約やdocumentation indexを上書きしません。

## ロードマップ

- **Wave 0 —「自分たちの環境で動く」:** ingest、registry、ledger、retention、snapshot/restore、operator CLI。**完了。**
- **初期実装gate:** paired BravePI temperature sensor → BLE Long Range → BravePI Mainboard → UART → IoTKit Edge Node → standard MQTT Broker → IoTKit Edge → raw SQLite → direct CLI query。実機経路、再起動・停止matrix、storage failure injection、bounded capacity、application `accepted-through`を検証済みです。purge eligibilityは検証済み`accepted-through`の後だけ進みます。**完了。**
- **IoTKit Edgeのsemantic/output slice:** 汎用numeric/boolean/cumulative/alarm、live preview、no backfill、耐久Output Adapter境界、合意済みPinikiet source/signal observation contract。**実装済み。**
- **Wave 1 —「他者へ配布できる」:** onboarding、calibration、configuration authority、その他の配布品質向上。既存HTTP ingressとcontrol-plane実装は残しますが、現在の完了条件ではありません。
- **Wave 2 —「公開OSS」:** client library、A/B update、OS image。

## License

[Apache License, Version 2.0](LICENSE)で提供します。
