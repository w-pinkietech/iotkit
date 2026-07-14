# Site Semantic Projection and MQTT Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** IoTKit Siteで保存済みのscalar接点seriesをfuture-onlyで`production_pulse`へ意味付けし、耐久outboxからversion付きMQTT eventとして配送する。

**Architecture:** raw archive/cursor custodyは現状のまま最優先で確定し、semantic projectorはcommit済みrawを非同期に読む。projectorは安定event IDとmapping内sequenceを生成し、独立したMQTT exporterがpayloadへ変換してPUBACK後だけoutboxを完了する。設定と確認は既存`iotkit-site` CLIへ追加し、UI、過去backfill、旧YokaKitの`ipAddress/pinNumber`完全互換は作らない。

**Tech Stack:** Go 1.25、`database/sql`、modernc SQLite、Paho MQTT、既存IoTKit Site CLI。

## Global Constraints

- raw `AcceptBatch` transactionと`accepted-through`はsemantic projection/exportの成否から独立させる。
- source seriesごとにactiveなsemantic mappingは1つまでとする。
- 初期meaningは`production_pulse`だけとする。
- triggerは`active_sample`または`active_edge`、`active_value`は0または1を必須指定する。暗黙defaultを作らない。
- `active_edge`はmapping作成後の最初のsampleをbaselineとして保存し、最初からactiveでもeventを生成しない。
- `active_sample`はactiveなsampleごとにeventを生成する。
- mapping作成前のraw recordを処理しない。backfill API/CLIを作らない。
- mapping改訂はfuture-onlyとし、改訂前eventを書き換えない。
- 1つのmappingから複数MQTT routeへfan-outできる。route追加前のeventは送らない。
- MQTT payloadは新しいversion付きIoTKit契約とし、旧`ipAddress`/`pinNumber`を復活させない。
- deliveryはat-least-onceとし、consumerがdedupできる安定`event_id`を必ず含める。
- MQTT PUBACK後だけoutboxをpublishedにする。timeout/切断時はpendingのまま再送する。
- credentialをDB payload、ログ、error、CLI outputへ含めない。
- 新しいWeb UI、login、Site Console、Edge管理経路、Edge R9、YokaKit business masterを作らない。
- 各Taskはfocused Go testだけを実行する。`scripts/verify.sh`は最終Taskで1回だけ実行する。

---

### Task 1: Semantic vocabulary、mapping、future-only開始境界を実装する

**Files:**
- Create: `iotkit-site/internal/semantic/types.go`
- Create: `iotkit-site/internal/semantic/types_test.go`
- Modify: `iotkit-site/internal/store/store.go`
- Modify: `iotkit-site/internal/store/store_test.go`

**Interfaces:**
- Produces `semantic.Meaning`, `semantic.TriggerMode`, `semantic.MappingSpec`, `semantic.Mapping`.
- Produces `Store.PutSemanticMapping(ctx, spec)` and `Store.ListSemanticMappings(ctx)`.

- [ ] **Step 1: vocabulary validationの失敗testを書く**

```go
func TestMappingSpecValidate(t *testing.T) {
    valid := MappingSpec{EdgeNodeID: "edge-node-01", SeriesKey: "subject:contact_state:na:primary", Meaning: MeaningProductionPulse, TriggerMode: TriggerActiveSample, ActiveValue: 1}
    if err := valid.Validate(); err != nil { t.Fatal(err) }
    for _, bad := range []MappingSpec{
        {EdgeNodeID: "edge-node-01", SeriesKey: valid.SeriesKey, Meaning: "production", TriggerMode: TriggerActiveSample, ActiveValue: 1},
        {EdgeNodeID: "edge-node-01", SeriesKey: valid.SeriesKey, Meaning: MeaningProductionPulse, TriggerMode: "automatic", ActiveValue: 1},
        {EdgeNodeID: "edge-node-01", SeriesKey: valid.SeriesKey, Meaning: MeaningProductionPulse, TriggerMode: TriggerActiveEdge, ActiveValue: 2},
    } {
        if bad.Validate() == nil { t.Fatalf("accepted invalid spec: %#v", bad) }
    }
}
```

- [ ] **Step 2: testが型未定義で失敗することを確認する**

Run: `(cd iotkit-site && go test ./internal/semantic)`

Expected: packageまたは型未定義でFAIL。

- [ ] **Step 3: typed vocabularyを実装する**

