# IoTKit Site Resilience Matrix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Siteのtransaction失敗、conflict、downstream停止、Edge/Broker/Site再起動でも、Edgeの保管責任と連続cursorが壊れず、重複なく収束することを証明する。

**Architecture:** atomicityとno-ack-on-errorはGoの実SQLite/Processorテストで決定的に検証する。process境界は通常binaryだけを使うhost側Docker scriptで検証し、Raspberry PiはUARTとBravePI Mainboardの復帰だけに限定する。

**Tech Stack:** Go 1.25 / modernc SQLite / Paho MQTT、Rust 1.95 / Tokio / rusqlite / rumqttc、Mosquitto 2、Docker Compose、Bash、SQLite CLI、Raspberry Pi Debian 13 arm64。

## Global Constraints

- 正本はD3/D9/D10。新しいwire、schema、custody契約を作らない。
- production failpoint、test-only production config、privileged fault injectionを追加しない。
- Product Store、Processor、Edge、wire、schemaは変更しない。実不具合が出たら停止して診断する。
- normal CIは高速な`go test ./...`だけ。重いDocker matrixはPR前に一度だけ実行する。
- 300 publicationで256-record batch境界を跨ぎ、実時間の長時間待機はしない。
- scriptは一意なCompose project、private temp directory、bounded wait、共通cleanupを使う。
- secretをterminal、log、Gitへ出さない。
- Piは`ssh -F /dev/null`、Dockerは`sudo -n docker`。state変更直前にユーザー承認を得る。
- subagent、外部review、push、PR、mergeはユーザーの別の明示依頼なしに実行しない。

---

### Task 1: Fast Site custody tests

**Files:**
- Modify: `iotkit-site/internal/store/store_test.go`
- Modify: `iotkit-site/internal/mqttsite/processor_test.go`

**Interfaces:**
- Consumes: `Store.AcceptBatch`, `Store.ListRawRecords`, `ErrConflict`, `Processor.Process`。
- Produces: transaction失敗/conflictからack非送信までのfast regression coverage。

- [ ] **Step 1: Store testの事後状態を強化する**

`TestExactReplayIsIdempotent`へ`cursor == 1`を追加する。`TestCursorWriteFailureRollsBackRawInsert`へ`accepted_cursors count == 0`を追加する。`TestConflictingReplayDoesNotAdvanceCursor`は最初の`ListRawRecords`結果を保存し、conflict後も`bytes.Equal(after[0].Record, before[0].Record)`かつ`cursor == 1`を確認する。

- [ ] **Step 2: focused Store testsを実行する**

```bash
docker run --rm --user "$(id -u):$(id -g)" \
  -e HOME=/tmp -e GOMODCACHE=/tmp/gomodcache -e GOCACHE=/tmp/gocache \
  -v /tmp/iotkit-go-mod:/tmp/gomodcache -v /tmp/iotkit-go-cache:/tmp/gocache \
  -v "$PWD:/src" -w /src/iotkit-site golang:1.25-bookworm \
  go test ./internal/store -run 'TestExactReplay|TestConflictingReplay|TestCursorWriteFailure' -count=1
```

Expected: PASS。既存behaviorのcharacterizationなのでproduction changeは不要。

- [ ] **Step 3: Processorをreal Store failureへ結合する**

`processor_test.go`へ`bytes`、`database/sql`、`path/filepath`、alias import `sitestore`を追加する。既存`validPayload`を`payloadWithMarker(t, marker)`経由にして、同じbatch identityでrecord JSONだけを変えられるようにする。

追加test:

```go
func TestProcessDoesNotPublishForRealStoreConflict(t *testing.T)
func TestProcessDoesNotPublishWhenRealStoreTransactionFails(t *testing.T)
```

前者は一度成功させた後、同じ`pub_seq=1`を別markerで処理し、`errors.Is(err, sitestore.ErrConflict)`、publish回数1、保存markerがoriginalのままを確認する。

後者は`store.Open(temp/site.db)`後、別`sql.DB`から次のtriggerを作る。

