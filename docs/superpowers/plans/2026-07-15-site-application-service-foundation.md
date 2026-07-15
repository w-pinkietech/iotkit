# Site Application Service Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Siteのsemantic mapping変更を、SQLite migration、同一transaction監査、typed application-service dispatcherを通る一つの経路へ集約する。

**Architecture:** `internal/siteapp`がoperation、actor、precondition、dispatcherを所有し、CLIはこのserviceだけを呼ぶ。`internal/store`はapplication serviceへ注入するrepositoryを実装し、future-only mapping境界と成功監査を同じSQLite transactionでcommitする。既存のraw custody、semantic projection、MQTT exportの挙動とDBデータは維持する。

**Tech Stack:** Go 1.25、`database/sql`、modernc SQLite、既存`semantic` package、標準`flag` CLI。

## Global Constraints

- 変更系操作はSite application service内のtyped operation dispatcherを経由する。CLIからStoreやSQLへ直接mutationしない。
- mapping作成、改訂、無効化はfuture-onlyとし、現在の全`accepted_cursors`を同一transactionで境界へ固定する。
- 成功mutationと成功監査は同じSQLite transactionでcommitし、片方だけを残さない。
- 監査へcredential、passphrase、raw payloadを保存しない。
- `local_cli` actorは個人識別を主張せず、固定の非秘密actor classとして記録する。
- 既存raw record、cursor、semantic mapping/event、MQTT route/outboxをmigrationで削除または再作成しない。
- HTTP API、settings認証、profile、descriptor、output revision、Site Console HTMLはこの実装単位へ含めない。
- focused Go testだけを実行する。project全体testとDocker testはSite Console実装完了後のpre-PR gateで一度実行する。

---

### Task 1: Site SQLite migration runnerを導入する

**Files:**
- Create: `iotkit-site/internal/store/migrations.go`
- Create: `iotkit-site/internal/store/migrations_test.go`
- Modify: `iotkit-site/internal/store/store.go`

**Interfaces:**
- Produces: `applyMigrations(ctx context.Context, db *sql.DB) error`
- Produces: schema version 1（既存table）とversion 2（`audit_events`）
- Preserves: `Store.Open(path)`の公開signature

- [ ] **Step 1: 既存DBを再openしてもデータが残り、schema versionが進む失敗testを書く**

```go
func TestOpenAppliesMigrationsWithoutDroppingExistingData(t *testing.T) {
    path := filepath.Join(t.TempDir(), "site.db")
    first, err := Open(path)
    if err != nil { t.Fatal(err) }
    if _, err := first.AcceptBatch(context.Background(), testBatch(t)); err != nil { t.Fatal(err) }
    if _, err := first.db.Exec("PRAGMA user_version = 0"); err != nil { t.Fatal(err) }
    if err := first.Close(); err != nil { t.Fatal(err) }

    reopened, err := Open(path)
    if err != nil { t.Fatal(err) }
    t.Cleanup(func() { _ = reopened.Close() })
    records, err := reopened.ListRawRecords(context.Background(), 10)
    if err != nil { t.Fatal(err) }
    if len(records) != 1 { t.Fatalf("records = %d, want 1", len(records)) }
    var version int
    if err := reopened.db.QueryRow("PRAGMA user_version").Scan(&version); err != nil { t.Fatal(err) }
    if version != 2 { t.Fatalf("schema version = %d, want 2", version) }
}
```

- [ ] **Step 2: testがschema version 0のままで失敗することを確認する**

Run: `cd iotkit-site && go test ./internal/store -run TestOpenAppliesMigrationsWithoutDroppingExistingData`

Expected: `schema version = 0, want 2`でFAIL。

- [ ] **Step 3: version付きmigration runnerを実装する**

`migrations.go`へ次の骨格を置き、現在`initialize`にある`CREATE TABLE IF NOT EXISTS`群をversion 1のSQLへそのまま移す。version 2はTask 2で使う監査tableを作る。

