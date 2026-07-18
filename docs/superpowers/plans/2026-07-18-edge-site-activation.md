# Edge–Site Activation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Brokerへ接続済みのEdgeをSite Consoleで発見・初回activationし、activation後の観測だけを既存のMQTT custody契約でSiteへ保存できるようにする。

**Architecture:** Edgeは登録前観測を`readings`へ保存するが`publication_log`へ採番せず、Siteが永続化したactivation requestを受けて将来のingestだけをpublication admissionする。Siteはmatching resultをcommitして`active`になった後だけ、activation検査・raw保存・cursor更新を同じtransactionで行う。既存の`RecordBatch`、`AcceptedThrough`、raw identityとpost-activation purge権威は変更しない。

**Tech Stack:** Rust 2024、tokio、rusqlite、rumqttc、Go、SQLite、Paho MQTT、HTML templates、Docker Mosquitto

## Global Constraints

- activation request/resultはMQTT QoS 1、non-retainedとし、PUBACKをapplication完了に使わない。
- 未登録Edgeはdescriptorだけを送信し、recordsを送らず、全record-family enqueueを同じdurable gateで止める。
- 初回activationは`publication_log`全体未使用、SQLite allocation sequence未使用、target cursor 0の新規streamだけに許可する。
- 同じactivation IDは同じ境界・同じresultを返し、別ID、別Site、別epochをfail closedで拒否する。
- Siteは`active`以外のrecordsをrawへ保存せず、accepted-throughを返さない。
- activation前prefixの境界は一度だけ固定し、物理削除は境界を変えない再開可能なEdge-local cleanupとする。
- 既存のaccepted-throughだけがpost-activation official outboxのcursorとpurge eligibilityを進める。
- 既存の未コミットConsole変更を保持し、activation以外の表示・設定挙動を退行させない。
- v1は初回activationだけを実装し、deactivation、reactivation、Site transfer、ID reuse、既存standalone outbox adoptionを実装しない。

---

### Task 0: Preserve and verify the current Console baseline

**Files:**
- Existing uncommitted `iotkit-site/` Console and onboarding files
- Existing untracked Console plan/spec files

**Interfaces:**
- Consumes: current working-tree Console implementation
- Produces: a clean committed baseline before activation changes

- [ ] **Step 1: Inspect the current diff and confirm it contains only the already requested Console/onboarding work**

Run:

```bash
git diff -- iotkit-site docs/superpowers
git status --short
```

Expected: no activation implementation and no unrelated destructive changes.

- [ ] **Step 2: Run the existing Site test suite**

Run:

```bash
go test ./...
```

Working directory: `iotkit-site`

Expected: PASS.

- [ ] **Step 3: Commit only the existing Console/onboarding baseline**

```bash
git add iotkit-site docs/superpowers/specs/2026-07-18-site-console-operator-journey-design.md docs/superpowers/plans/2026-07-18-site-console-*.md
git commit -m "fix(site): complete sensor onboarding console"
```

---

### Task 1: Add the Edge activation state and wire contract

**Files:**
- Create: `core/publish/migrations/0020_site_activation.sql`
- Create: `core/publish/src/activation.rs`
- Modify: `core/publish/src/lib.rs`
- Modify: `core/publish/src/mqtt.rs`
- Modify: `core/publish/src/store.rs`
- Test: `core/publish/src/activation.rs`
- Test: `core/publish/src/mqtt.rs`
- Test: `core/publish/src/store.rs`

**Interfaces:**
- Produces: `ActivationRequest`, `ActivationResult`, `ActivationState`, `apply_activation`, `publication_admitted`, `cleanup_pre_activation_batch`
- Consumes: `ledger_epoch`, `publication_log`, `target_registry`, `readings`

- [ ] **Step 1: Add failing migration and state-machine tests**

Tests must prove:

```rust
assert_eq!(activation_state(&conn)?, ActivationState::Standalone);
install_new_site_target(&conn)?;
assert_eq!(activation_state(&conn)?, ActivationState::DiscoveryOnly);

let result = apply_activation(&conn, request.clone(), 1000)?;
assert_eq!(result.first_publication_seq, 1);
assert_eq!(result.discard_through_reading_seq, 2);
assert_eq!(apply_activation(&conn, request, 1001)?, result);
assert!(apply_activation(&conn, conflicting_request, 1002).is_err());
```

Also prove activation rejects any of:

- a row in `publication_log`
- `sqlite_sequence.seq > 0` for `publication_log`
- target cursor other than zero
- mismatched Edge ID or ledger epoch

