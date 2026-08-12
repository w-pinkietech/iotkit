# IoTKitへのコントリビュート

日本語 | [English](CONTRIBUTING.md)

IoTKitを、現場で使えて他の開発者にも保守できる基盤にするための協力を歓迎します。
このprojectでは、先回りした抽象化より現場の根拠、data所有権の明示、初見のmaintainerが
履歴を掘り返さず理解できる変更を重視します。

## 最初に読むもの

初回のrepository案内では、次の順序で一度読んでください。

1. [製品モデル](docs/product/ja/concepts/product-model.md) — IoTKitが所有するものと、
   device・外部applicationに残すもの。
2. [Architecture](docs/product/ja/architecture/system-overview.md) — 実行component、crate map、
   code配置、依存rule。
3. 対象となる[現行契約](docs/product/ja/index.md#contracts) — ingest、Input Adapter、
   Edge Node保管責任、Output Adapter。
4. [AGENTS.md](AGENTS.md) — 人間とcoding agentが共通で守る規則の入口（issue駆動、
   不変条件、lane）。詳細は[`.agents/`](.agents/)にあります。

`docs/product/`が現行の人間向け製品知識の正本です。中身は OKF v0.2 形式で
パッケージされています（形式名であり第二の正本ではありません）。`docs/okf/`は
互換スタブだけです。`docs/redesign/`と`docs/superpowers/`は履歴であり、現行契約、
実行可能fixture、testを上書きしません。

残る製品事実を変える変更では、同じ変更の中で product docs を最新に保ちます。
調査メモなど一時記録は issue / PR に置き、正本へ混ぜません。詳細は
[`.agents/documentation-authority.md`](.agents/documentation-authority.md) にあります。

更新候補の正本を path から機械的に列挙する lower-bound セレクタ:

```bash
node scripts/product-docs-impact.mjs select --base origin/master
```

候補が空でも「正本更新不要」の証明にはなりません。編集後は
`node scripts/check-product-docs.mjs` を実行してください。

PR の CI では、セレクタが候補を出したのに `docs/product/` の更新も
「更新しない理由」も無い場合に **soft 警告**が出ます。ジョブは失敗しません。
警告が出たら正本を更新するか、PR 本文に具体的な不要理由を書いてください。

以後の各taskでは、[`.agents/change-map.md`](.agents/change-map.md)の**Before
changing code**表を使い、変更に該当する行だけを読んでください。作業はissue駆動です。
[`.agents/workflow.md`](.agents/workflow.md)を参照してください。

## 開発環境

対応する開発環境はLinuxです。repository rootで`mise install`を実行すると、
`mise.toml`に固定した言語・CLI toolを導入できます。CIも同じ`mise.toml`を
`jdx/mise-action`経由で使用します。

- rustfmtとclippyを含むRust 1.95.0
- Console assetとtest用のNode.js 22、npm
- trial script用のPython 3.11
- trial validationとintegration test用のjq 1.8.2
- Raspberry Pi transport依存の`pkg-config`、`libudev-dev`

統合testのDocker Compose、OpenSSL、`curl`はhost dependencyとして残り、`mise`では
管理しません。通常の開発loopにRaspberry Piや実センサーは必要ありません。

```bash
mise install
mise exec -- node --version
mise exec -- cargo --version
```

DebianまたはUbuntuでは、言語以外のpackageを次で導入できます。

```bash
sudo apt-get update
sudo apt-get install --yes pkg-config libudev-dev docker.io docker-compose-v2 \
  openssl curl
```

credential、生成した証明書、local DB、deployment出力directoryをcommitしてはいけません。

通常の開発でhostの全coreを使い切らないよう、repositoryのCargo既定値はcompiler jobと
Rust test threadをそれぞれ4に制限します。既存の環境変数が優先されるため、必要な場合は
command単位で明示的に上書きできます。

```bash
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p iotkit-edge
```

## 最初の60分

### 0〜10分: 地図を確認する

```bash
git clone git@github.com:w-pinkietech/iotkit-next.git
cd iotkit-next
node scripts/check-product-docs.mjs
scripts/check-layers
scripts/check-source-layout
```

続いて、上で案内した製品モデルとArchitectureを読みます。実行経路を短く表すと
次のようになります。

```text
sensor / device
  -> Rust製IoTKit Edge Node
  -> MQTT Broker
  -> Rust製IoTKit Edge
  -> Output Adapter
  -> 外部application
```

### 10〜30分: 各領域でfocused testを一つ動かす

```bash
# Edge Node・Input Adapter
cargo test -p bravepi-mainboard-adapter

# IoTKit Edge・Output Adapter
cargo test -p iotkit-edge
cargo test -p iotkit-output-adapter-testkit

# Browser動作と生成済みConsole型
npm ci --prefix edge/frontend
npm run check --prefix edge/frontend
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
[`.agents/change-map.md`](.agents/change-map.md)の**Before changing code**表を
使ってください。

## 変更目的から場所を探す

[`.agents/change-map.md`](.agents/change-map.md)のtask-routing表だけを、必読資料、
code入口、認証付きHTTP ingest、Console認証、運用、契約に関するrepository共通の
地図とします。完全なcrate mapと配置ruleはArchitectureにあります。新しいcrateは、
同文書と`scripts/check-layers`へ分類するまで追加しません。

短いcomponent別の入口も用意しています。

- 収集とcustody: [`edge-node/README.ja.md`](edge-node/README.ja.md)
- Transport、Driver、Input Adapter:
  [`edge-node/adapters/README.ja.md`](edge-node/adapters/README.ja.md)
- Raw受理、semantic、出力、Console: [`edge/README.ja.md`](edge/README.ja.md)

新しいsensorでは、認証付きHTTP ingestを使えるか、既存direct-I2C Adapterへ
追加できるか、本当に新しいAdapter familyが必要かを先に判断します。
対応ICを一つ増やすだけのために新しいfamilyを作りません。

## 1 issue、1 worktree、1 pull request

すべての開発taskで次のloopを使います。

1. 成果と対象外が明確なGitHub issueを一つ作るか選ぶ。
2. local `master`を更新し、`agent/issue-<number>-<slug>`を作る。
3. `.worktrees/issue-<number>-<slug>`を作り、その中だけで作業する。
4. 製品動作を変更する前に、最も近いfocused testを追加または更新する。
5. 差分をissueの範囲に収める。範囲が実質的に変わったら別issueにする。
6. commitしてbranchをpushし、issueをcloseするdraft pull requestを作る。
7. そこで停止して人間へreviewを依頼する。
8. Review指摘は同じbranchとpull requestで修正する。
9. 明示承認を得てからmergeする。`User`である人間のアカウントで、associationが`OWNER`、
   `MEMBER`、`COLLABORATOR`のいずれか、かつrepositoryへのeffective permissionが`admin`、
   `maintain`、`write`のいずれかであるmaintainerだけが、default branchを対象とするopenかつ
   non-draftのPRへ完全一致の`/auto-merge`をcommentして承認を記録できる。GitHubは`required CI`を
   待ち、current headの`human approval` statusが成功してからnative squash auto-mergeを行う。
   opened、reopened、ready-for-review、synchronizeごとにPR headの`human approval`はpendingへ戻る。
   新しいcommitはauto-mergeを解除し、そのstatusをpendingへ戻すため、更新をreviewした後に
   もう一度完全一致のcommentを残して再度armする。これはreviewを置き換えない。default branchの
   protectionでは`required CI`、`human approval`、CodeQLを必須にする。

最終reviewでは、[レビュースイート](review/README.ja.md)から入り、合う
perspective を選びます。製品や運用に触れる差分では battle-tested perspective を
常に検討し、差分に関係する現場の失敗観点だけを selector で選びます。

```bash
node scripts/battle-tested-review.mjs select --base origin/master
```

選択方法、現場報告の秘匿化、review項目をtestまたはrunbookへ昇格させる規則は
[Battle-testedレビュー視点](review/battle-tested/README.ja.md)にあります。実環境で
問題を見つけた場合は、GitHubの`Field report / 現場報告` Issue formを使い、生ログ、設定、
DB、credential、顧客・工場・network・deviceの識別情報を添付しないでください。

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
node scripts/check-product-docs.mjs

# 対象crateのRust testとlint
cargo test -p <owning-crate>
cargo clippy -p <owning-crate> --all-targets -- -D warnings

# 明示した場合だけのworkspace全体診断。通常のPR sweepではない
scripts/verify.sh --workspace

# Console schema・生成型/asset・unit test
scripts/test-edge-console-frontend.sh

# Chromium上のConsole operator journey
scripts/test-edge-console-e2e.sh

# MQTT出力とPostgreSQL variant
scripts/test-edge-output.sh
IOTKIT_TEST_STORAGE_PROFILE=postgres scripts/test-edge-output.sh
scripts/test-edge-postgres.sh
```

通常のRust製品変更では、最も近いfocused testとlintから始めます。CIが変更範囲に応じて
Rust、Console、Edge、trial laneを選び、これが権威ある検証です。local passで置き換えません。
`scripts/verify.sh --workspace`は明示的なcross-workspace診断だけに使います。文書だけの変更は
通常、文書checker、link・command確認、`git diff --check`に絞ります。
`scripts/test-edge-host-release-gate.sh`はPRごとではなく、release candidateで一度実行します。
完全なownerと想定runtimeは[検証所有マトリクス](.github/verification-ownership.md)を参照してください。

## 生成fileとcontract変更

- `edge/openapi/edge-console-v1.yaml`を編集し、
  `npm run generate:api --prefix edge/frontend`を実行する。
- 埋め込みConsole JavaScriptは
  `npm run build --prefix edge/frontend`で生成する。
- `Cargo.lock`、`package-lock.json`は各package managerからだけ更新する。
- `docs/product/`の日英fileは同時に変更し、concept 内容を変えたら共有 `revision` を上げる。
- `testdata/`の共有JSONは正規contract dataとして扱う。一実装を通すためだけに
  fixtureを変更しない。

## 絶対に破らない境界

- Token、credential、鍵、そのhashをlog、error、audit、fixture、PRへ出さない。
- 文書上のdurability pointより前にackせず、未ack originalを黙って削除しない。
- 状態変更は所有componentのtyped operation dispatcherを通す。HTTP、UI、CLI、
  AdapterからSQLへ直接writeする経路を追加しない。
- Rust製品test本体は製品`src/`外、frontend unit testは
  `edge/frontend/tests/unit/`へ置く。
- Legacy planや旧codeを新しい動作の正本にしない。

## Pull request checklist

- PRが一つのissueをlinkし、closeする。
- Descriptionに変更内容、理由、影響、検証を書く。
- 公開動作に実行可能testまたはfixtureがある。
- Contract変更では全表現を一緒に更新する。
- Operator・contributor workflow変更では文書を更新する。
- Battle-tested selectorの関連ID、または該当なしの理由を書く。
- 無関係なrefactor、secret、local DB、生成証明書、deployment artifactを含めない。
- Branchは人間がreviewできる状態だが、mergeされていない。
