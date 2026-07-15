# Site Inventory Read Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Edgeから複製したdescriptorとraw measurementを、安定した公開参照、Site固有profile、最新値を持つSite Console向けinventory read modelへ変換する。

**Architecture:** `internal/store`はsource identityを正本として128 bit randomの`device_ref` / `signal_ref`を永続化し、descriptorとraw measurementからinventoryを収束させる。`internal/siteapp`はprofile変更をtyped operationとしてrevision precondition・監査とともに扱い、read側にはsource identityを露出しないsummary型を返す。HTTP/HTMLは次スライスとし、この計画ではAPIが依存できるapplication-service境界までを完成させる。

**Tech Stack:** Go 1.25、`database/sql`、modernc SQLite、標準`crypto/rand` / `encoding/json`、既存Site application service。

## Global Constraints

- raw custodyと`accepted-through`をinventory、descriptor、profile、最新値の失敗から独立させる。
- source identityはdevice `(edge_node_id, system_id)`、signal `(edge_node_id, series_key)`を維持する。
- `device_ref`と`signal_ref`は認可tokenではなく、128 bit randomの安定した公開参照とする。
- profileはdescriptor複製と別tableに置き、descriptor更新・stale化・retireで消さない。
- device profileは`display_name`と`location`、signal profileは`display_name`を必須入力とする。
- profile mutationはSite application serviceのtyped dispatcherを通し、成功監査と同じSQLite transactionでcommitする。
- 最新値はlatest valid measurementだけを採用し、event timeとSite受信時刻を分ける。時刻だけから故障やstaleを断定しない。
- 匿名APIへsource identityやidentifierを出さない契約は後続HTTPスライスで守る。本スライスのsummary型にもsource identityを含めない。
- 実装中はfocused Go testだけを実行し、project全体testとDocker testはPR前の最終ゲートで一度だけ実行する。

---

### Task 1: Durable inventory sourceとmeasurement-first収束

**Files:**
- Modify: `iotkit-site/internal/store/migrations.go`
- Create: `iotkit-site/internal/store/inventory.go`
- Create: `iotkit-site/internal/store/inventory_test.go`
- Modify: `iotkit-site/internal/store/descriptors.go`
- Modify: `iotkit-site/internal/store/descriptors_test.go`
- Modify: `iotkit-site/internal/mqttsite/client.go`
- Modify: `iotkit-site/internal/mqttsite/client_test.go`

**Interfaces:**
- Produces: `Store.ReconcileInventorySources(ctx context.Context, limit int) (int, error)`
- Produces: durable `site_devices` / `site_signals` schema version 4
- Preserves: `Store.AcceptBatch`のsignatureとack behavior

- [x] **Step 1: migrationとstable refの失敗testを書く**

`inventory_test.go`でdescriptorを適用し、同じsourceに対する再適用・DB再open後もrefが変わらないこと、refが`dev_` / `sig_`と32桁hexであることを検査する。

```go
func TestDescriptorInventoryRefsAreStableAcrossReplayAndReopen(t *testing.T) {
    path := filepath.Join(t.TempDir(), "site.db")
    first, err := Open(path)
    if err != nil { t.Fatal(err) }
    snapshot := descriptorFixture(t)
    if _, err := first.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil { t.Fatal(err) }
    firstDeviceRef := testSourceRef(t, first.db, "site_devices", "device_ref")
    firstSignalRef := testSourceRef(t, first.db, "site_signals", "signal_ref")
    if err := first.Close(); err != nil { t.Fatal(err) }

    reopened, err := Open(path)
    if err != nil { t.Fatal(err) }
    t.Cleanup(func() { _ = reopened.Close() })
    reopenedDeviceRef := testSourceRef(t, reopened.db, "site_devices", "device_ref")
    reopenedSignalRef := testSourceRef(t, reopened.db, "site_signals", "signal_ref")
    if reopenedDeviceRef != firstDeviceRef || reopenedSignalRef != firstSignalRef {
        t.Fatalf("refs changed: device=%q signal=%q", reopenedDeviceRef, reopenedSignalRef)
    }
    assertResourceRef(t, firstDeviceRef, "dev_")
    assertResourceRef(t, firstSignalRef, "sig_")
}
```