- [ ] **Step 2: Run the focused Rust tests and confirm they fail**

Run:

```bash
cargo test -p iotkit-core-publish activation
```

Expected: FAIL because the activation module and migration do not exist.

- [ ] **Step 3: Add migration 20**

Create a singleton `site_activation` table with:

```text
singleton = 1
state = standalone | discovery_only | active
site_id
activation_id
ledger_epoch
discard_through_reading_seq
cleanup_through_reading_seq
result_json
activated_at
```

Migration state:

- existing `target_registry` row: `active`
- no target: `standalone`

- [ ] **Step 4: Implement strict wire validation**

`ActivationRequest` fields:

```rust
pub struct ActivationRequest {
    pub schema_version: u32,
    pub activation_id: String,
    pub site_id: String,
    pub edge_node_id: String,
    pub expected_ledger_epoch: String,
    pub grant_revision: u64,
    pub issued_at: i64,
}
```

`ActivationResult` fields:

```rust
pub struct ActivationResult {
    pub schema_version: u32,
    pub activation_id: String,
    pub site_id: String,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub status: String,
    pub discard_through_reading_seq: i64,
    pub first_publication_seq: i64,
    pub applied_at: i64,
}
```

IDs use their exact prefixes plus 32 lowercase hexadecimal characters. `grant_revision` and `schema_version` are exactly 1. Timestamps and boundaries are non-negative.

- [ ] **Step 5: Implement atomic activation and bounded cleanup**

`apply_activation` must use the caller's SQLite transaction boundary to:

1. validate current Edge identity/epoch and durable state;
2. replay the stored result for an exact duplicate;
3. reject a conflicting activation;
4. verify publication log count, `sqlite_sequence`, and target cursor are all zero;
5. freeze `MAX(readings.seq)` once;
6. persist `active` state and exact result JSON.

`cleanup_pre_activation_batch(conn, 5_000)` deletes at most 5,000 `readings` rows per transaction where `seq <= discard_through_reading_seq`, updates cleanup progress, and never touches a publication row.

- [ ] **Step 6: Extend `MqttBinding`**

Add exact fields:

```rust
pub activation_request_topic: String,
pub activation_result_topic: String,
```

with paths from the approved contract.

- [ ] **Step 7: Run focused tests**

Run:

```bash
cargo test -p iotkit-core-publish
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add core/publish
git commit -m "feat(edge): add Site activation state"
```

---

### Task 2: Gate every Edge publication path

**Files:**
- Modify: `core/collector/src/actor.rs`
- Modify: `core/ops/src/ops/commissioning_ops.rs`
- Modify: `iotkit-edge/src/epoch_start.rs`
- Modify: `core/publish/src/store.rs`
- Test: `core/collector/src/actor.rs`
- Test: `core/ops/tests/commissioning_smoke.rs`
- Test: `iotkit-edge/src/epoch_start.rs`

**Interfaces:**
- Consumes: `publication_admitted(conn) -> Result<bool, PublishError>`
- Produces: no publication row before activation; all existing enqueue APIs fail closed when discovery-only

- [ ] **Step 1: Add failing tests for every enqueue path**

Tests must prove:

```text
discovery-only measurement: readings +1, publication_log unchanged
standalone measurement: readings +1, publication_log +1
active measurement: readings +1, publication_log +1
discovery-only commissioning smoke: rejected, publication_log unchanged
discovery-only epoch_start: skipped, publication_log unchanged
```

Direct calls to `enqueue_measurement`, `enqueue_annotation`, and `enqueue_commissioning_smoke` must also respect the DB gate so a future caller cannot bypass the collector branch.

- [ ] **Step 2: Run focused tests and confirm failure**

Run:

```bash
cargo test -p iotkit-core-collector
cargo test -p iotkit-core-ops commissioning_smoke
cargo test -p iotkit-edge epoch_start
```

Expected: at least the new discovery-only assertions FAIL.

- [ ] **Step 3: Centralize the admission check in `core/publish` enqueue functions**

Do not add an in-memory activation boolean. Read activation state in the same SQLite transaction that inserts the reading/publication row. `standalone` and `active` admit publication; `discovery_only` does not.

- [ ] **Step 4: Keep the collector acknowledgement truthful**

A discovery-only normalized reading remains durably stored locally and keeps the existing durable ingest disposition. It is not described as Site-delivered and receives no publication identity.

- [ ] **Step 5: Run focused tests**

