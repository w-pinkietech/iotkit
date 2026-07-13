# IoTKit Edge / Site Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 公開前の旧`Gateway`名称を互換層なしで廃止し、Rust側を`IoTKit Edge`、Go側を`IoTKit Site`、MQTT v1 identityを`edge_node_id`へ一斉に切り替える。

**Architecture:** Edgeはセンサー信号をcanonical recordへ変換し、SQLite outboxへ耐久保存してMQTTで配送する。Siteはraw recordとEdge Nodeごとのcursorを同一transactionで保存し、保管責任を引き受けた後だけ`accepted-through`を返す。この責務とcustody semanticsは変えず、製品名、公開識別子、設定、DB schema、実行単位を新語へ揃える。

**Tech Stack:** Rust 2024 / Tokio / rusqlite / rumqttc、Go 1.25 / modernc SQLite / Paho MQTT、Mosquitto 2、Docker Compose、Raspberry Pi Debian 13 arm64、BravePI Mainboard UART。

## Global Constraints

- 承認済み設計は`docs/superpowers/specs/2026-07-13-iotkit-edge-site-naming-design.md`。判断が衝突したら実装を止め、設計を勝手に拡張しない。
- `BravePI Mainboard`と`BravePI Transmitter`は製品名なので変更しない。
- `ledger_epoch`、`cursor_start`、`cursor_end`、`accepted-through`、`accepted_through`の名称と意味は変更しない。
- `schema_version: 1`とMQTT `/v1/`を維持する。旧pre-release v1とのalias、dual subscribe、DB migration、旧binary aliasは作らない。
- Edge DB、Site DBとも旧schemaを自動削除しない。旧`gateway_identity` schemaは、再作成を促す明示的なエラーで起動を拒否する。
- `time_source = gateway` / `gateway_adjusted`は現行構成要素を指す公開語なので、互換aliasなしで`edge` / `edge_adjusted`へ変更する。時刻選択ロジック自体は変えない。
- 現行設定の`[gateway]`、`gateway_name`、Rustの`Gateway*`型も`[edge]`、`edge_name`、`Edge*`へ変更する。旧設定は`deny_unknown_fields`で受理しない。
- 作業中は各taskのfocused testだけを実行する。workspace全体、Go全package、Docker縦切りはTask 9まで実行しない。
- commitはtaskごとの意味単位で作る。push、PR作成、mergeはこのplanの実行権限に含めない。
- 秘密値を標準出力、ログ、fixture、Gitへ入れない。Pi上ではpassword fileの中身を表示しない。

---

## Task 1: 現行の設計正本をEdge / Siteへ切り替える

**Files:**

- Modify: `docs/redesign/terminology.md`
- Modify: `docs/redesign/responsibility-ledger.md`
- Modify: `docs/redesign/decisions/D2-data-authority-topology-operations.md`
- Modify: `docs/redesign/decisions/D3-process-and-wave-decisions.md`
- Modify: `docs/redesign/decisions/D7-exit-contract.md`
- Rename: `docs/redesign/decisions/D8-site-topology-multi-gateway.md` -> `docs/redesign/decisions/D8-site-topology-multi-edge.md`
- Modify: `docs/redesign/decisions/D9-exit-mqtt-binding.md`
- Modify: `docs/redesign/decisions/D13-ui-scope.md`
- Modify: `docs/exit-contract.md`

- [ ] **Step 1: 用語集へ正式な製品・役割名を反映する**

`IoTKit`、`IoTKit Edge`、`Edge Node`、`IoTKit Site`、`MQTT Broker`を定義し、`Node`単独を避ける。一般カテゴリとしての`IoT gateway`と、IoTKitの構成要素名を区別する。

- [ ] **Step 2: 責務台帳と決定文書を更新する**

Edgeは収集・正規化・耐久buffer・再送、Siteは複数Edgeの集約・raw保存・cursor・query・application export境界を持つ、と記載する。Siteが`production`の業務意味を所有するようには書かない。

- [ ] **Step 3: D8をrenameし、現行文書からのlinkを更新する**

Run:

```bash
rg -n 'D8-site-topology-multi-gateway|multi-Gateway|Site Server|Gateway' \
  docs/redesign docs/exit-contract.md \
  -g '!docs/redesign/reviews/**' -g '!docs/redesign/inputs/**'
```

Expected: 歴史説明や一般カテゴリ以外の現行参照が新名称へ置き換わっている。

- [ ] **Step 4: 文書差分を検査する**

Run:

```bash
git diff --check
rg -n 'BravePI (Mainboard|Transmitter)' docs/redesign docs/exit-contract.md
```

Expected: whitespace errorなし。BravePI製品名が維持される。

- [ ] **Step 5: commitする**

```bash
git add docs/redesign docs/exit-contract.md
git commit -m "docs: define IoTKit Edge and Site terminology"
```

---

## Task 2: MQTT v1 wire contractと共有fixtureを破壊的にrenameする

**Files:**

- Modify: `testdata/egress/v1/record-batch.json`
- Modify: `testdata/egress/v1/accepted-through.json`
- Modify: `core/publish/src/wire.rs`
- Modify: `core/publish/tests/egress_v1_fixtures.rs`
- Modify: `iotkit-site-server/internal/contract/contract.go`
- Modify: `iotkit-site-server/internal/contract/contract_test.go`

- [ ] **Step 1: fixtureとcontract testを新wireへ先に変更する**

期待するbatch/ackの共通部分は次とする。

```json
{
  "schema_version": 1,
  "edge_node_id": "edge-node-01",
  "ledger_epoch": "epoch-01",
  "publication_id": "edge-node-01:epoch-01:1:1"
}
```

Rust testは`RecordBatch.edge_node_id`を検査し、Go testは`RecordBatch.EdgeNodeID`を検査する。両言語へ旧`gateway_identity`だけを持つpayloadがdecode/validationに失敗するtestも追加する。

- [ ] **Step 2: testが旧型名で失敗することを確認する**

Run:

```bash
cargo test -p iotkit-core-publish --test egress_v1_fixtures
(cd iotkit-site-server && go test ./internal/contract)
```

Expected: `edge_node_id` / `EdgeNodeID`未実装によりFAIL。

- [ ] **Step 3: Rust wire型とvalidationをrenameする**

`RecordBatch`と`AcceptedThrough`は次のfieldを持つ。

```rust
pub edge_node_id: String,
```

`publication_id(edge_node_id, ledger_epoch, cursor_start, cursor_end)`は同じcorrelation形式を新identityで生成する。validation errorも`edge_node_id`を名指しする。

- [ ] **Step 4: Go wire型とvalidationをrenameする**

```go
EdgeNodeID string `json:"edge_node_id"`
```

`PublicationID`、`Validate`、`ValidateFor`の引数、比較、errorを新語へ揃える。旧field alias用のcustom unmarshalerは作らない。

- [ ] **Step 5: focused testを通す**

Run:

```bash
cargo test -p iotkit-core-publish --test egress_v1_fixtures
(cd iotkit-site-server && go test ./internal/contract)
```

Expected: PASS。旧fixture testは明示的にrejectされる。

- [ ] **Step 6: commitする**

```bash
git add core/publish testdata/egress iotkit-site-server/internal/contract
git commit -m "refactor: rename MQTT v1 Edge identity"
```

---

## Task 3: Edge DB identityとMQTT publisherを`edge_node_id`へ切り替える

**Files:**

- Modify: `core/ledger/src/store.rs`
- Modify: `core/ledger/src/lib.rs`
- Modify: `iotkit-gateway/src/main.rs`
- Modify: `iotkit-gateway/src/mqtt_publish_task.rs`

- [ ] **Step 1: Edge identityの生成・安定性・旧DB拒否testを書く**

`core/ledger/src/store.rs`へ次を追加・変更する。

```rust
#[test]
fn edge_node_id_is_generated_once_and_stable() { /* UUIDv7 and equality */ }

#[test]
fn legacy_gateway_identity_is_rejected() {
    // ledger_metaへgateway_identityだけを入れる
    // edge_node_id()はUnsupportedPreReleaseSchema相当の明示エラーを返す
}
```