- [x] **Step 2: testを実行してschemaとquery未実装で失敗することを確認する**

Run: `cd iotkit-site && env GOPATH=/tmp/iotkit-next-go-path GOCACHE=/tmp/iotkit-next-go-cache go test ./internal/store -run 'TestDescriptorInventoryRefsAreStable'`

Expected: `site_devices`または`site_signals` table不在でFAIL。

- [x] **Step 3: schema version 4とref生成helperを実装する**

`migrations.go`へ次を追加する。既存descriptor rowはmigration時にbackfillし、profile tableはTask 2で利用する。

```sql
CREATE TABLE IF NOT EXISTS site_devices (
    device_ref TEXT NOT NULL UNIQUE,
    edge_node_id TEXT NOT NULL,
    system_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (edge_node_id, system_id)
);
CREATE TABLE IF NOT EXISTS site_signals (
    signal_ref TEXT NOT NULL UNIQUE,
    edge_node_id TEXT NOT NULL,
    series_key TEXT NOT NULL,
    system_id TEXT,
    last_received_at INTEGER,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (edge_node_id, series_key)
);
CREATE TABLE IF NOT EXISTS inventory_projection_cursors (
    edge_node_id TEXT NOT NULL,
    ledger_epoch TEXT NOT NULL,
    last_pub_seq INTEGER NOT NULL CHECK(last_pub_seq >= 0),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (edge_node_id, ledger_epoch)
);
CREATE TABLE IF NOT EXISTS signal_current_values (
    edge_node_id TEXT NOT NULL,
    series_key TEXT NOT NULL,
    values_json BLOB NOT NULL CHECK(json_valid(values_json)),
    event_time INTEGER NOT NULL CHECK(event_time >= 0),
    site_received_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (edge_node_id, series_key)
);
CREATE TABLE IF NOT EXISTS device_profiles (
    edge_node_id TEXT NOT NULL,
    system_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    location TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (edge_node_id, system_id)
);
CREATE TABLE IF NOT EXISTS signal_profiles (
    edge_node_id TEXT NOT NULL,
    series_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (edge_node_id, series_key)
);
INSERT OR IGNORE INTO site_devices(device_ref, edge_node_id, system_id, created_at)
SELECT 'dev_' || lower(hex(randomblob(16))), edge_node_id, system_id, updated_at
FROM descriptor_devices;
INSERT OR IGNORE INTO site_signals(signal_ref, edge_node_id, series_key, system_id, created_at)
SELECT 'sig_' || lower(hex(randomblob(16))), edge_node_id, series_key, system_id, updated_at
FROM descriptor_signals;
```

`inventory.go`へ`newResourceRef(prefix string) (string, error)`を置き、`crypto/rand.Read`した16 bytesをhex化する。descriptor適用transactionはsnapshot rowを書き込む前に`ensureDeviceSourceTx` / `ensureSignalSourceTx`を呼び、既存refを変更しない。

- [x] **Step 4: measurement先着placeholderの失敗testを書く**

canonical `series_key`を持つraw batchだけを先に受理し、`ReconcileInventorySources`後にdescriptorなしのdevice/signal sourceが作られることを検査する。非measurement recordと不正な`series_key`はraw受理を妨げずinventory対象外になることも検査する。

```go
func TestReconcileInventorySourcesCreatesMeasurementFirstPlaceholder(t *testing.T) {
    store := openTestStore(t)
    batch := measurementBatch(t, canonicalSeriesKey, 1, 21.5)
    if _, err := store.AcceptBatch(context.Background(), batch); err != nil { t.Fatal(err) }
    count, err := store.ReconcileInventorySources(context.Background(), 100)
    if err != nil { t.Fatal(err) }
    if count != 1 { t.Fatalf("reconciled = %d, want 1", count) }
    if got := testTableCount(t, store.db, "site_signals"); got != 1 {
        t.Fatalf("site signal sources = %d, want 1", got)
    }
    if got := testTableCount(t, store.db, "descriptor_signals"); got != 0 {
        t.Fatalf("descriptor signals = %d, want placeholder only", got)
    }
}
```

- [x] **Step 5: bounded reconciliationを実装しMQTT convergenceへ接続する**

