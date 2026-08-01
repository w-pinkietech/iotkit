# IoTKitへのコントリビュート

日本語 | [English](CONTRIBUTING.md)

IoTKitを、現場で使えて他の開発者にも保守できる基盤にするための協力を歓迎します。
このprojectでは、先回りした抽象化より現場の根拠、data所有権の明示、初見のmaintainerが
履歴を掘り返さず理解できる変更を重視します。

## 最初に読むもの

初回のrepository案内では、次の順序で一度読んでください。

1. [製品モデル](docs/okf/ja/concepts/product-model.md) — IoTKitが所有するものと、
   device・外部applicationに残すもの。
2. [Architecture](docs/okf/ja/architecture/system-overview.md) — 実行component、crate map、
   code配置、依存rule。
3. 対象となる[現行契約](docs/okf/ja/index.md#contracts) — ingest、Input Adapter、
   Edge Node保管責任、Output Adapter。
4. [AGENTS.md](AGENTS.md) — 人間とcoding agentが共通で守る不変条件と検証lane。

`docs/okf/`が現行の人間向け製品知識の正本です。`docs/redesign/`と
`docs/superpowers/`は履歴であり、現行契約、実行可能fixture、testを上書きしません。

残る製品事実を変える変更では、同じ変更の中で OKF を最新に保ちます。調査メモや
捨てた案など一時的な記録は issue / PR に置き、正本へ混ぜません。詳細は
[AGENTS.md](AGENTS.md) の **Keep OKF current** と change-lane 表です。

以後の各taskでは、[AGENTS.md](AGENTS.md)の**Before changing code**表を使い、
変更に該当する行だけを読んでください。

## 開発環境

対応する開発環境はLinuxです。現在のCIは次を使います。

- `rust-toolchain.toml`が自動選択するRust 1.95.0
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

Rustは[rustup](https://rustup.rs/)、Node.js 22は通常使うpackage managerで
導入してください。credential、生成した証明書、local DB、deployment出力directoryを
commitしてはいけません。

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
[AGENTS.md](AGENTS.md)の**Before changing code**表を使ってください。

## 変更目的から場所を探す

[AGENTS.md](AGENTS.md)のtask-routing表だけを、必読資料、code入口、認証付きHTTP
ingest、Console認証、運用、契約に関するrepository共通の地図とします。完全なcrate
mapと配置ruleはArchitectureにあります。新しいcrateは、同文書と
`scripts/check-layers`へ分類するまで追加しません。

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
6. 残る製品事実が変わるなら、同じ worktree で対応する `docs/okf/` を更新する
   （日英同時、`revision` を上げる）。変わらないなら、PR 用に **No OKF update**
   の具体的な理由を用意する。
7. commitしてbranchをpushし、issueをcloseするdraft pull requestを作る。
   PR の **OKF impact / 正本への影響** 欄に、更新パスまたは更新しない理由を書く。
8. そこで停止して人間へreviewを依頼する。
9. Review指摘は同じbranchとpull requestで修正する。
10. 明示承認を得てからmergeする。

最終reviewでは、差分に関係する現場の失敗観点だけを選びます。

```bash
node scripts/battle-tested-review.mjs select --base origin/master
```

選択方法、現場報告の秘匿化、review項目をtestまたはrunbookへ昇格させる規則は
[Battle-testedレビュースイート](review/battle-tested/README.ja.md)にあります。実環境で
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

- `edge/openapi/edge-console-v1.yaml`を編集し、
  `npm run generate:api --prefix edge/frontend`を実行する。
- 埋め込みConsole JavaScriptは
  `npm run build --prefix edge/frontend`で生成する。
- `Cargo.lock`、`package-lock.json`は各package managerからだけ更新する。
- `docs/okf/`の日英fileは同時に変更し、concept 内容を変えたら共有 `revision` を上げる。
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
- **OKF impact / 正本への影響** に、更新した `docs/okf/` パス（日英）**または**
  更新しない具体的な理由を書く。残る製品事実を issue だけに残さない。
- 公開動作に実行可能testまたはfixtureがある。
- Contract変更では全表現を一緒に更新する（versioned 契約なら OKF + schema/types +
  fixtures + conformance tests）。
- merge 後も真である operator / contributor 手順は OKF に反映する（正本が
  変わらないなら PR で理由を書く）。
- Battle-tested selectorの関連ID、または該当なしの理由を書く。
- 無関係なrefactor、secret、local DB、生成証明書、deployment artifactを含めない。
- Branchは人間がreviewできる状態だが、mergeされていない。