```go
type Meaning string
const MeaningProductionPulse Meaning = "production_pulse"

type TriggerMode string
const (
    TriggerActiveSample TriggerMode = "active_sample"
    TriggerActiveEdge TriggerMode = "active_edge"
)

type MappingSpec struct {
    EdgeNodeID string `json:"edge_node_id"`
    SeriesKey string `json:"series_key"`
    Meaning Meaning `json:"meaning"`
    TriggerMode TriggerMode `json:"trigger_mode"`
    ActiveValue int `json:"active_value"`
}
```

`Validate`は空identity、`edge_node_id`内の`/+#`、未知meaning/mode、0/1以外を拒否する。default補完はしない。

- [ ] **Step 4: mapping schemaとfuture-only cursor snapshot testを書く**

```go
func TestPutSemanticMappingCapturesEveryExistingEpochCursor(t *testing.T) {
    store := openTestStore(t)
    accept(t, store, batch("edge-node-01", "epoch-a", 1, contact(1, 0)))
    accept(t, store, batch("edge-node-01", "epoch-b", 1, contact(1, 1)))
    mapping, err := store.PutSemanticMapping(context.Background(), semantic.MappingSpec{
        EdgeNodeID: "edge-node-01", SeriesKey: contactSeries,
        Meaning: semantic.MeaningProductionPulse,
        TriggerMode: semantic.TriggerActiveSample, ActiveValue: 1,
    })
    if err != nil { t.Fatal(err) }
    if got := store.testMappingStarts(t, mapping.ID, mapping.Revision); !reflect.DeepEqual(got, map[string]int64{"epoch-a": 1, "epoch-b": 1}) {
        t.Fatalf("starts = %#v", got)
    }
}
```

- [ ] **Step 5: mapping persistenceを実装する**

```sql
CREATE TABLE IF NOT EXISTS semantic_mappings (
  mapping_id TEXT NOT NULL,
  revision INTEGER NOT NULL,
  edge_node_id TEXT NOT NULL,
  series_key TEXT NOT NULL,
  meaning TEXT NOT NULL CHECK(meaning = 'production_pulse'),
  trigger_mode TEXT NOT NULL CHECK(trigger_mode IN ('active_sample','active_edge')),
  active_value INTEGER NOT NULL CHECK(active_value IN (0,1)),
  active INTEGER NOT NULL CHECK(active IN (0,1)),
  created_at INTEGER NOT NULL,
  PRIMARY KEY(mapping_id, revision)
);
CREATE UNIQUE INDEX IF NOT EXISTS ux_semantic_one_active_per_source
  ON semantic_mappings(edge_node_id, series_key) WHERE active = 1;
CREATE TABLE IF NOT EXISTS semantic_mapping_starts (
  mapping_id TEXT NOT NULL,
  mapping_revision INTEGER NOT NULL,
  ledger_epoch TEXT NOT NULL,
  start_after_pub_seq INTEGER NOT NULL,
  PRIMARY KEY(mapping_id, mapping_revision, ledger_epoch)
);
```

`PutSemanticMapping`は同じsourceの旧active revisionをdeactivateし、同じ`mapping_id`のrevisionを1増やす。初回IDは`crypto/rand`の128bitをhex化した`sm-...`とする。同一transactionで当該Edgeの全`accepted_cursors`を`semantic_mapping_starts`へsnapshotする。

- [ ] **Step 6: focused testを通してcommitする**

Run: `(cd iotkit-site && go test ./internal/semantic ./internal/store)`

Expected: PASS。

Commit: `feat: add site semantic mappings`

---

### Task 2: Commit済みrawから決定的なsemantic eventを生成する

**Files:**
- Create: `iotkit-site/internal/semantic/evaluator.go`
- Create: `iotkit-site/internal/semantic/evaluator_test.go`
- Modify: `iotkit-site/internal/store/store.go`
- Modify: `iotkit-site/internal/store/store_test.go`

**Interfaces:**
- Produces `semantic.Evaluate(mode, activeValue, previous, current)`.
- Produces `Store.ProjectSemanticEvents(ctx, limit)` and `Store.ListSemanticEvents(ctx, limit)`.

- [ ] **Step 1: trigger semanticsの失敗testを書く**