`ReconcileInventorySources`はlimit 1〜1000だけを許可し、`accepted_cursors`と独立した
`inventory_projection_cursors`の差分から、主キーrange queryで最大limit件のraw recordだけを読む。
非measurementと非canonical `series_key`はsourceを作らずcursorだけを進める。canonical measurementは
device/signal source、validな最新値、validityと独立した最終受信時刻、projection cursorを一transactionで
更新し、既存refを変更しない。これによりraw custodyへprojection failureを結合せず、履歴全走査も行わない。

`mqttsite.ExportQueue`へ次を追加し、`convergeExports`の最初に呼ぶ。失敗は`inventory reconciliation failed`として記録するが、semantic projectionとMQTT exportを継続する。

```go
type ExportQueue interface {
    ReconcileInventorySources(context.Context, int) (int, error)
    ProjectSemanticEvents(context.Context, int) (int, error)
    EnqueueMQTTExports(context.Context, int) (int, error)
    ListPendingMQTTExports(context.Context, int) ([]store.PendingMQTTExport, error)
    MarkMQTTExportPublished(context.Context, string) error
}
```

- [x] **Step 6: focused testsを通してcommitする**

Run: `cd iotkit-site && env GOPATH=/tmp/iotkit-next-go-path GOCACHE=/tmp/iotkit-next-go-cache go test ./internal/store ./internal/mqttsite`

Expected: PASS。

Commit: `feat: reconcile site inventory sources`

---

### Task 2: Revision保護されたSite-local profile

**Files:**
- Create: `iotkit-site/internal/store/profiles.go`
- Create: `iotkit-site/internal/store/profiles_test.go`
- Modify: `iotkit-site/internal/siteapp/types.go`
- Modify: `iotkit-site/internal/siteapp/service.go`
- Modify: `iotkit-site/internal/siteapp/service_test.go`

**Interfaces:**
- Produces: `siteapp.DeviceProfile`, `siteapp.SignalProfile`
- Produces: `siteapp.UpdateDeviceProfile`, `siteapp.UpdateSignalProfile`
- Produces: `Store.UpdateDeviceProfile(...)`, `Store.UpdateSignalProfile(...)`
- Consumes: Task 1の`site_devices` / `site_signals`

- [x] **Step 1: profile mutation・revision・監査の失敗testを書く**

```go
func TestUpdateDeviceProfileCommitsRevisionAndAuditAtomically(t *testing.T) {
    store := openTestStore(t)
    applyDescriptor(t, store)
    expected := int64(0)
    profile, err := store.UpdateDeviceProfile(
        context.Background(), siteapp.LocalCLIActor(), deviceRef(t, store),
        siteapp.DeviceProfileInput{DisplayName: "乾燥炉入口", Location: "第2工場・乾燥炉入口"},
        siteapp.RevisionPrecondition{Expected: &expected},
    )
    if err != nil { t.Fatal(err) }
    if profile.Revision != 1 { t.Fatalf("revision = %d", profile.Revision) }
    events, err := store.ListAuditEvents(context.Background(), 10)
    if err != nil { t.Fatal(err) }
    if events[0].Operation != "device_profile.update" || events[0].ResourceRef != profile.DeviceRef {
        t.Fatalf("audit = %#v", events[0])
    }
}
```

同様にsignal profile、誤revisionで`siteapp.ErrRevisionMismatch`、存在しないrefで`siteapp.ErrNotFound`、audit insert trigger失敗時にprofileもrollbackするtestを書く。

- [x] **Step 2: focused testが未定義型・methodで失敗することを確認する**

Run: `cd iotkit-site && env GOPATH=/tmp/iotkit-next-go-path GOCACHE=/tmp/iotkit-next-go-cache go test ./internal/store -run 'TestUpdate(Device|Signal)Profile'`

Expected: profile型または更新method未定義でFAIL。

- [x] **Step 3: profile型、validation、repository mutationを実装する**

