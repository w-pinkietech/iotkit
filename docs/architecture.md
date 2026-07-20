# Architecture

IoTKit currently ships the Rust IoTKit Edge binary (`iotkit-edge`) plus an operator CLI
(`iotkit-edgectl`), backed by a single SQLite database, and the independently deployable Go
IoTKit Site (`iotkit-site`), backed by its own SQLite database and a standard MQTT broker.
Site is not part of the Edge process. Edge runs unattended on a
Raspberry Pi under systemd. This document is the "get oriented in 10 minutes"
map **and the canon for code placement**; the authoritative *why* is the
Japanese design corpus under `docs/redesign/` (decision records D1–D13,
responsibility ledger R1–R23).

## Who this serves

Every structural choice below is graded against five audiences. When a change
makes life worse for one of them, that is a review finding, not a taste issue.

| Audience | What they touch | What "good" means for them |
|---|---|---|
| **Site installers & operators** | The install story, `iotkit-edgectl`, the API/UI, error messages | They can tell *what to install on which device and how to assemble a site* from one page. Errors name the thing to check next. Defaults are safe; nothing silently loses data; a Pi is fast enough. |
| **Self-made-device builders** (ESP32 hobbyists, plant engineers — often not Rust readers) | The ingest **wire contract only** | Onboarding stays a curl-3-lines experience. Contract docs never require reading Rust. Rejections come back with a reason code they can act on. |
| **Adapter developers** (Rust) | `core/types`, `iotkit-input-adapter-host-api`, `iotkit-input-adapter-testkit`, `iotkit-polling-adapter-runtime`, an existing adapter as template | The adapter boundary is obvious; a new sensor family means a new adapter crate, not core surgery. No knowledge of storage/ledger internals needed. |
| **Core contributors** (Rust) | `core/*`, IoTKit Edge, tests | The crate map fits in one screen. Layer rules are machine-checked, not tribal. Each crate has one responsibility; tests read as the executable spec. |
| **Data consumers** (dashboards, Node-RED, analytics) | The **exit contract** | The record schema, ack rules, and cursor semantics are documented and versioned; no schema surprises. |

## Site anatomy — what runs where

The design corpus describes four tiers of a deployment (terminology.md):
**[1] devices** (sensors/actuators in the field, incl. third-party self-made
hardware) → **[2] IoTKit Edge** (Raspberry Pi, this repo) → **[3] IoTKit Site**
(per-site aggregation; may be hosted off-premises) → **[4] cloud**.

**The current Cargo workspace ships tier [2].** Tier [3] is a separate Go program in this repository
that consumes only the public MQTT exit contract; it does not import Edge Rust packages or read
the Edge database. Tier [1] is hardware plus the ingest wire contract. Tier [4] remains external.
So a minimal Edge install is: flash a Pi, run the `iotkit-edge` daemon
under systemd, keep `iotkit-edgectl` on the same Pi as a hand-run CLI, and wire
adapters to sensors. A standalone site can stop there (D8: an upstream is
optional). IoTKit Site adds durable aggregation, Edge Node cursors, direct raw
query, Edge descriptor replica, configurable site-local sensor meaning, and the application-export boundary. Site maps one
stored series to one typed meaning such as `production_pulse`; a separate exporter converts the
result to an application-facing MQTT contract. Applications such as YokaKit own business masters
and logic such as products, processes, OEE, alarms, business UI, and notifications. Anything that
complicates this story needs a strong reason.

The exporter boundary is the versioned
[Output Adapter contract v1](output-adapter-contract.md). An Output Adapter is a deterministic
in-process transformer from a generic Site observation plus route configuration to one exact MQTT
publication. It never owns Broker connectivity, credentials, durable outbox state, retries, or
business masters. `yokakit.mqtt.v1` is the first implementation, not a privileged core path.

