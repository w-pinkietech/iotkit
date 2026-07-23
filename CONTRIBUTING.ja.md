# IoTKitへのコントリビュート

日本語 | [English](CONTRIBUTING.md)

IoTKitを、現場で使えて他の開発者にも保守できる基盤にするための協力を歓迎します。
このprojectでは、先回りした抽象化より現場の根拠、data所有権の明示、初見のmaintainerが
履歴を掘り返さず理解できる変更を重視します。

## 最初に読むもの

コードを変更する前に、次の順序で読んでください。

1. [製品モデル](docs/okf/ja/concepts/product-model.md) — IoTKitが所有するものと、
   device・外部applicationに残すもの。
2. [Architecture](docs/okf/ja/architecture/system-overview.md) — 実行component、crate map、
   code配置、依存rule。
3. 対象となる[現行契約](docs/okf/ja/index.md#contracts) — ingest、Input Adapter、
   Edge Node保管責任、Output Adapter。
4. [AGENTS.md](AGENTS.md) — 人間とcoding agentが共通で守る不変条件と検証lane。

`docs/okf/`が現行の人間向け製品知識の正本です。`docs/redesign/`と
`docs/superpowers/`は履歴であり、現行契約、実行可能fixture、testを上書きしません。

## 開発環境

対応する開発環境はLinuxです。現在のCIは次を使います。

- `rust-toolchain.toml`が自動選択するRust 1.95.0
- `iotkit-edge/go.mod`が指定するGo 1.25
- Console assetとtest用のNode.js 22、npm
- Raspberry Pi transport依存の`pkg-config`、`libudev-dev`

統合testにはDocker Compose、OpenSSL、`jq`、`curl`も必要です。通常の開発loopに
Raspberry Piや実センサーは必要ありません。

DebianまたはUbuntuでは、言語以外のpackageを次で導入できます。

```bash
sudo apt-get update
sudo apt-get install --yes pkg-config libudev-dev docker.io docker-compose-v2 \
  openssl jq curl
```

Rustは[rustup](https://rustup.rs/)、Go 1.25は公式配布またはversion manager、
Node.js 22は通常使うpackage managerで導入してください。credential、生成した証明書、
local DB、deployment出力directoryをcommitしてはいけません。

## 最初の60分

### 0〜10分: 地図を確認する

```bash
git clone git@github.com:w-pinkietech/iotkit-next.git
cd iotkit-next
node scripts/check-okf-docs.mjs
scripts/check-layers
scripts/check-source-layout
```

続いて、上で案内した製品モデルとArchitectureを読みます。実行経路を短く表すと
次のようになります。

```text
sensor / device
  -> Rust製IoTKit Edge Node
  -> MQTT Broker
  -> Go製IoTKit Edge
  -> Output Adapter
  -> 外部application
```

### 10〜30分: 各領域でfocused testを一つ動かす

```bash
# Rust製Edge Node・Adapter側
cargo test -p bravepi-mainboard-adapter

# Go製IoTKit Edge側
(cd iotkit-edge && go test ./internal/outputadapter)

# Browser動作と生成済みConsole型
npm ci --prefix iotkit-edge/frontend
npm run check --prefix iotkit-edge/frontend
```

### 30〜45分: 実機なしで製品経路を動かす

次のscriptは使い捨て環境と疑似recordを使います。

```bash
# clean bootstrap、TLS、login、Broker ACL、Edge起動
scripts/test-edge-bootstrap.sh

# semantic Output Adapter、MQTT PUBACK、通信断、再起動後の収束
scripts/test-edge-output.sh
```

どちらもDockerへの接続が必要です。production DB、credential、証明書、deployment
directoryを流用してはいけません。

### 45〜60分: 小さな変更を辿る

変更したい領域に近い既存testを一つ選び、そのcall pathから製品codeへ入ります。
最小変更を行い、同じfocused testを再実行します。repository全体を闇雲に検索する前に、
次の表を使ってください。

## 変更目的から場所を探す

| 目的 | 最初に見る場所 | Focused verification |
|---|---|---|
| Protocol非依存domain動作の変更 | `core/*` | `cargo test -p <owning-crate>` |
| Sensor IC変換の追加・変更 | `iotkit-sensor-drivers/` | `cargo test -p iotkit-sensor-drivers` |
| BravePI UART decode・mappingの変更 | `bravepi-mainboard-adapter/` | `cargo test -p bravepi-mainboard-adapter` |
| 異なるdevice familyの追加 | top-level `*-adapter` crateとInput Adapter契約 | Adapter conformance testと`scripts/check-layers` |
| Edge Node composition・CLIの変更 | `iotkit-edge-node/`、`iotkit-edge-nodectl/` | 所有packageのtest |
| Raw受理、意味付け、account、backup、出力の変更 | `iotkit-edge/internal/` | `(cd iotkit-edge && go test ./internal/<package>)` |
| Consoleのbrowser動作変更 | `iotkit-edge/frontend/src/` | `npm run check --prefix iotkit-edge/frontend` |
| Console HTML・navigation変更 | `iotkit-edge/internal/edgehttp/` | Go edgehttp testと`scripts/test-edge-console-e2e.sh` |
| Browser JSON API変更 | `iotkit-edge/openapi/edge-console-v1.yaml`から開始 | 型生成後、frontendとedgehttp test |
| 公開wire contract変更 | 日英契約、exported type/schema、共有fixture、conformance test | 完全なcontract gate |
| 導入・復旧変更 | `scripts/`、`deploy/`、日英operations文書 | 対応するDocker/PostgreSQL/security script |

完全なcrate mapと配置ruleはArchitectureにあります。新しいcrateは、同文書と
`scripts/check-layers`へ分類するまで追加しません。

## 1 issue、1 worktree、1 pull request

すべての開発taskで次のloopを使います。

1. 成果と対象外が明確なGitHub issueを一つ作るか選ぶ。
2. local `master`を更新し、`agent/issue-<number>-<slug>`を作る。
3. `.worktrees/issue-<number>-<slug>`を作り、その中だけで作業する。
4. 製品動作を変更する前に、最も近いfocused testを追加または更新する。
5. 差分をissueの範囲に収める。範囲が実質的に変わったら別issueにする。
6. commitしてbranchをpushし、issueをcloseするdraft pull requestを作る。
7. そこで停止して人間へreviewを依頼する。自分でmergeしない。
8. Review指摘は同じbranchとpull requestで修正する。

例:

```bash
git switch master
git pull --ff-only --prune
git worktree add .worktrees/issue-123-example \
  -b agent/issue-123-example origin/master
cd .worktrees/issue-123-example
```

GitHubではmerge済みbranchを自動削除して構いません。merge後はmain checkoutへ戻り、
local参照を掃除して対応するworktreeとbranchを削除します。

```bash
git worktree remove .worktrees/issue-123-example
git branch -d agent/issue-123-example
git pull --prune
```

## 検証の使い分け

まず変更を否定できる最小commandを実行し、riskが大きい場合だけ広げます。

```bash
# 文書構造
node scripts/check-okf-docs.mjs

# Rust format・依存rule・source/test配置・test・Clippyと全Go test
scripts/verify.sh

# Console schema・生成型/asset・unit test
scripts/test-edge-console-frontend.sh

# Chromium上のConsole operator journey
scripts/test-edge-console-e2e.sh

# MQTT出力とPostgreSQL variant
scripts/test-edge-output.sh
IOTKIT_TEST_STORAGE_PROFILE=postgres scripts/test-edge-output.sh
scripts/test-edge-postgres.sh
```

Rust製品動作や影響範囲が不明なcross-component変更では`scripts/verify.sh`を使います。
文書だけの変更は通常、文書checker、link・command確認、`git diff --check`に絞ります。
`scripts/test-edge-host-release-gate.sh`はPRごとではなく、release candidateで一度実行します。

## 生成fileとcontract変更

- `iotkit-edge/openapi/edge-console-v1.yaml`を編集し、
  `npm run generate:api --prefix iotkit-edge/frontend`を実行する。
- 埋め込みConsole JavaScriptは
  `npm run build --prefix iotkit-edge/frontend`で生成する。
- `Cargo.lock`、`go.sum`、`package-lock.json`は各package managerからだけ更新する。
- `docs/okf/`の日英fileは同時に変更する。
- `testdata/`の共有JSONは正規contract dataとして扱う。一実装を通すためだけに
  fixtureを変更しない。

## 絶対に破らない境界

- Token、credential、鍵、そのhashをlog、error、audit、fixture、PRへ出さない。
- 文書上のdurability pointより前にackせず、未ack originalを黙って削除しない。
- 状態変更は所有componentのtyped operation dispatcherを通す。HTTP、UI、CLI、
  AdapterからSQLへ直接writeする経路を追加しない。
- Rust製品test本体は製品`src/`外、Go testは`*_test.go`、frontend unit testは
  `iotkit-edge/frontend/tests/unit/`へ置く。
- Legacy planや旧codeを新しい動作の正本にしない。

## Pull request checklist

- PRが一つのissueをlinkし、closeする。
- Descriptionに変更内容、理由、影響、検証を書く。
- 公開動作に実行可能testまたはfixtureがある。
- Contract変更では全表現を一緒に更新する。
- Operator・contributor workflow変更では文書を更新する。
- 無関係なrefactor、secret、local DB、生成証明書、deployment artifactを含めない。
- Branchは人間がreviewできる状態だが、mergeされていない。