```go
type DeviceProfileInput struct {
    DisplayName string
    Location    string
}
type SignalProfileInput struct { DisplayName string }
type DeviceProfile struct {
    DeviceRef   string `json:"device_ref"`
    DisplayName string `json:"display_name"`
    Location    string `json:"location"`
    Revision    int64  `json:"revision"`
    UpdatedAt   int64  `json:"updated_at"`
}
type SignalProfile struct {
    SignalRef   string `json:"signal_ref"`
    DisplayName string `json:"display_name"`
    Revision    int64  `json:"revision"`
    UpdatedAt   int64  `json:"updated_at"`
}
```

表示名はtrim後1〜128 bytes、locationはtrim後1〜256 bytes、control characterなしとする。repositoryはrefからsource identityを解決し、現在revisionへ`RevisionPrecondition`を適用し、upsertと秘密を含まない成功監査を同じtransactionでcommitする。

- [x] **Step 4: typed dispatcherの失敗testを書く**

`UpdateDeviceProfile`と`UpdateSignalProfile`がactor・inputをrepository呼出し前に検証し、`Result.DeviceProfile` / `Result.SignalProfile`を返すことをfake repositoryで検査する。

```go
type UpdateDeviceProfile struct {
    DeviceRef   string
    Input       DeviceProfileInput
    Precondition RevisionPrecondition
}
func (UpdateDeviceProfile) isSiteOperation() {}

type UpdateSignalProfile struct {
    SignalRef   string
    Input       SignalProfileInput
    Precondition RevisionPrecondition
}
func (UpdateSignalProfile) isSiteOperation() {}
```

- [x] **Step 5: dispatcherとrepository interfaceを実装する**

`Service.Dispatch`のtype switchへ2 operationを追加する。source identityをoperationやResultへ入れず、resource refだけを境界にする。

- [x] **Step 6: focused testsを通してcommitする**

Run: `cd iotkit-site && env GOPATH=/tmp/iotkit-next-go-path GOCACHE=/tmp/iotkit-next-go-cache go test ./internal/siteapp ./internal/store`

Expected: PASS。

Commit: `feat: manage site inventory profiles`

---

### Task 3: Site Console向けinventory read model

**Files:**
- Modify: `iotkit-site/internal/siteapp/types.go`
- Modify: `iotkit-site/internal/siteapp/service.go`
- Modify: `iotkit-site/internal/siteapp/service_test.go`
- Modify: `iotkit-site/internal/store/inventory.go`
- Modify: `iotkit-site/internal/store/inventory_test.go`

**Interfaces:**
- Produces: `Service.ListDevices(ctx, page)` / `Service.ListSignals(ctx, page)`
- Produces: `siteapp.DeviceSummary`, `siteapp.SignalSummary`, `siteapp.LatestMeasurement`, `siteapp.PageRequest`
- Consumes: Task 1 source refsとTask 2 profile

- [x] **Step 1: source identityを露出しない一覧型とpaginationの失敗testを書く**

```go
type PageRequest struct {
    Limit    int
    AfterRef string
}
type LatestMeasurement struct {
    Values         json.RawMessage `json:"values"`
    EventTime      int64           `json:"event_time"`
    SiteReceivedAt int64           `json:"site_received_at"`
}
type DeviceSummary struct {
    DeviceRef     string         `json:"device_ref"`
    DisplayName   string         `json:"display_name"`
    Location      string         `json:"location"`
    ProfileRevision *int64       `json:"profile_revision"`
    DescriptorPresence string    `json:"descriptor_presence"`
    DeviceState   string         `json:"device_state"`
    LastReceivedAt *int64        `json:"last_received_at"`
}
type SignalSummary struct {
    SignalRef     string             `json:"signal_ref"`
    DeviceRef     *string            `json:"device_ref"`
    DisplayName   string             `json:"display_name"`
    ProfileRevision *int64           `json:"profile_revision"`
    DescriptorPresence string        `json:"descriptor_presence"`
    Unit          *string            `json:"unit"`
    ValueType     *string            `json:"value_type"`
    Latest        *LatestMeasurement `json:"latest"`
    LastReceivedAt *int64            `json:"last_received_at"`
    HasSemanticMapping bool          `json:"has_semantic_mapping"`
}
```

testはref昇順、`AfterRef` exclusive、limit 1〜100、未設定profileは空文字とnil revision、summaryをJSON化しても`edge_node_id`、`system_id`、`series_key`、`identifier`が存在しないことを検査する。

- [x] **Step 2: testがread method未実装で失敗することを確認する**