```go
type migration struct {
    version int
    sql     string
}

var schemaMigrations = []migration{
    {version: 1, sql: `
        CREATE TABLE IF NOT EXISTS raw_records (
            edge_node_id TEXT NOT NULL,
            ledger_epoch TEXT NOT NULL,
            pub_seq INTEGER NOT NULL,
            publication_id TEXT NOT NULL,
            record_json BLOB NOT NULL,
            record_sha256 BLOB NOT NULL,
            received_at INTEGER NOT NULL,
            PRIMARY KEY (edge_node_id, ledger_epoch, pub_seq)
        );
        CREATE TABLE IF NOT EXISTS accepted_cursors (
            edge_node_id TEXT NOT NULL,
            ledger_epoch TEXT NOT NULL,
            accepted_through INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (edge_node_id, ledger_epoch)
        );
        CREATE TABLE IF NOT EXISTS semantic_mappings (
            mapping_id TEXT NOT NULL,
            revision INTEGER NOT NULL,
            edge_node_id TEXT NOT NULL,
            series_key TEXT NOT NULL,
            meaning TEXT NOT NULL CHECK(meaning = 'production_pulse'),
            trigger_mode TEXT NOT NULL CHECK(trigger_mode IN ('active_sample', 'active_edge')),
            active_value INTEGER NOT NULL CHECK(active_value IN (0, 1)),
            active INTEGER NOT NULL CHECK(active IN (0, 1)),
            created_at INTEGER NOT NULL,
            PRIMARY KEY (mapping_id, revision)
        );
        CREATE UNIQUE INDEX IF NOT EXISTS ux_semantic_one_active_per_source
            ON semantic_mappings(edge_node_id, series_key) WHERE active = 1;
        CREATE TABLE IF NOT EXISTS semantic_mapping_starts (
            mapping_id TEXT NOT NULL,
            mapping_revision INTEGER NOT NULL,
            ledger_epoch TEXT NOT NULL,
            start_after_pub_seq INTEGER NOT NULL,
            PRIMARY KEY (mapping_id, mapping_revision, ledger_epoch)
        );
        CREATE TABLE IF NOT EXISTS semantic_mapping_ends (
            mapping_id TEXT NOT NULL,
            mapping_revision INTEGER NOT NULL,
            ledger_epoch TEXT NOT NULL,
            end_at_pub_seq INTEGER NOT NULL,
            PRIMARY KEY (mapping_id, mapping_revision, ledger_epoch)
        );
        CREATE TABLE IF NOT EXISTS semantic_mapping_state (
            mapping_id TEXT NOT NULL,
            mapping_revision INTEGER NOT NULL,
            last_value INTEGER,
            next_event_sequence INTEGER NOT NULL,
            PRIMARY KEY (mapping_id, mapping_revision)
        );
        CREATE TABLE IF NOT EXISTS semantic_results (
            mapping_id TEXT NOT NULL,
            mapping_revision INTEGER NOT NULL,
            ledger_epoch TEXT NOT NULL,
            pub_seq INTEGER NOT NULL,
            emitted_event_id TEXT,
            PRIMARY KEY (mapping_id, mapping_revision, ledger_epoch, pub_seq)
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
            UNIQUE (mapping_id, mapping_revision, event_sequence)
        );
        CREATE TABLE IF NOT EXISTS mqtt_routes (
            route_id TEXT PRIMARY KEY,
            mapping_id TEXT NOT NULL,
            topic TEXT NOT NULL,
            qos INTEGER NOT NULL CHECK(qos = 1),
            start_after_event_row_id INTEGER NOT NULL,
            active INTEGER NOT NULL CHECK(active IN (0, 1)),
            created_at INTEGER NOT NULL,
            UNIQUE (mapping_id, topic)
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
            UNIQUE (route_id, event_id)
        );
    `},
    {version: 2, sql: `
        CREATE TABLE IF NOT EXISTS audit_events (
            audit_row_id INTEGER PRIMARY KEY AUTOINCREMENT,
            occurred_at INTEGER NOT NULL,
            actor_class TEXT NOT NULL CHECK(actor_class IN ('local_cli', 'settings_session', 'system')),
            actor_ref TEXT NOT NULL,
            operation TEXT NOT NULL,
            resource_ref TEXT NOT NULL,
            outcome TEXT NOT NULL CHECK(outcome IN ('success', 'failure')),
            summary_json BLOB NOT NULL CHECK(json_valid(summary_json))
        );
    `},
}
```

`applyMigrations`は`PRAGMA user_version`を読み、未適用migrationごとにtransactionを開始し、SQL実行と`PRAGMA user_version = N`を同じtransactionでcommitする。未知の将来versionは拒否する。

- [ ] **Step 4: `Store.initialize`をDB設定とmigration適用へ分離する**