Run the commands from Step 2.

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add core/collector core/ops core/publish iotkit-edge/src/epoch_start.rs
git commit -m "feat(edge): gate publication until Site activation"
```

---

### Task 3: Implement Edge MQTT activation convergence

**Files:**
- Modify: `iotkit-edge/src/mqtt_publish_task.rs`
- Modify: `scripts/add-site-edge.sh`
- Modify: `scripts/bootstrap-site.sh`
- Modify: `scripts/test-mqtt-security.sh`
- Test: `iotkit-edge/src/mqtt_publish_task.rs`
- Test: script checks under `scripts/`

**Interfaces:**
- Consumes: activation request/result types and state functions from Task 1
- Produces: durable duplicate-safe request handling and result publication

- [ ] **Step 1: Add failing publisher tests**

Tests must prove:

- connection subscribes to both accepted-through and activation request;
- descriptor publishes while discovery-only;
- records do not publish while discovery-only;
- exact request applies once and publishes non-retained result;
- duplicate request republishes byte-identical result;
- reconnect does not recompute the discard boundary;
- records publish only after active state;
- cleanup deletes only the fixed prefix in batches.

- [ ] **Step 2: Run the focused test**

Run:

```bash
cargo test -p iotkit-edge mqtt_publish_task
```

Expected: FAIL.

- [ ] **Step 3: Add activation subscription and routing**

Track both subscriptions before starting convergence. Route incoming messages by exact topic:

```text
accepted-through -> existing custody handler
activation/request -> activation handler
anything else -> reject/log without payload
```

The handler persists state before publishing result. MQTT PUBACK does not mutate activation state.

- [ ] **Step 4: Add active-only records publishing and cleanup**

Before `prepare_batch`, load durable activation state. Discovery-only returns no batch without error. After activation, publish the stored result, permit records, and call one bounded cleanup batch per convergence tick until complete.

- [ ] **Step 5: Update binding validation and exact ACLs**

Edge:

```text
topic write .../records
topic write .../descriptors
topic write .../activation/result
topic read  .../accepted-through
topic read  .../activation/request
```

Site has the reverse permissions. Update exact JSON-key validation and negative cross-Edge ACL tests.

- [ ] **Step 6: Run focused tests**

Run:

```bash
cargo test -p iotkit-edge mqtt_publish_task
scripts/check-layers
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add iotkit-edge core/publish scripts
git commit -m "feat(edge): converge Site activation over MQTT"
```

---

### Task 4: Add Site activation storage and default-deny raw admission

**Files:**
- Create: `iotkit-site/internal/contract/activation.go`
- Create: `iotkit-site/internal/contract/activation_test.go`
- Create: `iotkit-site/internal/store/activations.go`
- Create: `iotkit-site/internal/store/activations_test.go`
- Modify: `iotkit-site/internal/store/migrations.go`
- Modify: `iotkit-site/internal/store/migrations_test.go`
- Modify: `iotkit-site/internal/store/store.go`
- Modify: `iotkit-site/internal/store/store_test.go`
- Modify: `iotkit-site/internal/store/descriptors.go`

**Interfaces:**
- Produces: `DiscoverEdge`, `RequestEdgeActivation`, `ApplyActivationResult`, `ListEdges`, `ListPendingActivationCommands`
- Consumes: descriptor identity and existing raw custody transaction

- [ ] **Step 1: Add failing migration and activation-store tests**

Add Site tables:

```text
site_meta(site_id)
edge_activations(edge_ref, edge_node_id, ledger_epoch, state, activation_id,
                 grant_revision, display_name, location, result_json,
                 revision, created_at, updated_at)
activation_command_outbox(activation_id, topic, payload_json, created_at, completed_at)
```

States are exactly `discovered`, `activating`, `active`, `recovery_hold`.

Tests must prove:

- descriptor creates `discovered`, not `active`;
- request operation atomically stores grant, audit, and command outbox;
- exact result transitions to `active` and completes the command;
- conflicting result enters `recovery_hold`;
- duplicate result is idempotent;
- ambiguous multiple-epoch legacy data is not auto-activated.

- [ ] **Step 2: Add failing default-deny custody tests**

For the same valid batch:

```go
_, err := store.AcceptBatch(ctx, batch) // discovered
assert.ErrorIs(t, err, store.ErrEdgeNotActive)
assertRawCount(t, 0)