```go
func TestEvaluateModes(t *testing.T) {
    if got := evaluateSequence(TriggerActiveSample, 1, []int{1, 1, 0, 1}); !reflect.DeepEqual(got, []bool{true, true, false, true}) {
        t.Fatalf("active_sample = %#v", got)
    }
    if got := evaluateSequence(TriggerActiveEdge, 1, []int{1, 1, 0, 1}); !reflect.DeepEqual(got, []bool{false, false, false, true}) {
        t.Fatalf("active_edge = %#v", got)
    }
}
```

- [ ] **Step 2: testが関数未定義で失敗することを確認する**

Run: `(cd iotkit-site && go test ./internal/semantic -run TestEvaluateModes)`

Expected: FAIL。

- [ ] **Step 3: pure evaluatorを実装する**

```go
func Evaluate(mode TriggerMode, activeValue int, previous *int, current int) (emit bool, next int, err error) {
    if current != 0 && current != 1 { return false, 0, errors.New("contact value must be 0 or 1") }
    switch mode {
    case TriggerActiveSample:
        return current == activeValue, current, nil
    case TriggerActiveEdge:
        return previous != nil && *previous != activeValue && current == activeValue, current, nil
    default:
        return false, 0, errors.New("unsupported trigger mode")
    }
}
```

- [ ] **Step 4: projection、dedup、no-backfill testを書く**

```go
func TestProjectSemanticEventsIsFutureOnlyAndIdempotent(t *testing.T) {
    store := openTestStore(t)
    accept(t, store, batch("edge-node-01", "epoch-a", 1, contact(1, 1)))
    mapping := putActiveSampleMapping(t, store)
    accept(t, store, batch("edge-node-01", "epoch-a", 2, contact(2, 1), contact(3, 0), contact(4, 1)))
    if _, err := store.ProjectSemanticEvents(context.Background(), 100); err != nil { t.Fatal(err) }
    if _, err := store.ProjectSemanticEvents(context.Background(), 100); err != nil { t.Fatal(err) }
    events := listEvents(t, store)
    assertEventSequences(t, events, mapping, []int64{1, 2})
    assertSourcePubSeqs(t, events, []int64{2, 4})
}
```

- [ ] **Step 5: processing tablesとprojectorを実装する**

```sql
CREATE TABLE IF NOT EXISTS semantic_mapping_state (
  mapping_id TEXT NOT NULL,
  mapping_revision INTEGER NOT NULL,
  last_value INTEGER,
  next_event_sequence INTEGER NOT NULL,
  PRIMARY KEY(mapping_id, mapping_revision)
);
CREATE TABLE IF NOT EXISTS semantic_results (
  mapping_id TEXT NOT NULL,
  mapping_revision INTEGER NOT NULL,
  ledger_epoch TEXT NOT NULL,
  pub_seq INTEGER NOT NULL,
  emitted_event_id TEXT,
  PRIMARY KEY(mapping_id, mapping_revision, ledger_epoch, pub_seq)
);
CREATE TABLE IF NOT EXISTS semantic_events (
  event_row_id INTEGER PRIMARY KEY AUTOINCREMENT,
  event_id TEXT NOT NULL UNIQUE,
  mapping_id TEXT NOT NULL,
  mapping_revision INTEGER NOT NULL,
  event_sequence INTEGER NOT NULL,
  meaning TEXT NOT NULL,
  edge_node_id TEXT NOT NULL,
  ledger_epoch TEXT NOT NULL,
  source_pub_seq INTEGER NOT NULL,
  source_series_key TEXT NOT NULL,
  occurred_at INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE(mapping_id, mapping_revision, event_sequence)
);
```

projectorはactive mappingごとに、`semantic_results`未登録かつstart snapshotより後のmatching rawを`received_at, ledger_epoch, pub_seq`順で読む。`values`がscalar 0/1でないrecordは結果を進めずerrorを返す。各入力についてstate更新、result、必要ならevent insertを同じtransactionで行う。`event_id`は`sha256(mapping_id, revision, edge_node_id, ledger_epoch, pub_seq)`のlowercase hexとする。

- [ ] **Step 6: focused testを通してcommitする**

Run: `(cd iotkit-site && go test ./internal/semantic ./internal/store)`

Expected: PASS。

Commit: `feat: project site semantic events`

---

### Task 3: Version付きMQTT payload、route、耐久outboxを実装する

**Files:**
- Create: `iotkit-site/internal/applicationcontract/production_pulse.go`
- Create: `iotkit-site/internal/applicationcontract/production_pulse_test.go`
- Modify: `iotkit-site/internal/store/store.go`
- Modify: `iotkit-site/internal/store/store_test.go`