```go
func (store *Store) initialize() error {
    if err := store.rejectLegacyGatewaySchema(); err != nil { return err }
    if _, err := store.db.Exec(`
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = FULL;
        PRAGMA foreign_keys = ON;
    `); err != nil { return err }
    return applyMigrations(context.Background(), store.db)
}
```

既存`gateway_identity`検査は`rejectLegacyGatewaySchema`へ移すだけで挙動を変えない。

- [ ] **Step 5: focused testを通してcommitする**

Run: `cd iotkit-site && go test ./internal/store`

Expected: PASS。

Commit: `refactor: add site schema migrations`

---

### Task 2: 監査付きfuture-only mapping repositoryを実装する

**Files:**
- Create: `iotkit-site/internal/siteapp/types.go`
- Create: `iotkit-site/internal/store/audit.go`
- Create: `iotkit-site/internal/store/audit_test.go`
- Modify: `iotkit-site/internal/store/store.go`
- Modify: `iotkit-site/internal/store/store_test.go`
- Modify: `iotkit-site/internal/mqttsite/processor_test.go`
- Modify: `iotkit-site/cmd/iotkit-site/main_test.go`

**Interfaces:**
- Consumes: `semantic.MappingSpec`, `semantic.Mapping`
- Produces: `siteapp.Actor`, `siteapp.RevisionPrecondition`, `siteapp.AuditEvent`
- Produces: `Store.ApplySemanticMapping(ctx, actor, spec, precondition)`
- Produces: `Store.DeactivateSemanticMapping(ctx, actor, edgeNodeID, seriesKey, precondition)`
- Produces: `Store.ListAuditEvents(ctx, limit)`

- [ ] **Step 1: actor、precondition、audit read型を定義する**

```go
package siteapp

type ActorClass string

const (
    ActorLocalCLI        ActorClass = "local_cli"
    ActorSettingsSession ActorClass = "settings_session"
    ActorSystem          ActorClass = "system"
)

type Actor struct {
    Class ActorClass
    Ref   string
}

func LocalCLIActor() Actor { return Actor{Class: ActorLocalCLI, Ref: "local_cli"} }

type RevisionPrecondition struct {
    Expected *int64
}

type AuditEvent struct {
    AuditRowID int64          `json:"audit_row_id"`
    OccurredAt int64          `json:"occurred_at"`
    ActorClass ActorClass     `json:"actor_class"`
    ActorRef   string         `json:"actor_ref"`
    Operation  string         `json:"operation"`
    ResourceRef string        `json:"resource_ref"`
    Outcome    string         `json:"outcome"`
    Summary    json.RawMessage `json:"summary"`
}
```

`Actor.Validate`は既知class、空でないref、最大128 bytes、control characterなしを要求する。

- [ ] **Step 2: mapping変更と監査のatomicityを示す失敗testを書く**

```go
func TestApplySemanticMappingCommitsAuditWithFutureOnlyRevision(t *testing.T) {
    store := openTestStore(t)
    acceptEpoch(t, store, "edge-node-01", "epoch-a", 3, 1)
    mapping, err := store.ApplySemanticMapping(context.Background(), siteapp.LocalCLIActor(), semantic.MappingSpec{
        EdgeNodeID: "edge-node-01", SeriesKey: contactSeries,
        Meaning: semantic.MeaningProductionPulse,
        TriggerMode: semantic.TriggerActiveEdge, ActiveValue: 1,
    }, siteapp.RevisionPrecondition{})
    if err != nil { t.Fatal(err) }
    events, err := store.ListAuditEvents(context.Background(), 10)
    if err != nil { t.Fatal(err) }
    if len(events) != 1 || events[0].Operation != "semantic_mapping.put" || events[0].ResourceRef != mapping.ID {
        t.Fatalf("audit events = %#v", events)
    }
    if got := store.testMappingStarts(t, mapping.ID, mapping.Revision); !reflect.DeepEqual(got, map[string]int64{"epoch-a": 3}) {
        t.Fatalf("mapping starts = %#v", got)
    }
}

func TestApplySemanticMappingRollsBackWhenAuditInsertFails(t *testing.T) {
    store := openTestStore(t)
    if _, err := store.db.Exec(`CREATE TRIGGER fail_audit BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT, 'fail'); END;`); err != nil { t.Fatal(err) }
    _, err := store.ApplySemanticMapping(context.Background(), siteapp.LocalCLIActor(), validMappingSpec(), siteapp.RevisionPrecondition{})
    if err == nil { t.Fatal("mapping mutation succeeded despite audit failure") }
    if got := store.testCount(t, "semantic_mappings"); got != 0 { t.Fatalf("mapping count = %d, want 0", got) }
}
```