activateExactEpoch(t, store, batch.EdgeNodeID, batch.LedgerEpoch)
ack, err := store.AcceptBatch(ctx, batch)
assert.NoError(t, err)
assert.Equal(t, batch.CursorEnd, ack.AcceptedThrough)
```

Also prove an unexpected epoch stores nothing and returns no ack.

- [ ] **Step 3: Run focused Go tests and confirm failure**

Run:

```bash
go test ./internal/contract ./internal/store
```

Working directory: `iotkit-site`

Expected: FAIL.

- [ ] **Step 4: Implement strict activation payload decoding**

Reject unknown fields, invalid prefixes/hex, control characters, non-v1 schema/revision, negative timestamps/boundaries, topic/body identity mismatch, and a result whose `first_publication_seq` is not 1.

- [ ] **Step 5: Implement Site IDs, discovery, activation, and legacy migration**

Generate one stable random `site_id`. Descriptor receipt creates or refreshes a discovered Edge for the exact epoch. Existing raw/cursor state is auto-active only when one current epoch is unambiguous; descriptor-only remains discovered; multiple epochs enter recovery hold.

- [ ] **Step 6: Put the activation gate inside `AcceptBatch`**

In the same SQL transaction, before raw insert:

```text
load exact edge activation
require state=active
require ledger_epoch=batch.ledger_epoch
then run existing gap/fingerprint/raw/cursor logic
```

No state mismatch may insert raw or publish accepted-through.

- [ ] **Step 7: Run focused tests**

Run the command from Step 3.

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add iotkit-site/internal/contract iotkit-site/internal/store
git commit -m "feat(site): persist Edge activation admission"
```

---

### Task 5: Converge Site activation commands over MQTT

**Files:**
- Modify: `iotkit-site/internal/mqttsite/processor.go`
- Modify: `iotkit-site/internal/mqttsite/processor_test.go`
- Modify: `iotkit-site/internal/mqttsite/client.go`
- Modify: `iotkit-site/internal/mqttsite/client_test.go`

**Interfaces:**
- Consumes: pending command outbox and activation result store from Task 4
- Produces: non-retained request retry and result processing

- [ ] **Step 1: Add failing processor and convergence tests**

Prove:

- Site subscribes to `iotkit/v1/edge-nodes/+/activation/result`;
- a valid result applies only to the matching locally pending grant;
- malformed/conflicting results do not activate an Edge;
- pending request publish success does not mark the command complete;
- request is retried until matching result commit;
- request publication is QoS 1 and non-retained;
- records received before active return no accepted-through.

- [ ] **Step 2: Run focused tests and confirm failure**

Run:

```bash
go test ./internal/mqttsite
```

Expected: FAIL.

- [ ] **Step 3: Extend processor routing**

Route descriptors, activation results, and records independently. Activation-result errors never produce an accepted-through response. Log topic and bounded error only, never payload or credentials.

- [ ] **Step 4: Extend the Site convergence loop**

Each tick lists bounded pending activation commands and publishes them QoS 1 non-retained. A successful MQTT publish increments attempt metadata only; matching result commit completes the command.

- [ ] **Step 5: Run focused tests**

Run:

```bash
go test ./internal/mqttsite
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add iotkit-site/internal/mqttsite
git commit -m "feat(site): converge Edge activation over MQTT"
```

---

### Task 6: Add the Edge hierarchy and activation operation to Site Console

**Files:**
- Modify: `iotkit-site/internal/siteapp/service.go`
- Modify: `iotkit-site/internal/siteapp/service_test.go`
- Modify: `iotkit-site/internal/siteapp/types.go`
- Modify: `iotkit-site/internal/sitehttp/server.go`
- Modify: `iotkit-site/internal/sitehttp/api_v1.go`
- Modify: `iotkit-site/internal/sitehttp/console.go`
- Modify: `iotkit-site/internal/sitehttp/console_view.go`
- Modify: `iotkit-site/internal/sitehttp/templates/console.html`
- Modify: `iotkit-site/internal/sitehttp/static/site.css`
- Modify: `iotkit-site/internal/sitehttp/server_test.go`

**Interfaces:**
- Produces: `ListEdges`, typed `ActivateEdge`, `POST /api/v1/edges/{edge_ref}/activation`, `/edges`
- Consumes: activation store operations from Task 4

- [ ] **Step 1: Add failing application-service tests**

Define:

```go
type ActivateEdge struct {
    EdgeRef string
    Precondition RevisionPrecondition
}
```

Only `admin` and `system_admin` may dispatch it. A duplicate click returns the same in-progress activation. Viewer is forbidden. Audit actor and operation are persisted by the repository transaction.

- [ ] **Step 2: Add failing HTTP and HTML tests**

