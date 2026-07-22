# IoTKit

[English](README.md) | 日本語

オンプレミスを優先し、データの完全性を重視するIoT収集基盤です。IoTKit Edge Nodeへ対象を絞ったセンサーAdapterを追加すると、IoTKitが耐久収集、再送、IoTKit Edgeへの明示的な保管責任移転を提供します。IoTKit Edgeが耐久保存するまで、データは削除可能になりません。

> **状態: v1リリース候補。** BravePIの温度・接点入力、1台以上のRust製IoTKit Edge Node、標準MQTT Broker、認証付きIoTKit Console、将来分だけに適用する意味付け、YokaKitへの耐久MQTT出力まで、一連の経路を実装済みです。API、ディスク上のschema、wire contractは今後変更される可能性があります。[ロードマップ](#ロードマップ)を参照してください。

現行の製品知識はOKF v0.1形式でも提供しています: [日本語](docs/okf/ja/index.md) / [英語](docs/okf/en/index.md)。

## なぜ作るのか

製造現場では、センサーを追加するたびに信頼性の仕組みを作り直さず、さまざまなセンサーを接続する必要があります。Adapterが担当するのは、センサーとの通信、設定、読取り、測定値への写像だけです。SQLite、MQTT、再送、保持、認証は所有しません。IoTKitがこれらを一度提供し、通信断の間も収集を継続し、停電後もデータを黙って失わず安全に復旧します。

IoTKit Edge Nodeは意図的に単純です。**1つのRust binary + SQLite + systemd**で構成し、Node上にcontainer orchestration、ML基盤、中央rule engineを置きません。Edge Nodeは倉庫ではなくbufferであり、IoTKit Edgeが耐久保存を確認するまでデータを保持します。MQTT PUBACKだけでは、その確認になりません。IoTKit Edgeはrecordを耐久保存し、commit後にだけ各Edge Nodeの`accepted-through`を進めます。また、raw query、archive query、センサーの意味付け、application出力のIoTKit側境界を提供します。

IoTKit Edgeは保存済みsignalを汎用的な`numeric`、`boolean`、`cumulative_value`、`alarm`へ写像し、別のOutput Adapterがapplication向けMQTT contractへ変換します。製品、工程、OEE、業務アラーム、業務UI、通知はYokaKitなどのapplicationが所有します。

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
- **Edge Node保管責任契約:** 標準Brokerを介したIoTKit Edgeへのat-least-once MQTT配送とtarget別cursor。IoTKit Edgeの耐久`accepted-through`だけが保持処理による削除を許可し、未ackのoriginalは古くても保護されます。[Edge Node保管責任契約](docs/okf/ja/contracts/edge-node-custody-v1.md)を参照してください。
- **認証付きHTTP取り込み:** 既定offのLAN向けTLS listenerが、device別bearer credential、上限制御、位置対応item結果、重複再送、副作用のないvalidationを持つJSON Envelopeを受理します。[認証付きingest契約](docs/okf/ja/contracts/ingest-v1.md)を参照してください。
- device ledger、measurement registry、snapshot/restore、IoTKit Edge targetを操作する**operator CLI**（`iotkit-edge-nodectl`）。
- 範囲指定history/CSV、storage診断、暗号化backup、新規pathへのrestoreを提供する**IoTKit Edge運用機能**。
- 新規または復元済み状態はlocal ownership/recoveryを必要とし、network setup routeは公開しません。復旧後はdevice tokenとoperator権限を再検査します。
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
  --edge-publish-topic 'yokakit/v1/sources/iotkit-01/signals/press-count/observations' \
  --edge-publish-topic 'yokakit/v1/sources/iotkit-01/status'
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml up --build --detach
```

上記は小規模・standalone向けの`embedded` profile（SQLite）です。同じConsoleとMQTT契約を維持しながら、より大きな常設環境で管理対象PostgreSQL profileを使う場合は、bootstrapへ`--storage-profile postgres`を追加し、起動時にもoverlayを追加します。

```bash
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml -f deploy/compose.edge-postgres.yaml \
  up --build --detach
```

profileは導入directoryの`storage-profile.json`へ固定され、起動flagと異なる場合は停止します。接続失敗時にSQLiteへfallbackせず、両DBへの二重書込みもしません。SQLiteからのoffline移行は[IoTKit Edgeの導入と復旧](docs/okf/ja/operations/installation-and-recovery.md)を参照してください。

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

起動後は上記のcommissioning smoke commandを実行してください。bootstrapを再実行しても既存の出力directoryは置換しません。診断、password recovery、certificate renewal、rollbackは[IoTKit Edgeの導入と復旧](docs/okf/ja/operations/installation-and-recovery.md)を参照してください。

### IoTKit Edgeの意味付けとapplication出力

Consoleの**信号**画面で、補正、threshold/hysteresis、boolean state、累積値count、alarm behaviorを設定します。5分間のlive previewはpreview開始後に受け取った観測だけを使い、mappingやoutput eventを書き込みません。保存すると将来分だけに適用する新しいrevisionが始まり、過去のraw recordを黙って再計算しません。

**出力**画面で、汎用semantic定義をYokaKitの`source-id`、`signal-id`、用途（`production`、`onoff`、`gantt_chart`、`alarm`）へbindします。IoTKitの累積値がYokaKitの`kind=production`になるのは、このAdapter内だけです。出力はQoS 1で、Broker PUBACKまでIoTKit Edge outboxに残ります。YokaKit statusはretained source statusとして別にpublishします。

Edge NodeとIoTKit Edgeを結ぶ内部Brokerと、外部application Brokerは別にできます。それぞれのendpoint、trust bundle、client ID、credentialをdeployment設定として配置し、external profileを`serve`の`--output-*` flagで渡します。Consoleは状態を表示しますが、Broker credentialは変更できません。

## Buildとテスト

[`rust-toolchain.toml`](rust-toolchain.toml)で固定したtoolchain（Rust 1.95.0）が必要です。`rustup`が自動的に導入します。

```bash
cargo build --workspace
cargo test  --workspace      # 約530テスト。hardware専用の2テストは#[ignore]
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check

# Docker Mosquittoによる外部Output Adapter、PUBACK、再接続ゲート
scripts/test-edge-output.sh
IOTKIT_TEST_STORAGE_PROFILE=postgres scripts/test-edge-output.sh

# SQLite/PostgreSQL共通契約と短時間capacity回帰smoke
scripts/test-edge-postgres.sh
scripts/test-edge-capacity.sh

# OpenAPI生成Console型、TypeScript、埋め込みJavaScriptの同期
npm ci --prefix iotkit-edge/frontend
scripts/test-edge-console-frontend.sh

# Chromiumによるlogin、Edge Node登録、センサー設定、意味付け、外部出力、権限導線
scripts/test-edge-console-e2e.sh
IOTKIT_TEST_STORAGE_PROFILE=postgres scripts/test-edge-console-e2e.sh

# 隣接するYokaKit checkoutとのconsumer contractゲート
scripts/test-yokakit-consumer-contract.sh

# v1 host統合ゲート。新しいreport directoryを指定する
scripts/test-edge-host-release-gate.sh /secure/report/iotkit-v1-YYYYMMDD
```

IoTKit ConsoleはGoのserver-side renderingを維持し、browser動作だけを`iotkit-edge/frontend/src/`のTypeScriptで実装します。JSON API型は`iotkit-edge/openapi/edge-console-v1.yaml`から生成します。配布物にはesbuild済みの`static/console.js`を埋め込むため、IoTKit Edgeの実行環境にNode.jsは不要です。

CIは各PRでcrate layer rule、Rust/Go unit test、生成済みConsole asset、埋め込みbrowser journeyを検査します（[`.github/workflows/ci.yml`](.github/workflows/ci.yml)）。Docker、PostgreSQL、Broker障害を含む統合検証は、release前に`test-edge-host-release-gate.sh`を一度実行します。`scripts/verify.sh`はfmt、layer rule、test、clippyをlocalで実行します。

## Repository構成

| Path | 役割 |
|------|------|
| `core/*` | domain。storage、ledger（device identity）、timeseries、registry、collector（ingest）、publish（outbox）、ops（R14 typed operationとauth）、types、engine（supervision）を1 crate 1責務で分離 |
| `iotkit-ingest-contract` / `iotkit-ingest-client` | Envelope/Ackの取り込みwire contractとAdapterが使うclient |
| `*-adapter*` / `iotkit-sensor-drivers` / `rpi4b-transport` | BravePI Mainboardとrpi-localのsensor Adapter、共有sensor IC driverとpolling runtime、raw bus transport |
| `iotkit-edge-node` / `iotkit-edge-nodectl` | IoTKit Edge Node daemonとoperator CLI |
| `iotkit-edge` | IoTKit Edge MQTT consumer、耐久raw/semantic store、cursor manager、application exporter、認証付きSSR Console、query/configuration CLI |

crate全体図、layer rule、新しいcodeの配置表は[Architecture](docs/okf/ja/architecture/system-overview.md)にあります。

Codex Cloudを含め、単一cloneから開発を再開できます。再開順序とcontext authorityは[docs/cloud-development.md](docs/cloud-development.md)を参照してください。

## Architectureと契約

- [ドキュメント入口](docs/README.md) — 読む順序と正本の優先関係。
- [製品モデル](docs/okf/ja/concepts/product-model.md) — IoTKitの所有範囲、component境界、外部applicationに残すもの。
- [Architecture](docs/okf/ja/architecture/system-overview.md) — crate map、配置rule、data flow、custody、concurrency。
- [契約](docs/okf/ja/index.md#contracts) — device ingest、Input Adapter、Edge Node保管責任、Output Adapterの境界。

過去のredesign決定と完了済みimplementation planは、理由と追跡可能性のためrepositoryに残します。ただし、現行の実行可能契約やdocumentation indexを上書きしません。

## ロードマップ

- **Wave 0 —「自分たちの環境で動く」:** ingest、registry、ledger、retention、snapshot/restore、operator CLI。**完了。**
- **初期実装gate:** paired BravePI temperature sensor → BLE Long Range → BravePI Mainboard → UART → IoTKit Edge Node → standard MQTT Broker → IoTKit Edge → raw SQLite → direct CLI query。実機経路、再起動・停止matrix、storage failure injection、bounded capacity、application `accepted-through`を検証済みです。purge eligibilityは検証済み`accepted-through`の後だけ進みます。**完了。**
- **IoTKit Edgeのsemantic/output slice:** 汎用numeric/boolean/cumulative/alarm、live preview、no backfill、耐久Output Adapter境界、合意済みYokaKit source/signal observation contract。**実装済み。**
- **Wave 1 —「他者へ配布できる」:** onboarding、calibration、configuration authority、その他の配布品質向上。既存HTTP ingressとcontrol-plane実装は残しますが、現在の完了条件ではありません。
- **Wave 2 —「公開OSS」:** client library、A/B update、OS image。

## License

[Apache License, Version 2.0](LICENSE)で提供します。