The current production-shaped reference installation keeps Edge native on its Raspberry Pi and
co-locates the standard Broker plus Site in Docker on one Linux host. Co-location is not a product
requirement: the Broker and Site may run on separate hosts and communicate only through the same
authenticated MQTT/TLS contract, without a shared filesystem or Compose project. The current
`scripts/bootstrap-site.sh` consumes the
non-secret `iotkit-edgectl mqtt-binding` document and operator-provided TLS material, then creates
an anonymous-disabled Broker, exact per-Edge-Node ACLs, owner-only credential files, and a small
Edge handoff. It does not issue certificates, configure DNS/firewalls/VPNs, or modify the Edge.
`deploy/compose.site.yaml` consumes only generated file paths and non-secret network settings; it
does not place MQTT credentials in Compose environment values or argv. A split deployment must
produce separate Broker-host, Site-client, and per-Edge-client artifacts. Site has its own Broker
principal and credential even when it is co-located with the Broker.

`deploy/mosquitto-image.env` is the repository's single source for the verified Mosquitto patch
release used by production generation, Compose, and integration tests. Updating that exact patch
reference requires the MQTT security matrix and the normal final verification gate; floating
major/minor tags are not production inputs.

## Data flow

The deployed BravePI path is `BravePI Mainboard -> UART -> IoTKit Edge -> MQTT Broker -> IoTKit Site`.
Inside Edge, the adapter and collector normalize and durably enqueue observations before publishing:

```
  ┌─────────────┐    Envelope       ┌───────────────┐
  │  adapters   │ ────────────────▶ │   collector   │  R8: dedup, series resolution,
  │ (BravePI,   │  (in-process      │   (ingest)    │      quarantine and activation admission;
  │  rpi-local) │   ingest client)  └──────┬────────┘      active records enqueue in the SAME tx
  └─────────────┘                          │
                                           ▼  one Immediate transaction
                              ┌────────────────────────────┐
                              │           SQLite           │
                              │  readings (internal seq)   │  R16: durable, crash-consistent
                              │  publication_log (pub_seq) │  R10: the outbox
                              │  series / devices / ...    │  R5–R7: identity & registry
                              └──────────┬─────────────────┘
                    ┌────────────────────┼────────────────────┐
                    ▼                    ▼                    ▼
             ┌────────────┐      ┌──────────────┐     ┌──────────────┐
             │ push task  │      │  retention   │     │ health writer│
             │ R10 exit   │      │ R17 custody- │     │ R12 status   │
             │ contract   │      │ aware purge  │     │ JSON         │
             └─────┬──────┘      └──────────────┘     └──────────────┘
                   │ MQTT QoS 1 (transport PUBACK only)
                   ▼
             standard MQTT broker
                   │
                   ▼
             IoTKit Site ── durable accepted-through topic ──▶ authorizes purge
```

Edgeは同じBroker上のEdge Node固有`descriptors` topicへ、ledger/registryから組み立てた1 MiB以下の
schema 2 complete snapshotをQoS 1 retainedで送る。機器に明示的に永続化された任意`model_id`だけを
入力モデル情報として含める。Siteはschema 2とrevision/epochを検証して専用tableへ
複製する。Adapterインスタンス、物理locator、hardware/provider識別子はこの境界を越えない。この経路は
publication outbox、raw transaction、accepted-through cursorと結合せず、失敗してもcustody処理を継続する。

Broker enrollment済みでもSite activation前のEdgeは、正規化済み観測をEdgeローカルへ保持するだけで
publication logへ採番せず、recordsを送信しない。SiteはdescriptorからEdgeを発見し、admin typed operationで
exact ledger epochをactivationする。Edgeはcollectorと同じSQLite write serializationで境界を一度だけ固定し、
それ以後のingestだけをoutboxへ入れる。Siteはmatching activation resultをcommitしてactiveになった後だけ、
activation検査、raw保存、cursor更新を同じtransactionで行う。登録前prefixの物理削除は固定境界を使う
Edge-local cleanupであり、accepted-throughの意味やpost-activation purge権威を変更しない。