Prove:

- unauthenticated API returns 401;
- viewer returns 403;
- admin POST with CSRF and revision returns 202;
- Edge list distinguishes `未登録`, `登録処理中`, `登録済み`, `復旧確認待ち`;
- Edge is a parent of descriptor devices, not another device;
- credential, password, CA path, internal activation payload, and raw hardware ID are absent;
- current device/sensor setup pages still render and save.

- [ ] **Step 3: Run focused tests and confirm failure**

Run:

```bash
go test ./internal/siteapp ./internal/sitehttp
```

Expected: FAIL.

- [ ] **Step 4: Implement the typed operation and API**

Use the existing actor, CSRF, revision precondition, error mapping, and audit patterns. The Console never edits Broker connection profiles or credentials.

- [ ] **Step 5: Add an Edge page and navigation**

Show:

```text
Edge display name
location
activation state
exact ledger epoch in diagnostics
descriptor device/sensor counts
last descriptor/result time
activation action for admin only
```

Do not label last-seen descriptor as “接続中”. Use `最終通信` until LWT exists.

- [ ] **Step 6: Run focused tests**

Run:

```bash
go test ./internal/siteapp ./internal/sitehttp
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add iotkit-site/internal/siteapp iotkit-site/internal/sitehttp
git commit -m "feat(site): add Edge activation Console"
```

---

### Task 7: Prove the complete protocol and deployment journey

**Files:**
- Modify: `scripts/test-mqtt-vertical.sh`
- Modify: `scripts/test-mqtt-security.sh`
- Modify: `scripts/test-clean-install.sh`
- Modify: `docs/site-operations.md`
- Modify: `testdata/egress/v1/`

**Interfaces:**
- Consumes: all prior tasks
- Produces: executable evidence for activation, custody, failure recovery, and ACL isolation

- [ ] **Step 1: Add cross-language activation fixtures**

Add valid request/result JSON fixtures decoded by Rust and Go. Add invalid fixtures for wrong epoch, wrong Edge ID, unknown fields, malformed IDs, result sequence other than 1, and conflicting duplicate activation ID.

- [ ] **Step 2: Extend the Docker vertical test**

The test sequence must prove:

```text
fresh Edge connects
descriptor reaches Site
preactivation sensor reading exists on Edge
publication_log remains empty
preactivation records published manually are rejected/no ack
admin/API activation requested
Edge result reaches Site
Site becomes active
next sensor reading receives pub_seq=1
Site raw commit occurs
accepted-through=1 reaches Edge
duplicate request and reconnect do not change boundary
preactivation prefix cleanup never deletes post-boundary reading
```

- [ ] **Step 3: Extend security tests**

Prove exact ACL direction for both activation topics, cross-Edge read/write rejection, anonymous rejection, and non-retained command behavior.

- [ ] **Step 4: Update installation and recovery instructions**

The operator order becomes:

```text
Broker enrollment and handoff
Edge descriptor discovery
Site Console activation
commissioning smoke
sensor meaning setup
```

Document `recovery_hold`, nonempty legacy outbox rejection, and that activation does not manage credentials or guarantee secure physical erasure.

- [ ] **Step 5: Run focused integration tests**

Run:

```bash
scripts/test-mqtt-vertical.sh
scripts/test-mqtt-security.sh
scripts/test-clean-install.sh
```

Expected: PASS.

- [ ] **Step 6: Run the one final full verification**

Run:

```bash
scripts/verify.sh
```

Expected: formatting, layer checks, Rust workspace tests, Clippy `-D warnings`, and Site tests all PASS.

- [ ] **Step 7: Commit**

```bash
git add scripts docs/site-operations.md testdata
git commit -m "test: prove Edge Site activation journey"
```

---

## Completion Criteria

- A fresh Broker-enrolled Edge is discovered but cannot place raw records in Site before activation.
- Activation is durable, idempotent, exact-epoch-bound, application-acknowledged, and independent of MQTT PUBACK.
- Only post-boundary observations receive publication identities, beginning at `pub_seq=1`.
- Site accepts only the active exact epoch and keeps existing raw/cursor atomic custody semantics.
- Reconnect, duplicate command, Broker outage, Edge crash, and Site crash converge without recomputing the boundary or deleting post-boundary data.
- Existing unambiguous deployments continue as active; ambiguous legacy/restore state fails closed.
- Console visibly separates Edge, device, and sensor and permits activation only to admin roles.
- Docker vertical, security, clean-install, and final repository verification pass.