- [ ] **Step 2: focused testが失敗することを確認する**

Run:

```bash
cargo test -p iotkit-core-ledger edge_node_id
cargo test -p iotkit-core-ledger legacy_gateway_identity_is_rejected
```

Expected: 新関数または新error variant未実装によりFAIL。

- [ ] **Step 3: `edge_node_id()`を実装し、起動時に常に検査する**

`ledger_meta.key = 'edge_node_id'`を初回UUIDv7生成・通常再起動で再利用する。旧`gateway_identity` keyが存在する場合は新IDを生成せず、`unsupported pre-release Gateway database; recreate the Edge database`相当のerrorを返す。

`iotkit-gateway/src/main.rs`はDB migration直後、MQTT有効/無効に関係なく`edge_node_id()`を一度呼ぶ。これにより旧DBがMQTT無効時だけ起動できる抜け道を作らない。

- [ ] **Step 4: MQTT publisherを新identity/topicへ変更する**

```text
iotkit/v1/edge-nodes/{edge_node_id}/records
iotkit/v1/edge-nodes/{edge_node_id}/accepted-through
client id: iotkit-edge-{edge_node_id}
username:  {edge_node_id}
```

`prepare_batch`、ack validation、test seedも`edge_node_id`へ揃える。PUBACKではなくvalidated `accepted-through`だけがcursorを進める既存testは保持する。

- [ ] **Step 5: focused testを通す**

Run:

```bash
cargo test -p iotkit-core-ledger edge_node_id
cargo test -p iotkit-core-ledger legacy_gateway_identity_is_rejected
cargo test -p iotkit-gateway mqtt_publish_task
```

Expected: PASS。

- [ ] **Step 6: commitする**

```bash
git add core/ledger iotkit-gateway/src/main.rs iotkit-gateway/src/mqtt_publish_task.rs
git commit -m "refactor: identify and publish from Edge Nodes"
```

---

## Task 4: Rustの設定、API、時刻由来語彙をEdgeへ揃える

**Files:**

- Modify: `iotkit-ingest-contract/src/envelope.rs`
- Modify: `iotkit-ingest-contract/src/validation.rs`
- Modify: `iotkit-ingest-client/**`
- Modify: `iotkit-ingest-http/**`
- Modify: `core/collector/**`
- Modify: `core/timeseries/**`
- Modify: `core/registry/**`
- Modify: `bravepi-mainboard-adapter/**`
- Modify: `rpi-local-adapter/**`
- Modify: `iotkit-polling-adapter-runtime/**`
- Modify: `iotkit-gateway/src/config.rs`
- Modify: `iotkit-gateway/src/api/routes.rs`
- Modify: `iotkit-gateway/src/api/tls.rs`
- Modify: `iotkit-gateway/src/main.rs`
- Modify: `iotkit-gateway/src/health.rs`
- Modify: `iotkit-gateway/src/publish_task.rs`
- Modify: `iotkit-gateway/src/retention.rs`
- Modify: `iotkit-gateway/tests/**`
- Modify: `iotkit-gatewayctl/src/**`
- Modify: `iotkit-gatewayctl/tests/cli.rs`
- Modify: `core/ops/tests/fingerprint.rs`

- [ ] **Step 1: 新しい公開語彙をtestへ先に反映する**

- TOML rootは`[edge]`。
- resolved typeは`EdgeConfig`、raw typeは`RawEdgeConfig`。
- API config/responseは`edge_name`。
- TLS CN/SAN defaultは`iotkit-edge` / `iotkit-edge.local`。
- `TimeSource::Edge`と`TimeSource::EdgeAdjusted`はwire上`edge` / `edge_adjusted`。
- 旧`[gateway]`、`gateway_name`、`time_source: "gateway"`が受理されないtestを残す。

- [ ] **Step 2: contract/config testが失敗することを確認する**

Run:

```bash
cargo test -p iotkit-ingest-contract
cargo test -p iotkit-gateway config
cargo test -p iotkit-gateway api::tls
```