- [ ] **Step 3: mapping SQLをtransaction helperへ分け、成功監査を同じtransactionへinsertする**

```go
func (store *Store) ApplySemanticMapping(
    ctx context.Context,
    actor siteapp.Actor,
    spec semantic.MappingSpec,
    precondition siteapp.RevisionPrecondition,
) (semantic.Mapping, error)
```

既存`PutSemanticMapping`本体を`putSemanticMappingTx(ctx, tx, spec, precondition)`へ移す。現在revisionが`precondition.Expected`と一致しなければsentinel `siteapp.ErrRevisionMismatch`を返す。`Expected == 0`はmapping未作成を表し、`Expected == nil`はlocal recovery/既存CLI用の明示的な無条件操作とする。mapping insert後にoperation `semantic_mapping.put`、resource ref `mapping_id`、秘密を含まないmeaning/trigger/active value/revisionだけをJSON summaryへ入れてからcommitする。

- [ ] **Step 4: 無効化のfuture-only境界と監査testを書く**

```go
func TestDeactivateSemanticMappingClosesCurrentRevisionAndAudits(t *testing.T) {
    store := openTestStore(t)
    mapping := applyMapping(t, store)
    acceptEpoch(t, store, "edge-node-01", "epoch-a", 4, 0)
    inactive, err := store.DeactivateSemanticMapping(
        context.Background(), siteapp.LocalCLIActor(), mapping.EdgeNodeID, mapping.SeriesKey,
        siteapp.RevisionPrecondition{Expected: &mapping.Revision},
    )
    if err != nil { t.Fatal(err) }
    if inactive.Active { t.Fatal("mapping remains active") }
    if got := store.testMappingEnds(t, mapping.ID, mapping.Revision); !reflect.DeepEqual(got, map[string]int64{"epoch-a": 4}) {
        t.Fatalf("mapping ends = %#v", got)
    }
    events, err := store.ListAuditEvents(context.Background(), 10)
    if err != nil { t.Fatal(err) }
    if events[len(events)-1].Operation != "semantic_mapping.deactivate" { t.Fatalf("audit = %#v", events) }
}
```

- [ ] **Step 5: `DeactivateSemanticMapping`とbounded audit queryを実装する**

無効化はactive mappingを取得し、現在の当該Edge全cursorを`semantic_mapping_ends`へsnapshotしてから`active = 0`へ更新し、成功監査をinsertしてcommitする。active mapping不在は`siteapp.ErrNotFound`、revision不一致は`siteapp.ErrRevisionMismatch`にする。`ListAuditEvents`はlimit 1〜100だけを受理し、`audit_row_id DESC`で返す。

- [ ] **Step 6: repositoryを使う既存test helperを更新してfocused testを通す**

既存product codeから`Store.PutSemanticMapping`を削除する。store/mqttsite/CLI test fixtureは、各testの既存`semantic.MappingSpec`を保ったまま`Store.ApplySemanticMapping(ctx, siteapp.LocalCLIActor(), spec, siteapp.RevisionPrecondition{})`へ置換する。

Run: `cd iotkit-site && go test ./internal/store ./internal/mqttsite ./cmd/iotkit-site`

Expected: PASS。

Commit: `feat: audit site semantic mutations`

---

### Task 3: typed application-service dispatcherを実装する

**Files:**
- Create: `iotkit-site/internal/siteapp/service.go`
- Create: `iotkit-site/internal/siteapp/service_test.go`

**Interfaces:**
- Consumes repository methods from Task 2
- Produces: `siteapp.Operation` sealed interface
- Produces: `siteapp.PutSemanticMapping`, `siteapp.DeactivateSemanticMapping`
- Produces: `siteapp.Result`
- Produces: `Service.Dispatch(ctx, actor, operation)`
- Produces: `Service.ListSemanticMappings(ctx)` and `Service.ListAuditEvents(ctx, limit)`

- [ ] **Step 1: dispatcherがactorとoperationを検証する失敗testを書く**