Run: `cd iotkit-site && env GOPATH=/tmp/iotkit-next-go-path GOCACHE=/tmp/iotkit-next-go-cache go test ./internal/store -run 'TestListInventory(Device|Signal)'`

Expected: read model methodまたはsummary型未定義でFAIL。

- [x] **Step 3: descriptor・profile・mappingを結合するbounded queryを実装する**

`ListInventoryDevices` / `ListInventorySignals`は`ref > afterRef ORDER BY ref LIMIT ?`を使う。descriptorがないsourceはpresence `unknown`、signalからdeviceへ解決できない場合は`DeviceRef = nil`とする。device最終受信時刻はmaterializedされたsignalの`last_received_at`最大値をset-wiseに返し、一覧queryからraw履歴を走査しない。

- [x] **Step 4: latest valid measurement選択の失敗testを書く**

同一signalへ古いvalid measurement、より新しいmalformed measurement、最新のvalid measurementを保存し、read modelが最新validだけを返すことを検査する。descriptor先着では`Latest=nil`、measurement先着ではpresence `unknown`と最新値が返ることも検査する。

```go
func TestListInventorySignalsUsesLatestValidMeasurement(t *testing.T) {
    store := openTestStore(t)
    applyDescriptor(t, store)
    acceptMeasurements(t, store,
		validMeasurement(1, canonicalSeriesKey, []float64{20.0}, 1000),
		validMeasurement(2, canonicalSeriesKey, []float64{21.5}, 2000),
		malformedMeasurement(3, canonicalSeriesKey),
    )
    signals, err := store.ListInventorySignals(context.Background(), 10, "")
    if err != nil { t.Fatal(err) }
    if string(signals[0].Latest.Values) != `[21.5]` || signals[0].Latest.EventTime != 2000 {
        t.Fatalf("latest = %#v", signals[0].Latest)
    }
}
```

- [x] **Step 5: bounded latest-value decoderを実装する**

bounded projection内で`family == "measurement"`、非空の有限number配列`values`、非負integer
`event_time`を満たすrecordだけを`signal_current_values`へmaterializeする。不正なmeasurementは最新値を
上書きしないが、sourceの`last_received_at`は更新する。一覧はこのtableをjoinするだけとし、measurement時刻から
descriptor presenceや故障状態を推測しない。

- [x] **Step 6: application service read境界を実装する**

Repositoryへ次を追加し、`Service.ListDevices` / `ListSignals`はlimitを1〜100で先に検証して委譲する。

```go
ListInventoryDevices(context.Context, int, string) ([]DeviceSummary, error)
ListInventorySignals(context.Context, int, string) ([]SignalSummary, error)
```

fake repository testでinvalid limit時にrepositoryが呼ばれないことと、返却型にsource identityがないことを確認する。

- [x] **Step 7: focused testsを通し、設計書の実装状態を更新してcommitする**

`docs/superpowers/specs/2026-07-15-site-console-api-design.md`のdescriptor実装状態を「public ref、profile、current-value read modelまで実装済み。HTTP APIとHTMLは後続スライス」に更新する。

Run: `cd iotkit-site && env GOPATH=/tmp/iotkit-next-go-path GOCACHE=/tmp/iotkit-next-go-cache go test ./internal/siteapp ./internal/store ./internal/mqttsite`

Expected: PASS。

Commit: `feat: expose site inventory read model`

---

## Plan Self-Review

- Scope: HTTP、settings auth、HTML、output candidateは含めず、匿名一覧APIが必要とするinventory境界だけに限定した。
- Custody: raw受理transactionへinventory更新を結合せず、独立した収束処理にした。
- Identity: application serviceの公開型はopaque refだけを持ち、source identityとidentifierを含まない。
- Mutation: profile更新はtyped dispatcher、revision precondition、同一transaction監査を必須にした。
- Measurement-first: descriptorが未到着でもplaceholder signalと最新値を作れ、後着descriptorで同じrefをenrichする。
- Bounded work: raw recordはdurable projection cursorにより最大1回処理し、一覧はraw履歴を走査しない。
- Verification: 実装中はfocused Go tests、全体/Docker/Piはこの内部スライスでは繰り返さない。