```sql
CREATE TRIGGER fail_cursor BEFORE INSERT ON accepted_cursors
BEGIN SELECT RAISE(ABORT, 'injected cursor failure'); END;
```

その状態で`Processor.Process`を呼び、errorあり、publishなし、raw row 0、cursor row 0を確認する。

- [ ] **Step 4: formatとfocused testsを実行する**

```bash
docker run --rm --user "$(id -u):$(id -g)" \
  -e HOME=/tmp -e GOMODCACHE=/tmp/gomodcache -e GOCACHE=/tmp/gocache \
  -v /tmp/iotkit-go-mod:/tmp/gomodcache -v /tmp/iotkit-go-cache:/tmp/gocache \
  -v "$PWD:/src" -w /src/iotkit-site golang:1.25-bookworm \
  sh -c 'gofmt -w internal/store/store_test.go internal/mqttsite/processor_test.go && go test ./internal/store ./internal/mqttsite -count=1'
```

Expected: both packages PASS、product file diffなし。

- [ ] **Step 5: commitする**

```bash
git add iotkit-site/internal/store/store_test.go iotkit-site/internal/mqttsite/processor_test.go
git diff --cached --check
git commit -m "test: strengthen Site custody failure coverage"
```

---

### Task 2: Fast Site CI job

**Files:**
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: `iotkit-site/go.mod`, `go.sum`, Task 1 tests。
- Produces: Rust jobと並列のfast `site go test` job。

- [ ] **Step 1: Go jobを追加する**

```yaml
  site:
    name: site go test
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: iotkit-site
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-go@v5
        with:
          go-version-file: iotkit-site/go.mod
          cache-dependency-path: iotkit-site/go.sum
      - name: Tests
        run: go test ./...
```

- [ ] **Step 2: local fast suiteを確認してcommitする**

```bash
docker run --rm --user "$(id -u):$(id -g)" \
  -e HOME=/tmp -e GOMODCACHE=/tmp/gomodcache -e GOCACHE=/tmp/gocache \
  -v /tmp/iotkit-go-mod:/tmp/gomodcache -v /tmp/iotkit-go-cache:/tmp/gocache \
  -v "$PWD:/src" -w /src/iotkit-site golang:1.25-bookworm go test ./...
git diff --check
git add .github/workflows/ci.yml
git commit -m "ci: run fast Site tests"
```

Expected: all Site packages PASS。Docker matrixは実行しない。

---

### Task 3: Host Docker resilience matrix

**Files:**
- Create: `scripts/test-site-resilience.sh`

**Interfaces:**
- Consumes: `compose.dev.yaml`、normal Edge/Site binaries、Mosquitto、Edge/Site SQLite。
- Produces: final cursor 304、Site 304 distinct contiguous rows、両DB quick checkを証明するexecutable script。

- [ ] **Step 1: isolated harnessを作る**

既存`test-site-mqtt.sh`の秘密値を出さないcredential/ACL作成パターンを使うが、happy-path script自体は変更しない。新scriptに次を持たせる。

```bash
compose()              # unique project + compose.dev.yaml
start_edge()           # edge-N.logへ出力しPIDを保持
stop_edge()            # 保持PIDだけへSIGINT
restart_edge()
edge_cursor()
site_stats()           # count|min|max|distinct count
diagnostics()          # secretを含まないlogs/count/cursor
wait_for_convergence() # 0.5秒 x 180回
cleanup()              # trap EXIT、Edge/Compose/tempだけを削除
```

初期値は`edge_pid=""`、`edge_node_id=""`、`ledger_epoch=""`とし、早期失敗時も`set -u`でdiagnosticsが壊れないようにする。依存commandは`cargo docker openssl sqlite3`と`docker compose version`。

- [ ] **Step 2: normal DB/configとcredentialを初期化する**

BravePI、rpi-local、APIを無効、MQTTを`127.0.0.1:18883`・`allow_insecure=true`とする一時`edge.toml`を作る。BrokerなしでEdgeを短く起動し、`edge_node_id`と`epoch`を読む。dynamic Edge userとSite userだけのACL/password databaseを作る。password内容は表示しない。