`mqtt_publish_task` is the active production exit binding. The older `publish_task` HTTPS code is
retained only as transitional code and is not spawned. A broker PUBACK confirms transport receipt
only; IoTKit Edge retains its outbox until IoTKit Site commits raw records and publishes
application-level `accepted-through`.

### Site semantic and application-export loop

While `iotkit-site serve` consumes raw batches, an independent 250 ms convergence loop projects
committed raw contact values, enqueues versioned application events in the Site outbox, and
publishes pending rows at MQTT QoS 1. Only a successful PUBACK marks an outbox row published;
failure or the 15-second timeout leaves it pending for a later tick. Projection, enqueue, and
publish errors are logged without payloads or credentials and do not stop the loop.

This is deliberately a two-stage failure boundary. The raw batch transaction and its
`accepted-through` publish never wait for semantic projection or application export, so an
application outage cannot hold Edge custody. Semantic mappings are future-only from each current
Edge cursor, and MQTT routes are future-only from the current semantic-event boundary.

Operators use JSON-producing CLI commands backed by the typed Site application-service dispatcher
(the raw `query` command remains available). Semantic mapping changes and the legacy MQTT route
command commit their success audit in the same SQLite transaction as the setting change:

```bash
iotkit-site mapping-set --db site.db --edge-node-id edge-node-01 \
  --series-key '<series_key>' --meaning production_pulse \
  --trigger-mode active_edge --active-value 1
iotkit-site mapping-deactivate --db site.db --edge-node-id edge-node-01 \
  --series-key '<series_key>'
iotkit-site mapping-list --db site.db
iotkit-site route-add --db site.db --mapping-id '<mapping_id>' \
  --topic 'iotkit/v1/application/production-pulses'
iotkit-site semantic-query --db site.db --limit 100
```

The application MQTT payload is contract v1: `schema_version`, stable `event_id`, `mapping_id`,
`mapping_revision`, revision-local `event_sequence`, `meaning`, source Edge Node/series/publication
sequence, `occurred_at`, and `count`. Count is cumulative only within a mapping revision and resets
to 1 for a new revision. It is an IoTKit event contract rather than a legacy device-address/pin
payload. The semantic slice is implemented and its future-only `active_edge` path, QoS 1 outbox,
application publish, and duplicate-publication idempotence are verified against a live Docker
Mosquitto broker on the host. Combined with the existing BravePI-to-raw hardware evidence, this
closes the slice without repeating the semantic path on the Pi.

Adapters speak the **ingest contract** (`Envelope`/`Ack`, crate
`iotkit-ingest-contract`) through `iotkit-ingest-client`. The current network
binding is a separate, default-off authenticated HTTP/TLS listener:

```
  device builder ── HTTPS + device bearer ──▶ bounded HTTP ingress ──▶ collector
       (Envelope/Ack; /api/v1/ingest)          (/validate is no-write)
```

In-process adapters and the HTTP binding both hand the collector a
receiver-created principal; sender-controlled `Envelope.source` never grants
authority. MQTT and pairing-window bindings remain future work. `AdapterEvent`/`AdapterCommand` are a
*frozen* legacy vocabulary, **not** the ingest path: `AdapterEvent` carries
adapter-lifecycle supervision into `core/engine`'s device-state projection,
and `AdapterCommand` carries shutdown plus legacy southbound commands into
the adapter runtimes. Both are defined in `core/supervision`. The freeze is
machine-enforced for **new crate dependents**: `scripts/check-layers` rule 7
pins the complete dependent set. Reviews still police usage growth inside the
existing dependent crates (D4/D12 decision 8).

The implemented compile-time northbound extension boundary for official in-process
sensor adapters is [Input Adapter host contract v1](input-adapter-contract.md). Adapter type,
configured instance, diagnostic source, receiver-owned principal, observed
subject, and ledger-owned system identity remain separate. The shared runtime
host/composition API carries no `AdapterEvent`/`AdapterCommand`; Edge-private factories
and package-private legacy projections isolate the frozen vocabulary while Edge retains principal creation,
configuration authority, restart policy, and health aggregation.