```go
func TestDispatchValidatesActorBeforeRepositoryMutation(t *testing.T) {
    repo := &fakeRepository{}
    service := NewService(repo)
    _, err := service.Dispatch(context.Background(), Actor{}, PutSemanticMapping{Spec: validSpec()})
    if err == nil { t.Fatal("empty actor was accepted") }
    if repo.applyCalls != 0 { t.Fatalf("repository calls = %d", repo.applyCalls) }
}

func TestDispatchRoutesPutAndDeactivateOperations(t *testing.T) {
    repo := &fakeRepository{mapping: semantic.Mapping{ID: "sm-01", Revision: 1, Active: true}}
    service := NewService(repo)
    put, err := service.Dispatch(context.Background(), LocalCLIActor(), PutSemanticMapping{Spec: validSpec()})
    if err != nil || put.SemanticMapping == nil { t.Fatalf("put result = %#v, err = %v", put, err) }
    deactivate, err := service.Dispatch(context.Background(), LocalCLIActor(), DeactivateSemanticMapping{EdgeNodeID: "edge-node-01", SeriesKey: "series-01"})
    if err != nil || deactivate.SemanticMapping == nil { t.Fatalf("deactivate result = %#v, err = %v", deactivate, err) }
}
```

- [ ] **Step 2: testがdispatcher未定義で失敗することを確認する**

Run: `cd iotkit-site && go test ./internal/siteapp`

Expected: `NewService`またはoperation型未定義でFAIL。

- [ ] **Step 3: sealed operationとdispatcherを実装する**

```go
type Operation interface{ isSiteOperation() }

type PutSemanticMapping struct {
    Spec         semantic.MappingSpec
    Precondition RevisionPrecondition
}
func (PutSemanticMapping) isSiteOperation() {}

type DeactivateSemanticMapping struct {
    EdgeNodeID   string
    SeriesKey    string
    Precondition RevisionPrecondition
}
func (DeactivateSemanticMapping) isSiteOperation() {}

type Result struct {
    SemanticMapping *semantic.Mapping
}

type Repository interface {
    ApplySemanticMapping(context.Context, Actor, semantic.MappingSpec, RevisionPrecondition) (semantic.Mapping, error)
    DeactivateSemanticMapping(context.Context, Actor, string, string, RevisionPrecondition) (semantic.Mapping, error)
    ListSemanticMappings(context.Context) ([]semantic.Mapping, error)
    ListAuditEvents(context.Context, int) ([]AuditEvent, error)
}

type Service struct{ repository Repository }
```

`Dispatch`はactor検証後、type switchで2 operationだけを受理する。putは`Spec.Validate`、deactivateは空identityとcontrol characterを検証してからrepositoryへ渡す。未知operationは外部packageから実装できないsealed interfaceのためcompile時に防ぐ。

- [ ] **Step 4: read methodがrepositoryへ委譲し、limitを先に検証するtestを書く**

```go
func TestListAuditEventsRejectsUnboundedLimit(t *testing.T) {
    repo := &fakeRepository{}
    service := NewService(repo)
    if _, err := service.ListAuditEvents(context.Background(), 101); err == nil { t.Fatal("limit 101 was accepted") }
    if repo.auditCalls != 0 { t.Fatalf("repository calls = %d", repo.auditCalls) }
}
```

- [ ] **Step 5: read serviceを実装してfocused testを通す**

Run: `cd iotkit-site && go test ./internal/siteapp ./internal/store`

Expected: PASS。

Commit: `feat: add site operation dispatcher`

---

### Task 4: 既存CLIをapplication serviceへ移す

**Files:**
- Modify: `iotkit-site/cmd/iotkit-site/main.go`
- Modify: `iotkit-site/cmd/iotkit-site/main_test.go`
- Modify: `docs/architecture.md`
- Modify: `docs/superpowers/specs/2026-07-15-site-console-api-design.md`

**Interfaces:**
- Consumes: `siteapp.NewService(store)` and `Service.Dispatch`
- Produces: existing `mapping-set` behavior through typed dispatch
- Produces: `mapping-deactivate --edge-node-id --series-key`
- Preserves: existing `mapping-list`, raw query, route, semantic query output

- [ ] **Step 1: CLI mapping変更が監査される失敗testを書く**