**Interfaces:**
- Produces `applicationcontract.ProductionPulseV1` and strict `Validate`.
- Produces `Store.PutMQTTRoute`, `Store.EnqueueMQTTExports`, `Store.ListPendingMQTTExports`, `Store.MarkMQTTExportPublished`.

- [ ] **Step 1: payload contract testを書く**

```go
func TestProductionPulseV1RoundTrip(t *testing.T) {
    event := ProductionPulseV1{
        SchemaVersion: 1, EventID: "event-01", MappingID: "sm-01", MappingRevision: 1,
        EventSequence: 2, Meaning: "production_pulse", EdgeNodeID: "edge-node-01",
        SourceSeriesKey: "subject:contact_state:na:primary", SourcePubSeq: 8,
        OccurredAt: 1720000000000, Count: 2,
    }
    if err := event.Validate(); err != nil { t.Fatal(err) }
    encoded, _ := json.Marshal(event)
    if bytes.Contains(encoded, []byte("ipAddress")) || bytes.Contains(encoded, []byte("pinNumber")) {
        t.Fatalf("legacy coordinate leaked: %s", encoded)
    }
}
```

- [ ] **Step 2: testが型未定義で失敗することを確認する**

Run: `(cd iotkit-site && go test ./internal/applicationcontract)`

Expected: FAIL。

- [ ] **Step 3: v1 payloadを実装する**

```go
type ProductionPulseV1 struct {
    SchemaVersion uint32 `json:"schema_version"`
    EventID string `json:"event_id"`
    MappingID string `json:"mapping_id"`
    MappingRevision int64 `json:"mapping_revision"`
    EventSequence int64 `json:"event_sequence"`
    Meaning string `json:"meaning"`
    EdgeNodeID string `json:"edge_node_id"`
    SourceSeriesKey string `json:"source_series_key"`
    SourcePubSeq int64 `json:"source_pub_seq"`
    OccurredAt int64 `json:"occurred_at"`
    Count int64 `json:"count"`
}
```

`Count`はmapping revision内の`event_sequence`と同値にする。旧YokaKit互換fieldは追加しない。

- [ ] **Step 4: routeのfuture-only fan-outとoutbox testを書く**

```go
func TestRouteExportsOnlyEventsCreatedAfterRouteAndFansOut(t *testing.T) {
    store := storeWithOneSemanticEvent(t)
    routeA := putRoute(t, store, "factory/a/production-pulses")
    routeB := putRoute(t, store, "factory/b/production-pulses")
    projectAnotherEvent(t, store)
    if _, err := store.EnqueueMQTTExports(context.Background(), 100); err != nil { t.Fatal(err) }
    pending := listPending(t, store)
    assertRoutes(t, pending, []string{routeA.Topic, routeB.Topic})
    assertCounts(t, pending, []int64{2, 2})
}
```

- [ ] **Step 5: route/outbox schemaを実装する**

```sql
CREATE TABLE IF NOT EXISTS mqtt_routes (
  route_id TEXT PRIMARY KEY,
  mapping_id TEXT NOT NULL,
  topic TEXT NOT NULL,
  qos INTEGER NOT NULL CHECK(qos = 1),
  start_after_event_row_id INTEGER NOT NULL,
  active INTEGER NOT NULL CHECK(active IN (0,1)),
  created_at INTEGER NOT NULL,
  UNIQUE(mapping_id, topic)
);
CREATE TABLE IF NOT EXISTS mqtt_export_outbox (
  export_id TEXT PRIMARY KEY,
  route_id TEXT NOT NULL,
  event_id TEXT NOT NULL,
  topic TEXT NOT NULL,
  qos INTEGER NOT NULL,
  payload_json BLOB NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  published_at INTEGER,
  created_at INTEGER NOT NULL,
  UNIQUE(route_id, event_id)
);
```

topicは空文字、先頭`/`、末尾`/`、`+/#`を拒否する。`PutMQTTRoute`は現在のmapping最大`event_row_id`をsnapshotする。enqueueはroute開始後のeventだけを決定的payloadへ変換し、`INSERT OR IGNORE`する。route境界にrevision内で1へ戻る`event_sequence`を使わない。

- [ ] **Step 6: focused testを通してcommitする**

Run: `(cd iotkit-site && go test ./internal/applicationcontract ./internal/store)`

Expected: PASS。

Commit: `feat: add durable semantic mqtt exports`

---