Expected: 新enum、field、hostname未実装によりFAIL。

- [ ] **Step 3: time-source語彙を機械的にrenameし、意味を維持する**

`Gateway -> Edge`、`GatewayAdjusted -> EdgeAdjusted`、DB文字列とJSON文字列を`gateway -> edge`、`gateway_adjusted -> edge_adjusted`へ変更する。`age_ms`からevent timeを再構成する順序、freshness validation、fallbackは変更しない。

- [ ] **Step 4: config/API/TLSとproduct-facing messageをrenameする**

`RawConfig.edge`、`EdgeConfig`、`ApiConfig.edge_name`へ変更する。旧fieldのserde aliasは付けない。help、log、error、test fixtureに残るIoTKit構成要素としての`gateway`もEdgeへ変更する。

- [ ] **Step 5: focused testを通す**

Run:

```bash
cargo test -p iotkit-ingest-contract
cargo test -p iotkit-core-collector
cargo test -p bravepi-mainboard-adapter
cargo test -p iotkit-gateway config
cargo test -p iotkit-gateway api::tls
cargo test -p iotkit-core-ops fingerprint
```

Expected: PASS。

- [ ] **Step 6: commitする**

```bash
git add iotkit-ingest-contract iotkit-ingest-client iotkit-ingest-http \
  core bravepi-mainboard-adapter rpi-local-adapter \
  iotkit-polling-adapter-runtime iotkit-gateway iotkit-gatewayctl
git commit -m "refactor: adopt Edge terminology in Rust contracts"
```

---

## Task 5: Rust crate、binary、CLIを物理renameする

**Files:**

- Rename: `iotkit-gateway/` -> `iotkit-edge/`
- Rename: `iotkit-gatewayctl/` -> `iotkit-edgectl/`
- Modify: `iotkit-edge/Cargo.toml`
- Modify: `iotkit-edgectl/Cargo.toml`
- Modify: `iotkit-edgectl/src/main.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `scripts/check-layers`
- Modify: `scripts/layer-fixtures/ingress-control-api/Cargo.toml`
- Modify: `.github/workflows/ci.yml`
- Modify: all Rust imports referring to `iotkit_gateway`

- [ ] **Step 1: directoriesをGit renameする**

```bash
git mv iotkit-gateway iotkit-edge
git mv iotkit-gatewayctl iotkit-edgectl
```

- [ ] **Step 2: package/lib/bin名を変更する**

```toml
[package]
name = "iotkit-edge"

[lib]
name = "iotkit_edge"