- [ ] **Step 3: deterministic outbox seederを作る**

`seed_range first last`はEdge停止中に、unquoted heredocで次のSQLを実行する。`$first`、`$last`、`$ledger_epoch`はshell functionの値へ展開する。explicit `pub_seq`によりack済みrowがpurgeされても次rangeを維持する。

```sql
WITH RECURSIVE n(value) AS (
  SELECT $first UNION ALL SELECT value + 1 FROM n WHERE value < $last
)
INSERT INTO publication_log(pub_seq, epoch, kind, subtype, annotation_json, created_at)
SELECT value, '$ledger_epoch', 'annotation',
       printf('resilience_%06d', value),
       '{"prior_epoch":"resilience-prior"}',
       1700000000000 + value
FROM n;
```

- [ ] **Step 4: ordered matrixを実装する**

順序とassertionを固定する。

1. `seed_range 1 300`。BrokerなしでEdgeを起動・再起動し、outbox 300、cursor 0。
2. Broker/Site起動。cursor 300、Site stats `300|1|300|300`。
3. Site停止、301をseed、Edge再起動。Broker稼働中でも2秒後cursor 300。
4. Site復旧とEdge再接続。cursor/Site stats 301へ収束。
5. 302をseedしてEdgeだけ再起動し、302へ収束。
6. Broker停止中に303をseedし、Brokerだけ再起動して303へ収束。
7. Siteだけ再起動後に304をseedし、Edgeの30秒retryをbounded waitして304へ収束。
8. Edge/Siteを停止し、両DB`PRAGMA quick_check = ok`、final cursor 304、Site stats `304|1|304|304`。

成功時だけ次を表示する。

```text
Edge/Broker/Site resilience matrix: OK (304 contiguous records)
```

- [ ] **Step 5: syntax/secret checkを行ってcommitする**

```bash
chmod +x scripts/test-site-resilience.sh
bash -n scripts/test-site-resilience.sh
if rg -n 'set -x|cat .*password|echo .*password|private.key' scripts/test-site-resilience.sh; then exit 1; fi
git diff --check
git add scripts/test-site-resilience.sh
git commit -m "test: add Site resilience matrix"
```

Expected: syntax PASS、secret-print patternなし。重いmatrixはまだ実行しない。

---

### Task 4: One final host verification and canonical evidence

**Files:**
- Modify: `docs/redesign/decisions/D3-process-and-wave-decisions.md`

**Interfaces:**
- Consumes: Tasks 1-3、`scripts/verify.sh`、existing MQTT vertical slice。
- Produces: fresh full host evidence and D3 record。

- [ ] **Step 1: scopeを確認する**

```bash
git status --short --branch
git diff origin/master...HEAD --stat
git diff --check
```

Expected: design/plan、2 Go test files、CI、new scriptだけ。product file変更があれば停止する。

- [ ] **Step 2: final full host verificationを一度だけ実行する**

```bash
scripts/verify.sh
docker run --rm --user "$(id -u):$(id -g)" \
  -e HOME=/tmp -e GOMODCACHE=/tmp/gomodcache -e GOCACHE=/tmp/gocache \
  -v /tmp/iotkit-go-mod:/tmp/gomodcache -v /tmp/iotkit-go-cache:/tmp/gocache \
  -v "$PWD:/src" -w /src/iotkit-site golang:1.25-bookworm go test ./...
scripts/test-site-mqtt.sh
scripts/test-site-resilience.sh
```

Expected: Rust full gate、Go全package、existing MQTT slice、新matrixがすべてPASS。失敗時はPRへ進まず原因を診断し、成功証拠が失われた検査だけを再実行する。product修正が必要ならscope変更としてユーザーへ報告する。

- [ ] **Step 3: host evidenceをD3へ追記してcommitする**