```go
func TestMappingSetAndDeactivateUseAuditedApplicationService(t *testing.T) {
    dbPath := filepath.Join(t.TempDir(), "site.db")
    setArgs := []string{
        "mapping-set", "--db", dbPath,
        "--edge-node-id", "edge-node-01", "--series-key", "contact-series-01",
        "--meaning", "production_pulse", "--trigger-mode", "active_edge", "--active-value", "1",
    }
    if err := run(setArgs); err != nil { t.Fatal(err) }
    if err := run([]string{"mapping-deactivate", "--db", dbPath, "--edge-node-id", "edge-node-01", "--series-key", "contact-series-01"}); err != nil { t.Fatal(err) }

    archive, err := store.Open(dbPath)
    if err != nil { t.Fatal(err) }
    defer archive.Close()
    events, err := archive.ListAuditEvents(context.Background(), 10)
    if err != nil { t.Fatal(err) }
    if len(events) != 2 { t.Fatalf("audit events = %d, want 2", len(events)) }
}
```

- [ ] **Step 2: testがunknown `mapping-deactivate`で失敗することを確認する**

Run: `cd iotkit-site && go test ./cmd/iotkit-site -run TestMappingSetAndDeactivateUseAuditedApplicationService`

Expected: `unknown command "mapping-deactivate"`でFAIL。

- [ ] **Step 3: CLI composition helperとmapping command adapterを実装する**

```go
func openSiteService(dbPath string) (*siteapp.Service, *store.Store, error) {
    archive, err := store.Open(dbPath)
    if err != nil { return nil, nil, err }
    return siteapp.NewService(archive), archive, nil
}
```

`runMappingSet`はflagと`semantic.MappingSpec`を組み立て、`service.Dispatch(ctx, siteapp.LocalCLIActor(), siteapp.PutSemanticMapping{Spec: spec})`だけを呼ぶ。`runMappingDeactivate`も同じ経路で`DeactivateSemanticMapping`をdispatchする。`runMappingList`は`service.ListSemanticMappings`へ移す。CLI adapterから`archive.ApplySemanticMapping`、`archive.DeactivateSemanticMapping`を直接呼ばない。

- [ ] **Step 4: usage、architecture、spec statusを実挙動へ更新する**

usageへ`mapping-deactivate`を追加する。`docs/architecture.md`のSite CLI説明を「typed Store operation」から「Site application-service dispatcher」へ修正し、mapping set/deactivateが同一transaction監査を持つことを書く。設計書のstatusを`Approved; implementation in progress`へ変更する。

- [ ] **Step 5: focused testと境界検査を通してcommitする**

Run:

```bash
cd iotkit-site && go test ./cmd/iotkit-site ./internal/siteapp ./internal/store ./internal/mqttsite
rg -n "PutSemanticMapping|ApplySemanticMapping|DeactivateSemanticMapping" cmd internal
```

Expected: Go testがPASSし、product CLIのmutation呼出は`siteapp.Service.Dispatch`だけ。Store mutation methodのproduct callerはdispatcher経由だけ。

Commit: `refactor: route site cli mutations through service`

---

### Task 5: 第1段階の整合性を検証する

**Files:**
- No product file changes are expected.

- [ ] **Step 1: formatterとfocused package testを実行する**

Run:

```bash
cd iotkit-site && gofmt -w internal/store/migrations.go internal/store/migrations_test.go internal/store/audit.go internal/store/audit_test.go internal/siteapp/types.go internal/siteapp/service.go internal/siteapp/service_test.go cmd/iotkit-site/main.go cmd/iotkit-site/main_test.go
cd iotkit-site && go test ./cmd/iotkit-site ./internal/siteapp ./internal/store ./internal/mqttsite ./internal/semantic ./internal/applicationcontract
```

Expected: PASS。

- [ ] **Step 2: migration、監査、mutation境界を静的確認する**

Run:

```bash
rg -n "PRAGMA user_version|audit_events|semantic_mapping.(put|deactivate)" iotkit-site/internal
rg -n "ApplySemanticMapping|DeactivateSemanticMapping" iotkit-site/cmd iotkit-site/internal
git diff --check
```

Expected: version 1/2 migration、2つの監査operation、Store実装とsiteapp dispatcherだけが見つかり、CLIからStore mutation直呼びがない。`git diff --check`が無出力で成功する。

- [ ] **Step 3: planと設計のセルフレビューを行う**

設計書のApplication service、Migration、Audit、future-only mapping節を読み直し、この第1段階で対象とした要件がtestへ対応していることを確認する。HTTP認証、descriptor、profile、preview、output revision、HTML/Caddyはこのplanの対象外であり、後続の独立した実装単位として残っていることを確認する。

- [ ] **Step 4: verificationで修正が発生した場合だけ修正commitを作る**

Commit: `fix: close site service foundation gaps`