### Task 4: Projector/export loopとCLIを結合する

**Files:**
- Modify: `iotkit-site/internal/mqttsite/client.go`
- Modify: `iotkit-site/internal/mqttsite/client_test.go`
- Modify: `iotkit-site/cmd/iotkit-site/main.go`
- Modify: `iotkit-site/cmd/iotkit-site/main_test.go`
- Modify: `README.md`
- Modify: `docs/architecture.md`

**Interfaces:**
- `mqttsite.Run`は既存record subscriptionに加えてpending application exportをQoS 1で配送する。
- CLIに`mapping-set`, `mapping-list`, `route-add`, `semantic-query`を追加する。

- [ ] **Step 1: PUBACK前後のoutbox testを書く**

```go
func TestExportLoopMarksPublishedOnlyAfterPublishSuccess(t *testing.T) {
    queue := &fakeExportQueue{pending: onePendingExport()}
    err := publishPending(context.Background(), queue, func(topic string, qos byte, payload []byte) error {
        return errors.New("broker unavailable")
    })
    if err == nil || queue.marked != 0 { t.Fatalf("failure marked published") }
    if err := publishPending(context.Background(), queue, successfulPublish); err != nil { t.Fatal(err) }
    if queue.marked != 1 { t.Fatalf("marked = %d", queue.marked) }
}
```

- [ ] **Step 2: testが関数未定義で失敗することを確認する**

Run: `(cd iotkit-site && go test ./internal/mqttsite -run TestExportLoop)`

Expected: FAIL。

- [ ] **Step 3: background convergence loopを実装する**

`serve`起動中は250ms tickerで次を順に行う。

1. `ProjectSemanticEvents(ctx, 256)`
2. `EnqueueMQTTExports(ctx, 256)`
3. `ListPendingMQTTExports(ctx, 256)`
4. Paho `Publish(topic, 1, false, payload)`を実行し、15秒以内のPUBACK成功後だけ`MarkMQTTExportPublished`

project/enqueue/export failureは構造化logへ出してloopを継続する。record batch handlerとaccepted-through publishはこのloopを待たない。

- [ ] **Step 4: CLI parse testを書く**

```go
func TestMappingSetRequiresExplicitTriggerAndActiveValue(t *testing.T) {
    if err := run([]string{"mapping-set", "--db", testDB, "--edge-node-id", "edge-node-01", "--series-key", contactSeries, "--meaning", "production_pulse"}); err == nil {
        t.Fatal("mapping-set accepted implicit trigger defaults")
    }
}
```

- [ ] **Step 5: CLIを実装する**

```text
iotkit-site mapping-set --db site.db --edge-node-id edge-node-01 \
  --series-key '<series_key>' --meaning production_pulse \
  --trigger-mode active_sample --active-value 1

iotkit-site mapping-list --db site.db
iotkit-site route-add --db site.db --mapping-id '<mapping_id>' \
  --topic 'iotkit/v1/application/production-pulses'
iotkit-site semantic-query --db site.db --limit 100
```

成功時は秘密を含まないJSONをstdoutへ出す。設定変更はStoreのtyped methodだけを使い、CLIからSQLを直書きしない。

- [ ] **Step 6: focused integration testを通す**

Run:

```bash
(cd iotkit-site && go test ./cmd/iotkit-site ./internal/mqttsite ./internal/store ./internal/semantic ./internal/applicationcontract)
```

Expected: 全PASS。

- [ ] **Step 7: docsを実挙動へ更新してcommitする**

READMEと`docs/architecture.md`へCLI例、future-only、two-stage failure independence、MQTT payload v1を追記する。

Commit: `feat: run site semantic mqtt pipeline`

---

### Task 5: 最終整合性と全体検証

**Files:**
- Modify only if verification finds a task-scoped defect.

- [ ] **Step 1: stale terminologyを検査する**

Run:

```bash
rg -n "Site Console|production mapping.*YokaKit|pinNumber|ipAddress" \
  README.md docs/architecture.md docs/redesign iotkit-site
```

Expected: 現行YokaKit入力調査や明示的non-goal以外に、今回の境界と矛盾する記述がない。

- [ ] **Step 2: 最終全体検証を1回だけ行う**

Run: `scripts/verify.sh`

Expected: fmt、layer rules、workspace tests、doctests、Clippy `-D warnings`が全PASS。

- [ ] **Step 3: 最終commitを作る**

必要な修正があった場合のみcommitする。