[[bin]]
name = "iotkit-edge"
```

CLI package/binaryは`iotkit-edgectl`、Clap commandは`edgectl`ではなくbinaryと一致する`iotkit-edgectl`とする。workspace member、path dependency、`iotkit_gateway` importをすべて更新する。

- [ ] **Step 3: layer checkerの分類を更新する**

`BINARIES`は`iotkit-edge` / `iotkit-edgectl`へ変更する。`SUPERVISION_DEPENDENTS`は旧daemon名`iotkit-gateway`だけを`iotkit-edge`へ変更し、依存していないCLIを追加しない。rule 8 negative fixtureは新Edge packageへの禁止依存を検査する。

- [ ] **Step 4: metadataと変更対象binaryの全target compileを確認する**

Run:

```bash
cargo metadata --no-deps --format-version 1 >/tmp/iotkit-edge-metadata.json
scripts/check-layers
cargo check -p iotkit-edge -p iotkit-edgectl --all-targets
```

Expected: package名衝突なし、layer checker PASS、変更対象の旧crate importなし。workspace全体testはTask 9まで実行しない。

- [ ] **Step 5: binary名を確認する**

Run:

```bash
cargo run -p iotkit-edgectl -- --help
cargo build -p iotkit-edge --bin iotkit-edge
test -x target/debug/iotkit-edge
test -x target/debug/iotkit-edgectl
```

Expected: help先頭が`iotkit-edgectl`、新binaryのみ生成対象になる。

- [ ] **Step 6: commitする**

```bash
git add Cargo.toml Cargo.lock .github scripts iotkit-edge iotkit-edgectl
git commit -m "refactor: rename Edge crates and binaries"
```

---

## Task 6: Go Site serviceとSite DBをrenameする

**Files:**

- Rename: `iotkit-site-server/` -> `iotkit-site/`
- Rename: `iotkit-site-server/cmd/iotkit-site-server/` -> `iotkit-site/cmd/iotkit-site/`
- Modify: `iotkit-site/go.mod`
- Modify: `iotkit-site/cmd/iotkit-site/main.go`
- Modify: `iotkit-site/cmd/iotkit-site/main_test.go`
- Modify: `iotkit-site/internal/contract/**`
- Modify: `iotkit-site/internal/mqttsite/**`
- Modify: `iotkit-site/internal/store/**`
- Modify: `iotkit-site/Dockerfile`

- [ ] **Step 1: Site DBとMQTT processorの新契約testを書く**

- `raw_records.edge_node_id`と`accepted_cursors.edge_node_id`を主キーの一部にする。
- query JSONは`edge_node_id`を返す。
- topic parserは`iotkit/v1/edge-nodes/{id}/records`だけを受理する。
- topic/bodyの`edge_node_id`不一致では保存もack publishもしない。
- `gateway_identity`列を持つ旧tableを用意し、`Open()`が`unsupported pre-release Site database; recreate it`相当で失敗するtestを追加する。

- [ ] **Step 2: focused testが失敗することを確認する**

Run:

```bash
(cd iotkit-site-server && go test ./internal/store ./internal/mqttsite)
```

Expected: 新列/topic/legacy schema guard未実装によりFAIL。

- [ ] **Step 3: Site storeを新schemaへ変更する**

`Store.initialize()`はtable作成前に既存`raw_records` / `accepted_cursors`のcolumnを調べる。`gateway_identity`があれば自動ALTERや新table作成をせず、明示errorを返す。新規DBでは全SQL、`RawRecord`、`readCursor`を`EdgeNodeID` / `edge_node_id`へ揃える。

- [ ] **Step 4: Site MQTT consumer/processorを新topicへ変更する**

```go
const recordsTopicFilter = "iotkit/v1/edge-nodes/+/records"
```

ack先は`iotkit/v1/edge-nodes/{edgeNodeID}/accepted-through`。logは`IoTKit Site subscribed`とし、旧Site Server名を残さない。

- [ ] **Step 5: directory、module、binary、Docker entrypointをrenameする**

```bash
git mv iotkit-site-server iotkit-site
git mv iotkit-site/cmd/iotkit-site-server iotkit-site/cmd/iotkit-site
```

Go moduleは`github.com/w-pinkietech/iotkit-next/iotkit-site`、binaryとDocker entrypointは`iotkit-site`へ変更する。

- [ ] **Step 6: focused testを通す**

Run:

```bash
(cd iotkit-site && go test ./internal/contract ./internal/store ./internal/mqttsite)
(cd iotkit-site && go test ./cmd/iotkit-site)
(cd iotkit-site && go build ./cmd/iotkit-site)
```

Expected: PASS。旧Site DB testだけは意図どおり起動拒否を確認する。

- [ ] **Step 7: commitする**

```bash
git add -A -- iotkit-site-server iotkit-site
git commit -m "refactor: rename and harden IoTKit Site"
```

---

## Task 7: MQTT ACL、Compose、縦切りscriptを新名称へ切り替える

**Files:**

- Modify: `deploy/mosquitto/dev.acl`
- Modify: `compose.dev.yaml`
- Modify: `scripts/test-site-mqtt.sh`

- [ ] **Step 1: development ACLを新topicへ変更する**

```text
user edge-node-01
topic write iotkit/v1/edge-nodes/edge-node-01/records
topic read  iotkit/v1/edge-nodes/edge-node-01/accepted-through

user site
topic read  iotkit/v1/edge-nodes/+/records
topic write iotkit/v1/edge-nodes/+/accepted-through
```

- [ ] **Step 2: Compose build contextとentrypointをSiteへ変更する**

build contextは`./iotkit-site`。queryは`iotkit-site query`。compose service名`site`はすでに役割名なので維持する。

- [ ] **Step 3: 縦切りscriptをEdge語彙へ変更する**

`gateway_pid`、`gateway.db`、`gateway.toml`、`gateway-password`、`[gateway]`、旧binary、旧SQL key、旧JSON fieldをそれぞれEdge名へ変更する。実際の`edge_node_id`をDBから読み、ACL username/topicへ束縛する。

- [ ] **Step 4: full Docker testをまだ実行せず、静的検査だけ行う**

Run:

```bash
bash -n scripts/test-site-mqtt.sh
IOTKIT_MOSQUITTO_PASSWORD_FILE=/tmp/placeholder \
IOTKIT_SITE_PASSWORD_FILE=/tmp/placeholder \
IOTKIT_SITE_DATA_DIR=/tmp \
docker compose -f compose.dev.yaml config --quiet
```

Expected: shell syntaxとCompose解決がPASS。`scripts/test-site-mqtt.sh`自体はTask 9まで実行しない。

- [ ] **Step 5: commitする**

```bash
git add deploy/mosquitto/dev.acl compose.dev.yaml scripts/test-site-mqtt.sh
git commit -m "chore: switch development MQTT path to Edge Nodes"
```

---

## Task 8: README、architecture、運用規則、現行図を新名称へ統一する

**Files:**

- Modify: `README.md`
- Modify: `AGENTS.md`
- Modify: `CLAUDE.md`
- Modify: `docs/architecture.md`
- Modify: `docs/ingest-contract.md`
- Modify: `docs/cloud-development.md`
- Modify: current files under `docs/redesign/decisions/`
- Modify: current diagrams under `docs/redesign/diagrams/`
- Modify: product-facing comments/messages found under `core/`, adapters, `scripts/`, `.github/`
- Preserve: `docs/redesign/reviews/**`
- Preserve: `docs/redesign/inputs/**`
- Preserve: `docs/redesign/adr-inventory.md`
- Preserve: `rewrite-prep.md`
- Preserve: the approved old/new mapping in `docs/superpowers/specs/2026-07-13-iotkit-edge-site-naming-design.md`

- [ ] **Step 1: current documentationを更新する**

install/run例を`iotkit-edge` / `iotkit-edgectl` / `iotkit-site`へ変更する。data flowは`BravePI Mainboard -> UART -> IoTKit Edge -> MQTT Broker -> IoTKit Site`。Siteをstorage-onlyと説明しない。

- [ ] **Step 2: AGENTS/CLAUDEのcrate mapと検証規則を更新する**

既存のSSH権限注意、秘密情報、custody、不変条件、verification economyは削らず、構成要素名とpathだけを更新する。

- [ ] **Step 3: 現行図とlinkを更新する**

D8 rename後のlink、HTML内の表示名、crate pathを更新する。過去レビューの本文は変更しない。

- [ ] **Step 4: exact legacy identifier scanを行う**

Run:

```bash
rg -n -i \
  'iotkit-gateway|iotkit_gateway|gatewayctl|gateway_identity|GatewayIdentity|gateway_name|/gateways/' . \
  -g '!target/**' \
  -g '!docs/redesign/reviews/**' \
  -g '!docs/redesign/inputs/**' \
  -g '!docs/redesign/adr-inventory.md' \
  -g '!rewrite-prep.md' \
  -g '!docs/superpowers/specs/2026-07-13-iotkit-edge-site-naming-design.md' \
  -g '!docs/superpowers/plans/2026-07-13-iotkit-edge-site-rename.md'
```

Expected: no matches。

- [ ] **Step 5: generic Gateway scanを手動分類する**

Run:

```bash
rg -n '\bGateway\b|\bgateway\b' README.md AGENTS.md CLAUDE.md docs core \
  bravepi-mainboard-adapter rpi-local-adapter iotkit-* scripts .github \
  -g '!docs/redesign/reviews/**' \
  -g '!docs/redesign/inputs/**' \
  -g '!docs/redesign/adr-inventory.md' \
  -g '!docs/superpowers/**'
```

Expected: 一般カテゴリ`IoT gateway`だけが許容。TimeSource、型、変数、構成要素名、test名、errorに旧語を残さない。

- [ ] **Step 6: documentation checksを行う**

```bash
git diff --check
rg -n 'D8-site-topology-multi-gateway' README.md AGENTS.md CLAUDE.md docs \
  -g '!docs/redesign/reviews/**' -g '!docs/redesign/adr-inventory.md' \
  -g '!docs/superpowers/**'
```

Expected: no whitespace error、no stale D8 link。

- [ ] **Step 7: commitする**

```bash
git add README.md AGENTS.md CLAUDE.md docs core bravepi-mainboard-adapter \
  rpi-local-adapter iotkit-* scripts .github
git commit -m "docs: present IoTKit as Edge and Site"
```

---

## Task 9: PR前のローカル最終検証を一度まとめて実行する

**Files:**

- Verify only; failure修正時は該当fileとfocused testだけを変更する。

- [ ] **Step 1: worktreeと秘密情報混入を確認する**

```bash
git status --short
git diff --check HEAD
git diff --cached --check
```

Expected: 意図しないfile、credential、generated binaryなし。

- [ ] **Step 2: Rustのfull gateを一度実行する**

```bash
scripts/verify.sh
```

Expected: fmt、layer、workspace tests、Clippy `-D warnings`がPASS。

- [ ] **Step 3: Go全package testを一度実行する**

```bash
(cd iotkit-site && go test ./...)
```

Expected: PASS。

- [ ] **Step 4: Docker MQTT縦切りを一度実行する**

```bash
scripts/test-site-mqtt.sh
```

Expected: `Edge -> MQTT -> Site -> accepted-through vertical slice: OK`。broker再起動後も再subscribeし、validated ackでEdge cursorが進む。

- [ ] **Step 5: legacy residual scanを再実行する**

Task 8 Step 4/5のcommandを同じ除外条件で実行する。

Expected: exact legacy identifierなし。一般カテゴリ以外のgeneric old termなし。

- [ ] **Step 6: 失敗時の扱い**

失敗したらPRへ進まない。原因を切り分け、該当focused testで修正してcommitする。その後、成功証拠が失われた検査だけを再実行する。「一度」を守るために失敗を放置しない。

---

## Task 10: 実験用Piを新構成で再作成し、BravePI実機縦切りを確認する

**Remote paths:**

- SSH: `iotkit@iotkit` with `ssh -F /dev/null`
- Clean source copy: `/home/iotkit/iotkit/iotkit-next-current`
- Disposable lab: `/home/iotkit/iotkit-lab`
- Existing old DB/config/credentials in the disposable lab are intentionally deleted and recreated only in this task。

- [ ] **Step 1: 読み取り専用preflightを行う**

```bash
ssh -F /dev/null -o BatchMode=yes -o ConnectTimeout=5 iotkit@iotkit \
  'uname -m; readlink -f /dev/serial0; id; docker version --format "{{.Server.Version}}"; cargo --version'
```

Expected: `aarch64`、`/dev/ttyAMA0`、`iotkit`が`dialout`所属、Docker/Cargo利用可能。BravePI信号が停止中なら、削除前にユーザーへ送信再開を依頼する。

- [ ] **Step 2: commit済みsourceをPiのclean test copyへ同期する**

ローカルの`.git`、`target`、秘密fileを送らない。同期前にPi側pathが専用test copyであることを再確認する。

```bash
rsync -a --delete \
  --exclude .git --exclude target --exclude '*.db' --exclude '*.pem' --exclude '*password*' \
  -e 'ssh -F /dev/null' ./ \
  iotkit@iotkit:/home/iotkit/iotkit/iotkit-next-current/
```

- [ ] **Step 3: 旧実験環境を停止し、専用labだけを削除・再作成する**

まず旧Edge/Gateway processと`/home/iotkit/iotkit-lab`のCompose projectを停止する。対象pathを表示して確認後、承認済みの旧DB、旧TOML、旧MQTT credential、旧Site DBを削除する。repoや`$HOME`全体へ削除範囲を広げない。

```bash
ssh -F /dev/null iotkit@iotkit \
  'docker compose \
    -f /home/iotkit/iotkit/iotkit-next-current/compose.dev.yaml \
    -f /home/iotkit/iotkit-lab/compose.override.yaml \
    --env-file /home/iotkit/iotkit-lab/compose.env \
    down --volumes --remove-orphans || true'
ssh -F /dev/null iotkit@iotkit \
  'rm -rf /home/iotkit/iotkit-lab && install -d -m 700 /home/iotkit/iotkit-lab'
```

- [ ] **Step 4: Pi上でdebug Edge binaryをbuildする**

```bash
ssh -F /dev/null iotkit@iotkit \
  'cd /home/iotkit/iotkit/iotkit-next-current && cargo build -p iotkit-edge --bin iotkit-edge'
```

初回release buildは使わない。

- [ ] **Step 5: 新identityと資格情報を再生成する**

`[edge]`、BravePI `/dev/serial0`、Site MQTT exitを使う新TOMLを作る。秘密値は`openssl rand`で新規生成してmode 600のfileへ保存し、terminalへ表示しない。

最初はMQTTなしの短いEdge起動で新DBへ`edge_node_id`を生成し、停止後にSQLでIDだけを読む。そのIDをMosquitto usernameと次のACL topicへ束縛する。

```text
iotkit/v1/edge-nodes/{edge_node_id}/records
iotkit/v1/edge-nodes/{edge_node_id}/accepted-through
```

Site credentialは別username/passwordを使う。旧`gateway_identity` key、旧topic、旧credentialを再利用しない。

- [ ] **Step 6: BrokerとIoTKit SiteをDockerで起動する**

repoの`compose.dev.yaml`を使い、Pi専用のpassword/ACL/data pathをenvironmentで渡す。container logにpasswordが出ていないことを確認する。

```bash
docker compose -f /home/iotkit/iotkit/iotkit-next-current/compose.dev.yaml \
  --env-file /home/iotkit/iotkit-lab/compose.env up --build --detach
```

- [ ] **Step 7: BravePIを有効にしたIoTKit Edgeを起動する**

`/dev/serial0`を開くprocessが他にないことを確認し、Edgeを起動する。PIDとlogはlab配下へ保存する。MQTT password fileの中身はlogへ出さない。

- [ ] **Step 8: UARTからSiteとack cursorまで確認する**

新しい温度観測について次を確認する。

1. Edge `readings`へ`temperature_c`が入る。
2. `publication_log`へ連続`pub_seq`が入る。
3. Site queryが同じ`edge_node_id`、`ledger_epoch`、`pub_seq`を返す。
4. Edge `target_registry.cursor_pub_seq`がSite保存済みseqまで進む。
5. `PRAGMA quick_check`がEdge DBとSite DBで`ok`。

Site query:

```bash
docker compose -f /home/iotkit/iotkit/iotkit-next-current/compose.dev.yaml \
  --env-file /home/iotkit/iotkit-lab/compose.env exec -T site \
  iotkit-site query --db /data/site.db --limit 10
```

Expected: BravePI Mainboardからの温度recordが`edge_node_id`付きで表示され、ack後のEdge cursorが同じseq以上になる。

- [ ] **Step 9: 実機結果を正本文書へ記録する**

`docs/redesign/decisions/D3-process-and-wave-decisions.md`へ日付、commit、確認したsensor、cursor範囲、Edge/Site quick check、停止時のUART解放を追記する。秘密値、実password、private keyは書かない。

```bash
git add docs/redesign/decisions/D3-process-and-wave-decisions.md
git commit -m "docs: record Edge and Site hardware validation"
git diff --check HEAD~1
```

- [ ] **Step 10: PR準備完了を報告して停止する**

`git status --short --branch`、Task 9の検証結果、Pi smokeの証拠、commit一覧を報告する。push、PR作成、mergeはユーザーの別承認を待つ。