## The custody loop (the core idea)

IoTKit Edge is a **buffer, not a warehouse**. A measurement's lifecycle:

1. **Ingest** — the collector writes a `readings` row and, in the *same* SQLite
   transaction, enqueues an outbox row in `publication_log` (only for
   non-quarantined measurements). Crash-consistent: you never get a reading
   without its outbox row, or vice versa.
2. **Publish** — the publisher batches undelivered outbox rows and sends them through a standard
MQTT broker with a per-Edge-Node credential. The DB lock is **not** held across the network
   round-trip. Broker PUBACK does not release application custody.
3. **Ack → cursor** — after its SQLite commit, Site publishes a valid `accepted-through` ack
   (matching Edge Node, epoch, publication id, and batch bound), which
   advances the per-target cursor. The cursor is the consumer's durable
   watermark: "I have taken custody up to here."
4. **Purge/degrade** — normal retention deletes archive-acknowledged data beyond the minimum floor.
   Under pressure the authoritative D2/R17 order is: acknowledged data, out-of-custody-policy data,
   unresolved quarantine, then unacknowledged originals only as the final explicit data-loss class.
   Reaching the last class requires `custody_lost` audit plus a structured gap annotation; silent
   deletion is forbidden.

If the consumer is down, the cursor stops advancing, the backlog grows, and disk
fills — at which point *new writes fail loudly* (`ENOSPC`). IoTKit Edge never
silently drops stored data to make room. (Graceful active back-pressure is future
work; today the contract is "safe, not graceful" under sustained pressure.)

Installers can enqueue the optional `commissioning_smoke` record through the R14 operation-backed
`iotkit-edgectl smoke enqueue` command and compare its epoch/publication sequence with the
`accepted-through` cursor through `smoke status`. This verifies the normal custody path without
direct SQLite access or pretending that a physical sensor produced a measurement.

## Control plane

Since plan 5 IoTKit Edge also runs an **HTTPS API server** (axum + rustls,
self-signed certificate whose SHA-256 fingerprint is surfaced for pinning;
private-address clients only). State changes ride the **R14 typed operation
catalog** (`core/ops`): each operation is a descriptor with a permission tier
(`Routine` / `Daily` / `Construction`), dry-run support, and unconditional
audit into `ledger_events`. Today the API's mutation routes go through catalog
dispatch. The plan-5 CLI commands reuse the same `core/ops` functions —
`token issue|revoke` call exactly what the catalog descriptors call, and
`passphrase reset` is a deliberate out-of-band recovery path — each audited
as `local_cli`. Migrating the older CLI mutation paths (targets, snapshots,
replace-hardware) onto R14 is queued across plans 7–8. Session login is the
only network endpoint here that changes state outside R14 by design. Authentication is an admin
passphrase (argon2id) plus operator tokens (issued once in plaintext, stored as
SHA-256; AI tokens are structurally capped at `Routine`). Initial ownership and
admin recovery are local-root maintenance commands: an unowned/recovering box
does not bind the control API/UI, and `iotkit-edgectl passphrase reset` atomically
replaces the credential and revokes all operator/session tokens. There is no
network setup route or unauthenticated setup allowlist. The prescriptive rule:
**a new mutation surface is an R14 descriptor — never a fresh SQL mutation
path.** Local-root ownership/recovery (and the separately specified factory-reset
maintenance family) are explicit non-network exceptions, never API/UI/AI operations.

## Current implementation state

The repository currently ships the first network measurement path as a separate,
default-off `iotkit-ingest-http` listener. It accepts authenticated JSON
envelopes over a site-LAN TLS binding, applies finite header/body/time/queue and
admission limits, returns custody-correct acknowledgements, and exposes the
side-effect-free `/api/v1/ingest/validate` endpoint. Its principal, staging,
deduplication, health, and episode-audit boundaries are distinct from the
control API.

