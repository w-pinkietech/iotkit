# Edge / Site Descriptor Snapshot Implementation Plan

> **Execution:** Implement inline in this isolated worktree with test-first changes. Do not dispatch subagents.

**Goal:** Edgeのadapter-neutralなデバイス・信号descriptorをretained MQTTで配信し、Siteがraw custodyとは独立に検証・複製できるようにする。

**Architecture:** Edge ledger/registryを正本とし、`core/publish`がcomplete snapshotを組み立てる。Edge MQTT taskはrecordsとは別topicへQoS 1 retained publishし、Siteはstrict contract validation後にSQLiteへrevision-awareに反映する。descriptorの失敗はrecords受理、`accepted-through`、semantic projectionを止めない。

**Tech Stack:** Rust 2024, rusqlite, rumqttc, Go 1.25, modernc SQLite, Paho MQTT, Mosquitto 2.

## Global constraints

- Topicは`iotkit/v1/edge-nodes/{edge_node_id}/descriptors`、QoS 1、retainedとする。
- Snapshot v1はcomplete snapshotだけを許し、encoded sizeは最大1 MiB。超過時はtruncateしない。
- `presentation_identifier`は任意・非一意・非権威の表示値であり、hardware IDやcredentialをsnapshotへ含めない。
- `descriptor_revision`はEdge DBへ永続化し、descriptor内容を変えるDB transactionで増加する。
- Siteは同epochの低revisionを無視し、同revision・異内容をconflictとして監査する。
- descriptor処理はraw custodyと`accepted-through`から独立させる。
- Site固有profile、semantic mapping、raw recordはsnapshot反映で変更・削除しない。

---

### Task 1: Edge descriptor schema and wire contract

**Files:**
- Create: `core/ledger/migrations/0018_descriptor_metadata.sql`
- Create: `core/registry/migrations/0019_descriptor_revision.sql`
- Create: `core/publish/src/descriptor.rs`
- Create: `iotkit-edge/src/descriptor_snapshot.rs`
- Create: `testdata/egress/v1/descriptor-snapshot.json`
- Modify: `core/ledger/src/{lib.rs,store.rs}`
- Modify: `core/registry/src/lib.rs`
- Modify: `core/publish/{Cargo.toml,src/lib.rs}`

**Interfaces:**
- `set_presentation_identifier(conn, system_id, Option<&str>) -> Result<(), LedgerError>` validates a short printable identifier and changes no identity semantics.
- `iotkit_edge::descriptor_snapshot::build_descriptor_snapshot(conn, edge_node_id) -> Result<DescriptorSnapshot, PublishError>` composes ledger and registry without adding a core dependency cycle.
- `DescriptorSnapshot::encode_bounded() -> Result<Vec<u8>, PublishError>` rejects payloads above 1 MiB.

- [x] Add failing migration tests proving existing Edge data survives, descriptor revision is initialized, and relevant device/series/registry mutations increment it in the same transaction.
- [x] Add failing contract tests for the shared fixture, deterministic ordering, resolved registry metadata, optional identifier, unknown fields, invalid identifier, and 1 MiB rejection.
- [x] Implement migrations, identifier validation/storage, and snapshot builder with no hardware/provider/secret fields.
- [x] Run focused ledger/registry/publish tests and commit this independently testable contract.

### Task 2: Edge MQTT retained publication and deployment binding

**Files:**
- Modify: `core/publish/src/mqtt.rs`
- Modify: `core/publish/tests/mqtt_binding.rs`
- Modify: `iotkit-edge/src/mqtt_publish_task.rs`
- Modify: `iotkit-edgectl/tests/cli.rs`
- Modify: `scripts/{bootstrap-site.sh,test-site-bootstrap.sh,test-site-mqtt.sh}`
- Modify: `deploy/mosquitto/dev.acl`

**Interfaces:**
- `MqttBinding` adds `descriptor_topic` and explicit retained behavior while preserving records as non-retained.
- MQTT task publishes the current snapshot after every connection and republishes only when the persisted `(ledger_epoch, descriptor_revision)` changes while connected.

- [ ] Add failing binding/runtime tests proving exact topic derivation, retained QoS 1 publication, reconnect republish, changed-revision republish, and oversize/error isolation from records.
- [ ] Implement publication without coupling descriptor success to records subscription, batch delivery, or acknowledgement handling.
- [ ] Add descriptor read/write ACLs to development and generated production configuration; update bootstrap validation for the new non-secret binding fields.
- [ ] Extend the Docker MQTT vertical slice to observe the retained descriptor independently of record custody.
- [ ] Run focused Rust, bootstrap, and MQTT vertical-slice tests and commit.

### Task 3: Site descriptor contract and durable replica

**Files:**
- Create: `iotkit-site/internal/contract/descriptor.go`
- Create: `iotkit-site/internal/contract/descriptor_test.go`
- Create: `iotkit-site/internal/store/descriptors.go`
- Create: `iotkit-site/internal/store/descriptors_test.go`
- Modify: `iotkit-site/internal/store/{migrations.go,migrations_test.go}`

**Interfaces:**
- `contract.DecodeDescriptorSnapshot(payload) (DescriptorSnapshot, error)` performs strict bounded decoding and reconstructs every `series_key` for equality validation.
- `Store.ApplyDescriptorSnapshot(ctx, snapshot) (DescriptorApplyResult, error)` atomically updates the current replica.
- Result distinguishes `applied`, `idempotent`, and `stale_ignored`; same-revision content conflict returns a typed error after writing a secret-free system audit event.

- [ ] Add failing strict-decoder tests using the shared fixture plus malformed identity, state, series-key, duplicate, unknown-field, and oversize cases.
- [ ] Add failing migration/store tests for first apply, idempotent replay, lower revision ignore, same-revision conflict audit, epoch replacement, and missing-entry stale marking.
- [ ] Implement schema v3 and atomic apply without touching raw, cursor, mapping, semantic event, or output tables.
- [ ] Run focused Site contract/store tests and commit.

### Task 4: Site MQTT subscription and canonical documentation

**Files:**
- Modify: `iotkit-site/internal/mqttsite/{client.go,client_test.go,processor.go,processor_test.go,integration_test.go}`
- Modify: `docs/{architecture.md,exit-contract.md}`
- Modify: `docs/redesign/decisions/{D9-exit-mqtt-binding.md,D10-exit-authentication.md}`
- Modify: `docs/superpowers/specs/2026-07-15-site-console-api-design.md`

**Interfaces:**
- Site subscribes to both `+/records` and `+/descriptors` at QoS 1 and routes each message to its independent processor path.
- Descriptor errors are logged without publishing `accepted-through`; record behavior remains unchanged.

- [ ] Add failing client/processor tests for exact descriptor topic matching, valid apply, invalid descriptor isolation, and records continuing after descriptor failure.
- [ ] Implement multi-topic subscription and store application.
- [ ] Update canonical contract/auth/architecture text with the implemented fields, ACLs, retained semantics, and failure isolation; mark this spec slice implemented without duplicating it elsewhere.
- [ ] Run all Site tests and focused MQTT integration tests, then commit.

### Task 5: Review and full verification

- [ ] Review the diff against the approved Site Console/API descriptor section and the invariants in `AGENTS.md`.
- [ ] Run `scripts/verify.sh` because Rust product behavior and migrations changed.
- [ ] Run `go test ./... -count=1` for `iotkit-site` in the existing Go Docker image.
- [ ] Run `scripts/test-site-bootstrap.sh` and the Docker MQTT vertical slice.
- [ ] Run `git diff --check`, inspect `git status`, and report any intentionally omitted Pi test (no hardware behavior changed).