D3へ日付、commit、transaction失敗時のraw/cursor/ack 0、conflict時のoriginal/cursor保持、Broker停止中300件とEdge再起動、2 batch以上で300へ収束、Site停止中cursor不変、個別再起動後final cursor 304、Site 304 distinct contiguous rows、両DB quick check `ok`を記録する。secretは記載しない。

```bash
git add docs/redesign/decisions/D3-process-and-wave-decisions.md
git diff --cached --check
git commit -m "docs: record Site resilience validation"
```

---

### Task 5: Raspberry Pi hardware-only confirmation

**Files:**
- Modify: `docs/redesign/decisions/D3-process-and-wave-decisions.md`

**Interfaces:**
- Consumes: `/home/iotkit/iotkit-lab`、`/home/iotkit/iotkit/iotkit-next-current`、`ble:246880020140018b`。
- Produces: UART/real-sensor recovery evidence only。

- [ ] **Step 1: read-only preflightを行う**

```bash
ssh -F /dev/null -o BatchMode=yes -o ConnectTimeout=5 iotkit@iotkit '
  uname -m; readlink -f /dev/serial0; id
  test -f /home/iotkit/iotkit-lab/edge.db
  test -f /home/iotkit/iotkit-lab/edge.toml
  pgrep -af iotkit-edge || true
  sudo -n docker ps --format "{{.Names}} {{.Status}}"
  sqlite3 /home/iotkit/iotkit-lab/edge.db "
    SELECT count(*) FROM readings;
    SELECT cursor_pub_seq FROM target_registry WHERE target_id='''site'\'';
    PRAGMA quick_check;
  "
'
```

Expected: arm64、`/dev/ttyAMA0`、dialout membership、lab DB/config、quick check `ok`。BravePI信号停止中ならユーザーへ再開を依頼する。

- [ ] **Step 2: state変更の承認を得る**

labのBroker/Site停止、lab PIDのEdge起動/再起動、downstream復旧、最後のEdge停止を提示して承認を得る。Mainboard power cycleは別途ユーザーの物理操作として依頼する。

- [ ] **Step 3: downstream停止中のcollectionとEdge restartを確認する**

承認後、Pi上で`sudo -n docker compose`を使ってlabのSite/Brokerだけを停止する。lab PID fileのEdgeだけをSIGINT/startし、45秒前後で`readings`とcurrent-epoch `publication_log`が増え、cursorが停止前値のままであることを確認する。Edgeをもう一度再起動し、修復操作なしで`temperature_c` collectionが再開することを確認する。

- [ ] **Step 4: downstreamを復旧して全件収束を確認する**

labのBroker/Siteを`sudo -n docker compose up --detach broker site`で戻す。Edge cursorが停止中のcurrent-epoch `MAX(publication_log.pub_seq)`へ到達し、Siteに同じ`edge_node_id`、`epoch`、欠けのないrangeがあることを確認する。

- [ ] **Step 5: Mainboard restart recoveryを確認する**

ユーザーへBravePI Mainboardのpower cycleを一度依頼する。操作前後の`MAX(readings.seq)`を比較し、45秒以内に新しい`temperature_c`が入り、config/DB/device approvalを修復せず復帰することを確認する。物理操作できない場合はTask未完了として報告し、実装ゲート完了とは書かない。

- [ ] **Step 6: 安全に停止してD3へ記録する**

lab PIDのEdgeだけをSIGINTで止め、UART解放、Edge/Site両DB quick check `ok`を確認する。D3へ日付、commit、sensor ID、停止前後のreading/pub_seq/cursor、Edge/Mainboard restart recovery、最終cursor、quick check、UART解放を記録する。source、lab、credentials、containersは削除しない。

```bash
git add docs/redesign/decisions/D3-process-and-wave-decisions.md
git diff --cached --check
git commit -m "docs: record resilience hardware validation"
git diff origin/master...HEAD --check
git status --short --branch
```

Expected: clean worktree。Task 4後は観測事実の文書だけなのでfull code suiteを再実行しない。結果を報告し、push/PR前で停止する。