The current slice is deliberately narrow: one paired BravePI temperature sensor through
the existing Long Range BLE/BravePI Mainboard/UART path, one IoTKit Edge, one standard MQTT Broker,
one IoTKit Site, raw SQLite storage, application-level accepted-through, future-only semantic
projection, a durable application MQTT outbox, and direct CLI queries. BravePI owns
BLE, pairing through its existing iOS application, and transmitter management; IoTKit starts at the
BravePI Mainboard UART stream. A production-shaped one-Edge bootstrap exists for the Broker/Site
TLS boundary. Automatic Broker certificate issuance/renewal is now designed as a Broker-host
operations component but is not implemented in this slice. Enrollment, credential rotation,
Site backup/restore, legacy HTTPS migration, multi-Edge-Node hardware, YokaKit integration, and UI
remain later implementation work.

## Crate map

Twenty-four crates, five layers. `scripts/check-layers` enforces the layer rules
below mechanically (in `verify.sh` and CI).

| Crate | Path | Responsibility (one line) |
|---|---|---|
| `iotkit-core-types` | `core/types` | Domain entity types (no protocol specifics). Leaf. |
| `iotkit-ingest-contract` | `iotkit-ingest-contract` | Ingest wire contract v1: `Envelope`/`Ack`/reason codes. The wire is normative; runtime deps = serde only (serde_json appears only in dev-dependencies). Leaf. |
| `iotkit-core-storage` | `core/storage` | SQLite handle (`DbHandle`) + cross-crate migration harness. Leaf. |
| `iotkit-core-supervision` | `core/supervision` | Frozen supervision / legacy-southbound `AdapterEvent`/`AdapterCommand` vocabulary (D4/D12). Depends only on `types`; its dependent set is pinned by rule 7. |
| `iotkit-core-engine` | `core/engine` | In-memory device-state projection consuming the frozen `AdapterEvent` vocabulary from `core/supervision`. Depends only on `types` and `supervision`; adapters must never depend on it. |
| `iotkit-core-ledger` | `core/ledger` | Device ledger: `system_id` issuance, series identity, sightings, epochs, audit events. |
| `iotkit-core-timeseries` | `core/timeseries` | `readings` + staged readings persistence, event-time derivation, queries. |
| `iotkit-core-publish` | `core/publish` | Exit-contract data layer: Site activation admission, `publication_log` (outbox), `target_registry`, cursors. |
| `iotkit-core-collector` | `core/collector` | Ingest actor: dedup, series resolution, quarantine and activation admission, active-record same-tx outbox enqueue. Owns the `RegistryPolicy` trait. |
| `iotkit-core-registry` | `core/registry` | D6 measurement registry (standard catalog + site overrides); implements `RegistryPolicy`. |
| `iotkit-core-ops` | `core/ops` | R14 operation catalog, permission tiers, auth store (passphrase/tokens), dispatch + audit. |
| `iotkit-ingest-client` | `iotkit-ingest-client` | The ingest-contract client adapters use (D4). In-process binding for official adapters; network device builders use the separate HTTP binding. MQTT remains future. |
| `iotkit-input-adapter-host-api` | `iotkit-input-adapter-host-api` | Supervision-free official adapter composition API: validated identities, source-bound ingest, delivery receipts/retry, bounded diagnostics/activity, completion, and shutdown. |
| `iotkit-input-adapter-testkit` | `iotkit-input-adapter-testkit` | Dev-only conformance assertions and a non-catalog two-subject/two-measurement reference adapter. |
| `iotkit-ingest-http` | `iotkit-ingest-http` | **INGRESS.** Listener parsing, exposure/TLS validation, accepted-peer checks, and transport construction; never control-API routes or measurement domain logic. |
| `iotkit-polling-adapter-runtime` | `iotkit-polling-adapter-runtime` | Supervision-, ingest-, mapping-free I2C polling engine; emits decoded observations and lifecycle facts. The current name is retained for compatibility but its public SPI is I2C-specific. |
| `rpi4b-transport` | `rpi4b-transport` | Raw host I/O. I2C exposes an injectable device/factory boundary and combined write-read; GPIO/SPI/PWM use Raspberry Pi `rppal`. The historical crate name does not limit support to Pi 4B. |
| `iotkit-sensor-drivers` | `iotkit-sensor-drivers` | Vendor-neutral per-sensor-IC constants, identity metadata, and datasheet conversion components shared by adapters; not a complete transport-owning driver. |
| `bravepi-codec` | `bravepi-mainboard-adapter/codec` | BravePI frame encoding/decoding. |
| `bravepi-mainboard-adapter` | `bravepi-mainboard-adapter` | BravePI-protocol adapter: transport + codec + sensor drivers → Envelopes. |
| `rpi-local-adapter` | `rpi-local-adapter` | Direct Linux I2C adapter under its accepted `rpi-local` compatibility name; owns the typed supported-device catalog, driver construction, measurement projection, and inventory metadata. |
| `bravepi-poc` | `bravepi-mainboard-adapter/poc` | Hardware proof-of-concept harness for BravePI (dev tool, not shipped). |
| `iotkit-edge` | `iotkit-edge` | **Binary.** IoTKit Edge composition root: adapter supervision, MQTT exit publisher, retention, health, HTTPS API. |
| `iotkit-edgectl` | `iotkit-edgectl` | **Binary.** Edge operator CLI: ledger, registry, snapshots, targets, tokens (audited; plan-5 commands reuse the `core/ops` functions; older mutation paths migrate to R14 in plans 7–8). |

Approved next-slice non-Rust placement:

| Component | Path | Responsibility (one line) |
|---|---|---|
| IoTKit Site | `iotkit-site/` | MQTT consumer, durable raw acceptance, Edge Node cursor manager, accepted-through publisher, query, future-only semantic projection, and durable MQTT application export. |
| Site Console browser source | `iotkit-site/frontend/src/` | TypeScript browser behavior for the server-rendered Console; it does not own authorization, persistence, or domain state transitions. |
| Site Console API schema | `iotkit-site/openapi/site-console-v1.yaml` | Browser-facing JSON contract used to generate TypeScript request and response types. HTML form endpoints are not duplicated here. |
| Cross-language fixtures | `testdata/egress/v1/`, `testdata/egress/v2/` | Normative JSON examples decoded by both Rust and Go tests. Descriptor uses only `v2`; the other current egress messages remain in `v1`. |

### Layer rules (machine-checked)

1. **Adapters never depend on `core/engine`** — projection machinery is the
   Edge composition root's business (D4). Note: the frozen `AdapterEvent`/`AdapterCommand`
   vocabulary lives in `core/supervision`; rule 7 prevents new dependents,
   while reviews police usage growth inside existing dependents.
2. **Adapters reach the data plane only through `iotkit-ingest-client`** —
   never directly on storage/ledger/timeseries/publish/collector/registry/ops.
3. **`iotkit-ingest-contract`'s runtime deps are serde and nothing else** —
   third-party conformance tests must be able to depend on it alone.
4. **`core/types` and `core/storage` are leaves**, `core/supervision` depends
   only on `core/types`, `core/engine` depends only on `core/types` and
   `core/supervision`, and nothing in `core/*` depends on adapters or binaries
   (no upward edges).
5. **A new workspace crate must be classified deliberately** in
   `scripts/check-layers` (and placed on this map) — an unclassified crate
   fails CI.
6. **`iotkit-ingest-client`'s workspace dependencies are exactly
   `core/collector` + the contract** — never adapters, binaries, or
   `core/engine`.
7. **The non-dev dependent set of `core/supervision` is pinned exactly** — the
   frozen supervision vocabulary gains no new dependents without a corpus
   decision (D4/D12 decision 8). Dev-dependencies remain exempt.
8. **`INGRESS` is separate from the control API** — `iotkit-ingest-http` must not
   depend on `iotkit-edge`; its internal allowlist is the ingest contract,
   collector boundary, storage/auth services (`core/ops`), and no other workspace
   crate. IoTKit Edge composes it, never the reverse.
9. **Site consumes the wire contract, not Edge internals** — IoTKit Site shares JSON
   fixtures and schema semantics only. It never imports Edge packages or opens the Edge DB.
10. **Input-adapter supervision reachability is checked transitively** —
   RPi-local and the polling runtime must not reach `core/supervision` through
   helper crates. Only BravePI may reach it for its frozen, separate care path.

Rule numbers match the `scripts/check-layers` error messages. Edge binaries may compose their
allowed layers — with one exception: rule 7 pins the
`core/supervision` dependent set, so even a binary (today: `iotkit-edgectl`)
cannot pick up the frozen vocabulary without a deliberate rule-7 + canon update.
Dev-dependencies are exempt (tests may cross layers); build-dependencies are
checked.

### Deliberate exceptions (do not "fix" these)

- **`core/registry` → `core/collector`** looks inverted but is by design:
  the *collector* owns the `RegistryPolicy` trait (the port), and the registry
  implements it. Dependency inversion, not layering drift.
- **`iotkit-ingest-client` reaches the ingest data plane.** Its one normal core
  dependency is `core/collector` (behind the default `inproc` feature), which
  *transitively* pulls ledger, publish, storage, and timeseries — that is the
  in-proc binding (D4): official adapters get durable ingest behind the
  contract without carrying an HTTP stack. (Its other `core/*` entries are
  dev-dependencies for tests.)
- **BravePI-owned subcrates remain colocated intentionally:** `bravepi-codec`
  and `bravepi-poc` stay nested under `bravepi-mainboard-adapter/`.
- **Some custody-critical SQL lives in IoTKit Edge, deliberately.** The
  retention purge (`iotkit-edge/src/retention.rs`) and exit-record
  materialization (`src/record.rs`) join tables owned by *different* `core/*`
  crates, so no single core crate could own them today. Graduation triggers: a
  `core/retention` crate when retention gains its next feature (active
  back-pressure); `record.rs` → `core/publish` when the D9 MQTT exit binding
  needs shared materialization (update `docs/exit-contract.md`'s
  implementation pointers at the same time). Separately, the epoch-start read
  (`src/epoch_start.rs`) touches only a `core/ledger`-owned table — a
  temporary raw read queued to become `core/ledger::last_epoch_renewal()`
  (deferred ledger D-11), not a cross-crate join.
- **`core/ledger` is a deliberate single-crate aggregate** (devices, series,
  sightings, audit events, epochs share identity rules and transactions).
  Splitting its `store.rs` into modules is fine; splitting it into crates is
  rejected — it would break transaction ownership.

## Placement rules — "where does new code go?"

| You are adding… | It goes in… |
|---|---|
| Datasheet conversion reusable across acquisition paths | `iotkit-sensor-drivers` |
| A new IC using the same I2C transport, polling lifecycle, positional identity recipe, and config shape | The typed supported-device catalog in `rpi-local-adapter`; deployment config selects catalog models and settings, while Edge treats those fields opaquely and must not learn the IC model. |
| A device family with different discovery, wire protocol, security, lifecycle, identity recipe, or southbound model | A **new top-level `*-adapter` crate**. Never inside `core/*` or IoTKit Edge. |
| A change to the ingest wire (envelope fields, ack semantics, reason codes) | `iotkit-ingest-contract` **only**, with its conformance tests; consumers adapt. The wire is the contract — the Rust types follow it, not vice versa. |
| A new Edge operator / AI / UI operation that changes state | A descriptor in `core/ops` `standard_catalog()` + R14 dispatch. Never a new SQL mutation path, never a bespoke API handler with its own writes. |
| A new Site operator / UI operation that changes state | A typed operation in the Site application-service dispatcher, the Site implementation of R14. HTTP, HTML, and CLI remain thin adapters and never write SQL directly. |
| A new table / column | A migration in the **owning** `core/*` crate's version slice (the binaries concatenate the slices; the `core/storage` harness applies them by set difference). |
| A new control-plane HTTP API route | `iotkit-edge/src/api/` as a thin layer; the logic lives in the owning `core/*` crate. |
| An authenticated measurement-ingress HTTP binding | Shipped Plan 6 binding: `iotkit-ingest-http` in the `INGRESS` layer; never place it in the control-plane API module. |
| A new CLI command | `iotkit-edgectl`, calling `core/*` (state changes go through the R14 catalog, audit actor `local_cli`). |
| Site acceptance, query, sensor semantic mapping, or application-export behavior | `iotkit-site/`; communicate through versioned MQTT contracts and shared fixtures, never Edge internals. Business masters, production records, OEE, and alarms stay in applications. |
| Raw bus/pin access | `rpi4b-transport`. |
| An Edge module that has grown its own tables, is needed by both binaries, or holds more than one responsibility | **Graduate it to a new `core/<name>` crate.** IoTKit Edge is a composition root, not a home for domain logic. |

## Key data structures

Getting these right is most of the design (see D5, D7).

- **Series identity** (`series` table): a series is `UNIQUE(system_id,
  measurement_key, channel_index, variant)`.
  - `system_id` — immutable UUIDv7, issued only by the ledger. The real key.
  - `hardware_id` — the swappable physical address; unique only among *live*
    devices. A hardware swap re-points `system_id` → new `hardware_id` and
    **continues** history.
  - `user_label` — display only, never a key.
  - `channel_index` defaults to the sentinel `-1` (not NULL) to avoid SQLite's
    `UNIQUE`-treats-every-NULL-as-distinct trap.
- **Two sequences, on purpose:**
  - `readings.seq` — internal insertion order. Never leaves the box.
  - `publication_log.pub_seq` — external delivery order. A quarantined reading
    gets a `seq` immediately but no `pub_seq` until (if ever) released. The exit
    id is always `pub_seq`.
- **`(epoch, pub_seq)` record identity** — `epoch` is a restore-generation fence.
  A snapshot restore (box swap) mints a *new* epoch, so a stale consumer cursor
  from before the restore is detected (epoch mismatch → treat everything as
  unacked, re-baseline) rather than silently trusted. The exit contract never
  promises anything it can't keep across a box swap.

## Concurrency model

- **One `Arc<Mutex<Connection>>`** for the whole process (`core/storage/DbHandle`).
  Every subsystem (collector, push, retention, health, API) serializes through it
  via `spawn_blocking`. SQLite has exactly one writer anyway, so a connection pool
  would be over-engineering. WAL + `synchronous=FULL` (D8 amendment:
  custody-critical transactions must not lose an acked commit on power loss;
  NORMAL is reserved for reconstructable metadata, which today shares the same
  connection, so the whole connection runs FULL).
- **The publisher never holds the DB lock across MQTT.** It builds the batch under the lock,
  publishes and waits for application acknowledgement without the lock, then advances the cursor
  under the lock. A slow broker or IoTKit Site cannot hold the ingest DB mutex.
- **The custody-critical retention purge is one Immediate transaction** (readings
  delete + outbox prune + dedup purge + quarantine expiry + audit), internally
  chunked so a large batch doesn't build an oversized SQL statement. Housekeeping
  that must never be able to roll back that work — the `sightings` TTL/cap purge —
  runs in a **separate best-effort transaction after** the critical one commits
  (its failure is logged and retried next pass, never aborting a readings purge).

## Migrations & compatibility

`core/storage/migrate.rs` applies migrations by **set difference** of applied
versions (not a `MAX(version)` watermark), because the version-number space is
split across crates (each `core/*` owns a slice; the binaries concatenate and
sort them). It refuses to run an older binary against a newer on-disk schema
(`SchemaVersionAhead`). This is the "don't corrupt the user's data on a
downgrade" discipline.

## Where to go next

- The exit-contract wire details: [exit-contract.md](exit-contract.md).
- The authoritative rationale: `docs/redesign/` (D1–D13, R-ledger) — Japanese,
  for deep dives only.
