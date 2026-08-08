---
type: Architecture
title: "IoTKit system architecture"
description: "Defines the complete runtime architecture, data and custody flows, code placement, concurrency, and compatibility rules."
language: en
translation_key: architecture.system-overview
status: stable
revision: 15
---

# Architecture

IoTKit currently ships the Rust IoTKit Edge Node binary (`iotkit-edge-node`) plus an operator CLI
(`iotkit-edge-nodectl`), backed by a single SQLite database, and the independently deployable Rust
IoTKit Edge (`iotkit-edge`), backed by one selected `embedded` (SQLite) or `postgres`
(PostgreSQL) storage profile and a standard MQTT broker.
IoTKit Edge is not part of the Edge Node process. Edge Node runs unattended on a
Raspberry Pi under systemd. This document is the "get oriented in 10 minutes"
map **and the canon for code placement**. The product boundary is defined in
[product model](../concepts/product-model.md), and the [English documentation index](../index.md) defines the
source-of-truth order. `docs/redesign/` preserves historical rationale and does
not override current executable contracts or this architecture.

## Who this serves

Every structural choice below is graded against five audiences. When a change
makes life worse for one of them, that is a review finding, not a taste issue.

| Audience | What they touch | What "good" means for them |
|---|---|---|
| **IoTKit Edge installers & operators** | The install story, `iotkit-edge-nodectl`, the API/UI, error messages | They can tell *what to install on which device and how to assemble a deployment* from one page. Errors name the thing to check next. Defaults are safe; nothing silently loses data; a Pi is fast enough. |
| **Self-made-device builders** (ESP32 hobbyists, plant engineers — often not Rust readers) | The ingest **wire contract only** | Onboarding stays a curl-3-lines experience. Contract docs never require reading Rust. Rejections come back with a reason code they can act on. |
| **Adapter developers** (Rust) | `edge-node/core/types`, `iotkit-input-adapter-host-api`, `iotkit-input-adapter-testkit`, `iotkit-polling-adapter-runtime`, an existing adapter as template | The adapter boundary is obvious; a new sensor family means a new adapter crate, not core surgery. No knowledge of storage/ledger internals needed. |
| **Core contributors** (Rust) | `edge-node/core/*`, IoTKit Edge Node, tests | The crate map fits in one screen. Layer rules are machine-checked, not tribal. Each crate has one responsibility; tests read as the executable spec. |
| **Raw custody implementers** | The **Edge Node custody contract** | Record families, ack rules, and cursor semantics are documented and versioned; no schema surprises. |
| **Application integrators** (Pinikiet, dashboards, analytics) | The **Output Adapter contract** | They receive application-facing topics and payloads without depending on the raw custody stream. |

## IoTKit Edge anatomy — what runs where

The [product model](../concepts/product-model.md) describes four tiers of a deployment:
**[1] devices** (sensors/actuators in the field, incl. third-party self-made
hardware) → **[2] IoTKit Edge Node** (Raspberry Pi, this repo) → **[3] IoTKit Edge**
(per-deployment aggregation; may be hosted remotely) → **[4] Fleet layer**.

**This repository ships tiers [2] and [3].** Tier [3] is a separate Rust program
that consumes only the public Edge Node custody contract; it does not import Edge Node Rust packages or read
the Edge Node database. Tier [1] is hardware plus the ingest wire contract. Tier [4] remains external
and spans multiple `edge_id` values regardless of whether it runs in cloud or on premises.
So a minimal Edge Node install is: flash a Pi, run the `iotkit-edge-node` daemon
under systemd, keep `iotkit-edge-nodectl` on the same Pi as a hand-run CLI, and wire
adapters to sensors. A standalone deployment can stop there (D8: an upstream is
optional). IoTKit Edge adds durable aggregation, Edge Node cursors, direct raw
query, Edge Node descriptor replica, configurable Edge-scoped sensor meaning, and the application-export boundary. IoTKit Edge maps one
stored series to one generic typed meaning such as a cumulative value; a separate exporter converts the
result to an application-facing MQTT contract. Applications such as Pinikiet own business masters
and logic such as products, processes, OEE, alarms, business UI, and notifications. Anything that
complicates this story needs a strong reason.

The exporter boundary is the versioned
[Output Adapter contract v1](../contracts/output-adapter-v1.md). An Output Adapter is a deterministic
in-process transformer from a generic IoTKit Edge observation plus route configuration to one exact MQTT
publication. It never owns Broker connectivity, credentials, durable outbox state, retries, or
business masters. `pinikiet.mqtt.v1` is the first implementation, not a privileged core path.

The current production-shaped reference installation keeps Edge Node native on its Raspberry Pi and
co-locates the standard Broker plus IoTKit Edge in Docker on one Linux host. Co-location is not a product
requirement: the Broker and IoTKit Edge may run on separate hosts and communicate only through the same
authenticated MQTT/TLS contract, without a shared filesystem or Compose project. The current
`scripts/bootstrap-edge.sh` consumes the
non-secret `iotkit-edge-nodectl mqtt-binding` document and operator-provided TLS material, then creates
an anonymous-disabled Broker, exact per-Edge-Node ACLs, owner-only credential files, and a small
Edge Node handoff. It does not issue certificates, configure DNS/firewalls/VPNs, or modify the Edge Node.
`deploy/compose.edge.yaml` consumes only generated file paths and non-secret network settings; it
does not place MQTT credentials in Compose environment values or argv. A split deployment must
produce separate Broker-host, IoTKit Edge-client, and per-Edge Node-client artifacts. IoTKit Edge has its own Broker
principal and credential even when it is co-located with the Broker.

The target IoTKit Edge storage architecture has two implemented profiles that satisfy the
same product contract: `embedded` (SQLite) and `postgres` (PostgreSQL). The SQLite file
lives on local storage on the same host as the IoTKit Edge process. One Edge never
dual-writes both databases or falls back to an empty backend after a failure. The profile
is fixed at installation. Moving from SQLite to PostgreSQL is an offline operation with a
consistent backup and verification of every identity, cursor, and outbox. Storage behavior
and placement follow the [IoTKit Edge operations](../operations/installation-and-recovery.md)
and [capacity runbook](../operations/storage-capacity.md). TimescaleDB or a similar extension
is not a third authority; it may be considered inside the `postgres` profile only after
measurements show a need.

`deploy/mosquitto-image.env` is the repository's single source for the verified Mosquitto patch
release used by production generation, Compose, and integration tests. Updating that exact patch
reference requires the MQTT security matrix and the normal final verification gate; floating
major/minor tags are not production inputs.

## Data flow

The deployed BravePI path is `BravePI Mainboard -> UART -> IoTKit Edge Node -> MQTT Broker -> IoTKit Edge`.
Inside Edge Node, the adapter and collector normalize and durably enqueue observations before publishing:

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
             │ R10 custody│      │ R17 custody- │     │ R12 status   │
             │ contract   │      │ aware purge  │     │ JSON         │
             └─────┬──────┘      └──────────────┘     └──────────────┘
                   │ MQTT QoS 1 (transport PUBACK only)
                   ▼
             standard MQTT broker
                   │
                   ▼
             IoTKit Edge ── durable accepted-through topic ──▶ authorizes purge
```

On the same Broker, an Edge Node sends a schema-2 complete snapshot assembled from its
ledger and registry to its Edge-Node-specific `descriptors` topic as a retained QoS 1
message of at most 1 MiB. It includes only an optional `model_id` explicitly persisted for
the device. IoTKit Edge validates schema 2 and its revision and epoch before replicating it
into dedicated tables. Adapter instances, physical locators, and hardware or provider
identifiers do not cross this boundary. This path is independent of the publication outbox,
raw transaction, and `accepted-through` cursor, so custody processing continues if it fails.

Broker enrollment does not activate an Edge Node. Before activation, an Edge Node keeps
normalized observations locally, assigns no publication sequence, and sends no records.
IoTKit Edge discovers an Edge Node from its descriptor and activates the exact ledger epoch
through an admin typed operation. Using the same SQLite write serialization as the collector,
the Edge Node fixes the boundary once and places only later ingest into the outbox. IoTKit
Edge validates activation and atomically stores raw data and advances the cursor only after
committing the matching activation result and marking the incarnation active. Physical
deletion of the pre-registration prefix is Edge-Node-local cleanup against the fixed boundary;
it changes neither `accepted-through` nor post-activation purge authority.

`mqtt_publish_task` is the active production exit binding. The older `publish_task` HTTPS code is
retained only as transitional code and is not spawned. A broker PUBACK confirms transport receipt
only; IoTKit Edge Node retains its outbox until IoTKit Edge commits raw records and publishes
application-level `accepted-through`.

### IoTKit Edge semantic and application-export loop

While `iotkit-edge serve` consumes raw batches, an independent 100 ms convergence loop projects
durable rule-record work from `semantic_projection_queue`, enqueues versioned application events in
the IoTKit Edge outbox, and publishes pending rows at MQTT QoS 1. Raw acceptance atomically adds a
queue row for every matching active rule and snapshots that rule and calibration revision. Candidate
selection is therefore proportional to pending work: it orders the queue, then joins immutable raw
records and those snapshots; it never rescans retained raw history or receipt history. A queue row is
one pending rule-record pair, not a raw-record count or receipt lag.

One projection transaction creates the observation, routes its outbox rows, writes the durable receipt
and runtime state, then removes the queue row. A poison input writes its failure and terminal receipt
before removing that row. Any other failure rolls all of that back, leaving the queue row retryable;
receipts remain the durable idempotency authority. Pending counter-reset boundaries fence later queue
rows for that rule until the bounded pre-reset work drains. Each loop tick admits at most 16 items and
stops admitting another after 20 ms; one in-flight transaction can exceed that wall-time budget. It
checks cancellation and yields between items so login, diagnostics, and custody work remain available
during recovery. Only a successful PUBACK marks an
outbox row published; failure or the 15-second timeout leaves it pending for a later tick. A
transactional projection or enqueue failure rolls its changes back and leaves the queue row
restart-retryable. A critical storage or projection-task failure is logged without payloads or
credentials and cancels service rather than silently continuing the loop.

This is deliberately a two-stage failure boundary. The raw batch transaction and its
`accepted-through` publish never wait for semantic projection or application export, so an
application outage cannot hold Edge Node custody. Semantic mappings are future-only from each current
Edge Node cursor, and MQTT routes are future-only from the current semantic-event boundary.

Operators use JSON-producing CLI commands backed by the typed IoTKit Edge application-service dispatcher
(the raw `query` command remains available). Semantic mapping changes and the legacy MQTT route
command commit their success audit in the same authoritative-store transaction as the setting change.
The following commands show the legacy embedded-profile interface:

```bash
iotkit-edge mapping-set --db edge.db --edge-node-id edge-node-01 \
  --series-key '<series_key>' --meaning production_pulse \
  --trigger-mode active_edge --active-value 1
iotkit-edge mapping-deactivate --db edge.db --edge-node-id edge-node-01 \
  --series-key '<series_key>'
iotkit-edge mapping-list --db edge.db
iotkit-edge route-add --db edge.db --mapping-id '<mapping_id>' \
  --topic 'iotkit/v1/application/production-pulses'
iotkit-edge semantic-query --db edge.db --limit 100
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
adapter-lifecycle supervision into `edge-node/core/engine`'s device-state projection,
and `AdapterCommand` carries shutdown plus legacy southbound commands into
the adapter runtimes. Both are defined in `edge-node/core/supervision`. The freeze is
machine-enforced for **new crate dependents**: `scripts/check-layers` rule 7
pins the complete dependent set. Reviews still police usage growth inside the
existing dependent crates (D4/D12 decision 8).

The implemented compile-time northbound extension boundary for official in-process
sensor adapters is [Input Adapter host contract v1](../contracts/input-adapter-v1.md). Adapter type,
configured instance, diagnostic source, receiver-owned principal, observed
subject, and ledger-owned system identity remain separate. The shared runtime
host/composition API carries no `AdapterEvent`/`AdapterCommand`; Edge Node-private factories
and package-private legacy projections isolate the frozen vocabulary while Edge Node retains principal creation,
configuration authority, restart policy, and health aggregation.

## The custody loop (the core idea)

IoTKit Edge Node is a **buffer, not a warehouse**. A measurement's lifecycle:

1. **Ingest** — the collector always decides local reading acceptance first. For
   an active Edge Node incarnation, a non-quarantined measurement that passes
   publication admission is written with its `publication_log` outbox row in the
   *same* SQLite transaction. Pre-activation readings remain local without a
   publication row and are never backfilled into the custody stream. Quarantined
   readings have no publication row while quarantined; a later release must pass
   the durable activation and publication-admission gate before enqueue. For
   publishable readings, crash consistency guarantees that the reading and its
   outbox row appear together or not at all.
2. **Publish** — the publisher batches undelivered outbox rows and sends them through a standard
MQTT broker with a per-Edge-Node credential. The DB lock is **not** held across the network
   round-trip. Broker PUBACK does not release application custody.
3. **Ack → cursor** — after committing records and cursor atomically in its selected authoritative
   store, IoTKit Edge publishes a valid `accepted-through` ack
   (matching Edge Node, epoch, publication id, and batch bound), which
   advances the per-target cursor. The cursor is the consumer's durable
   watermark: "I have taken custody up to here."
4. **Purge/degrade** — normal retention deletes archive-acknowledged data beyond the minimum floor.
   Under pressure the authoritative D2/R17 order is: acknowledged data, out-of-custody-policy data,
   unresolved quarantine, then unacknowledged originals only as the final explicit data-loss class.
   Reaching the last class requires `custody_lost` audit plus a structured gap annotation; silent
   deletion is forbidden.

If the consumer is down, the cursor stops advancing, the backlog grows, and disk
fills — at which point *new writes fail loudly* (`ENOSPC`). IoTKit Edge Node never
silently drops stored data to make room. (Graceful active back-pressure is future
work; today the contract is "safe, not graceful" under sustained pressure.)

Installers can enqueue the optional `commissioning_smoke` record through the R14 operation-backed
`iotkit-edge-nodectl smoke enqueue` command and compare its epoch/publication sequence with the
`accepted-through` cursor through `smoke status`. This verifies the normal custody path without
direct SQLite access or pretending that a physical sensor produced a measurement.

## Control plane

Since plan 5 IoTKit Edge Node also runs an **HTTPS API server** (axum + rustls,
self-signed certificate whose SHA-256 fingerprint is surfaced for pinning;
private-address clients only). State changes ride the **R14 typed operation
catalog** (`edge-node/core/ops`): each operation is a descriptor with a permission tier
(`Routine` / `Daily` / `Construction`), dry-run support, and unconditional
audit into `ledger_events`. Today the API's mutation routes go through catalog
dispatch. The plan-5 CLI commands reuse the same `edge-node/core/ops` functions —
`token issue|revoke` call exactly what the catalog descriptors call, and
`passphrase reset` is a deliberate out-of-band recovery path — each audited
as `local_cli`. Migrating the older CLI mutation paths (targets, snapshots,
replace-hardware) onto R14 is queued across plans 7–8. Session login is the
only network endpoint here that changes state outside R14 by design. Authentication is an admin
passphrase (argon2id) plus operator tokens (issued once in plaintext, stored as
SHA-256; AI tokens are structurally capped at `Routine`). Initial ownership and
admin recovery are local-root maintenance commands: an unowned/recovering box
does not bind the control API/UI, and `iotkit-edge-nodectl passphrase reset` atomically
replaces the credential and revokes all operator/session tokens. There is no
network setup route or unauthenticated setup allowlist. The prescriptive rule:
**a new mutation surface is an R14 descriptor — never a fresh SQL mutation
path.** Local-root ownership/recovery (and the separately specified factory-reset
maintenance family) are explicit non-network exceptions, never API/UI/AI operations.

Optional Edge Node recovery filesystem operations trust local root and the
effective owner as one principal. Their configuration parent and protected
files/directories deny all group/other access. Supported configure,
destination verification/probe, publication, and retention calls hold one
stable owner-only config-adjacent nonblocking lock across the operation; a
second supported call fails as `operation_busy`. This lock coordinates product
code, not hostile code already running with the same effective UID. Such code
is outside the filesystem namespace protection boundary and requires host
containment.

## Current implementation state

The repository currently ships the first network measurement path as a separate,
default-off `iotkit-ingest-http` listener. It accepts authenticated JSON
envelopes over a local-network TLS binding, applies finite header/body/time/queue and
admission limits, returns custody-correct acknowledgements, and exposes the
side-effect-free `/api/v1/ingest/validate` endpoint. Its principal, staging,
deduplication, health, and episode-audit boundaries are distinct from the
control API.

The current v1 candidate supports BravePI temperature/contact input through the existing Long
Range BLE/BravePI Mainboard/UART path and generic Input Adapter/driver boundaries, multiple
IoTKit Edge Nodes, one standard MQTT Broker, one IoTKit Edge, one selected
embedded SQLite or PostgreSQL raw store,
application-level accepted-through, future-only semantic projection, durable Output Adapter MQTT
outboxes, an authenticated IoTKit Console, a bounded live dashboard of processed results for each saved active `cumulative_counter` measurement rule (numeric, boolean, and alarm rule cards, plus ruleless signals, are omitted and a dashboard-level setup message is shown only when no active cumulative rules exist), bounded history graphs, and generic CSV export. BravePI owns
BLE, pairing through its existing iOS application, and transmitter management; IoTKit starts at the
BravePI Mainboard UART stream. A production-shaped multi-Edge Node bootstrap exists for the Broker/IoTKit Edge
TLS boundary. The Broker-host certificate component validates and atomically installs bundles,
supports `lego` ACME renewal, probes MQTT/HTTPS, and rolls back a failed install. IoTKit Edge has local
accounts, factual storage/diagnostic views, and encrypted backup/new-path restore with explicit
archive-gap recovery. Pinikiet remains outside IoTKit and is reached through its versioned Output
Adapter contract. Short-lived credential enrollment/rotation and retained replay for a restored
archive gap remain post-v1 hardening work.

## Crate map

Thirty-two crates, five layers. `scripts/check-layers` enforces the layer rules
below mechanically (in `verify.sh` and CI).

| Crate | Path | Responsibility (one line) |
|---|---|---|
| `iotkit-core-types` | `edge-node/core/types` | Domain entity types (no protocol specifics). Leaf. |
| `iotkit-ingest-contract` | `edge-node/ingest/contract` | Ingest wire contract v1: `Envelope`/`Ack`/reason codes. The wire is normative; runtime deps = serde only (serde_json appears only in dev-dependencies). Leaf. |
| `iotkit-core-storage` | `edge-node/core/storage` | SQLite handle (`DbHandle`) + cross-crate migration harness. Leaf. |
| `iotkit-core-supervision` | `edge-node/core/supervision` | Frozen supervision / legacy-southbound `AdapterEvent`/`AdapterCommand` vocabulary (D4/D12). Depends only on `types`; its dependent set is pinned by rule 7. |
| `iotkit-core-engine` | `edge-node/core/engine` | In-memory device-state projection consuming the frozen `AdapterEvent` vocabulary from `edge-node/core/supervision`. Depends only on `types` and `supervision`; adapters must never depend on it. |
| `iotkit-core-ledger` | `edge-node/core/ledger` | Device ledger: `system_id` issuance, series identity, sightings, epochs, audit events. |
| `iotkit-core-timeseries` | `edge-node/core/timeseries` | `readings` + staged readings persistence, event-time derivation, queries. |
| `iotkit-core-publish` | `edge-node/core/publish` | Exit-contract data layer: Edge Node activation admission, `publication_log` (outbox), `target_registry`, cursors. |
| `iotkit-core-collector` | `edge-node/core/collector` | Ingest actor: dedup, series resolution, quarantine and activation admission, active-record same-tx outbox enqueue. Owns the `RegistryPolicy` trait. |
| `iotkit-core-registry` | `edge-node/core/registry` | D6 measurement registry (standard catalog + deployment overrides); implements `RegistryPolicy`. |
| `iotkit-core-ops` | `edge-node/core/ops` | R14 operation catalog, permission tiers, auth store (passphrase/tokens), dispatch + audit. |
| `iotkit-core-recovery` | `edge-node/core/recovery` | Optional Edge Node backup/recovery durable state, complete migration set, read-only startup fence probe, and recovery-model redaction boundary. |
| `iotkit-ingest-client` | `edge-node/ingest/client` | The ingest-contract client adapters use (D4). In-process binding for official adapters; network device builders use the separate HTTP binding. MQTT remains future. |
| `iotkit-input-adapter-host-api` | `edge-node/input/host-api` | Supervision-free official adapter composition API: validated identities, source-bound ingest, delivery receipts/retry, bounded diagnostics/activity, completion, and shutdown. |
| `iotkit-input-adapter-testkit` | `edge-node/input/testkit` | Dev-only conformance assertions and a non-catalog two-subject/two-measurement reference adapter. |
| `iotkit-ingest-http` | `edge-node/ingest/http` | **INGRESS.** Listener parsing, exposure/TLS validation, accepted-peer checks, and transport construction; never control-API routes or measurement domain logic. |
| `iotkit-polling-adapter-runtime` | `edge-node/input/runtimes/polling` | Supervision-, ingest-, mapping-free I2C polling engine; emits decoded observations and lifecycle facts. The current name is retained for compatibility but its public SPI is I2C-specific. |
| `rpi4b-transport` | `edge-node/input/hardware/transports/rpi` | Raw host I/O. I2C exposes an injectable device/factory boundary and combined write-read; GPIO/SPI/PWM use Raspberry Pi `rppal`. The historical crate name does not limit support to Pi 4B. |
| `iotkit-sensor-drivers` | `edge-node/input/hardware/sensor-drivers` | Vendor-neutral per-sensor-IC constants, identity metadata, and datasheet conversion components shared by adapters; not a complete transport-owning driver. |
| `bravepi-codec` | `edge-node/adapters/bravepi-mainboard/codec` | BravePI frame encoding/decoding. |
| `bravepi-mainboard-adapter` | `edge-node/adapters/bravepi-mainboard` | BravePI-protocol adapter: transport + codec + sensor drivers → Envelopes. |
| `rpi-local-adapter` | `edge-node/adapters/rpi-local` | Direct Linux I2C adapter under its accepted `rpi-local` compatibility name; owns the typed supported-device catalog, driver construction, measurement projection, and inventory metadata. |
| `trial-sample-adapter` | `edge-node/adapters/trial-sample` | Explicitly configured local-only sample Input Adapter for the trial profile; emits two series through the standard adapter host and custody path: continuous triangle-wave illuminance and square-wave contact state. Field enablement requires `IOTKIT_ENABLE_TRIAL_SAMPLE=1`, and inventory model ids are non-hardware (`trial-sample-illuminance`, `trial-sample-contact`). |
| `bravepi-poc` | `edge-node/tools/bravepi-poc` | Hardware proof-of-concept harness for BravePI (dev tool, not shipped). |
| `iotkit-edge-node` | `edge-node/apps/node` | **Binary.** IoTKit Edge Node composition root: adapter supervision, MQTT exit publisher, retention, health, HTTPS API. |
| `iotkit-edge-nodectl` | `edge-node/apps/nodectl` | **Binary.** Edge Node operator CLI: ledger, registry, snapshots, targets, tokens (audited; plan-5 commands reuse the `edge-node/core/ops` functions; older mutation paths migrate to R14 in plans 7–8). |
| `iotkit-edge` | `edge/` | **Binary and library.** Rust composition root for MQTT custody, storage, semantics, Output Adapters, authenticated Console, backup, diagnostics, and operator CLI. |
| `iotkit-edge-custody-contract` | `edge/custody-contract` | Leaf Rust representation and strict validation of the versioned Edge Node MQTT descriptor, activation, record-batch, and custody acknowledgement wire messages. |
| `iotkit-output-adapter-api` | `edge/output-adapters/api` | Leaf Rust API for deterministic Observation-to-MQTT transformation and provider-neutral profile setup policy. |
| `iotkit-output-adapter-testkit` | `edge/output-adapters/testkit` | Dev-only shared descriptor, configuration, publication, and determinism conformance assertions. |
| `iotkit-output-adapter-example` | `edge/output-adapters/example` | Compile-tested vendor-neutral author example; deliberately absent from the production registry. |
| `iotkit-output-adapter-generic-mqtt-json-v1` | `edge/output-adapters/generic-mqtt-json-v1` | Built-in generic IoTKit Observation JSON transformer. |
| `iotkit-output-adapter-pinikiet-mqtt-v1` | `edge/output-adapters/pinikiet-mqtt-v1` | Built-in Pinikiet MQTT contract transformer and profile policy. |

Approved non-crate placement:

| Component | Path | Responsibility (one line) |
|---|---|---|
| IoTKit Console browser source | `edge/frontend/src/` | TypeScript browser behavior for the server-rendered Console; it does not own authorization, persistence, or domain state transitions. |
| IoTKit Console API schema | `edge/openapi/edge-console-v1.yaml` | Browser-facing JSON contract used to generate TypeScript request and response types. HTML form endpoints are not duplicated here. |
| Wire fixtures | `testdata/egress/v1/`, `testdata/egress/v2/` | Normative JSON examples decoded by Rust conformance tests. Descriptor uses only `v2`; the other current egress messages remain in `v1`. |

### Layer rules (machine-checked)

1. **Adapters never depend on `edge-node/core/engine`** — projection machinery is the
   Edge Node composition root's business (D4). Note: the frozen `AdapterEvent`/`AdapterCommand`
   vocabulary lives in `edge-node/core/supervision`; rule 7 prevents new dependents,
   while reviews police usage growth inside existing dependents.
2. **Adapters reach the data plane only through `iotkit-ingest-client`** —
   never directly on storage/ledger/timeseries/publish/collector/registry/ops.
3. **`iotkit-ingest-contract`'s runtime deps are serde and nothing else** —
   third-party conformance tests must be able to depend on it alone.
4. **`edge-node/core/types` and `edge-node/core/storage` are leaves**, `edge-node/core/supervision` depends
   only on `edge-node/core/types`, `edge-node/core/engine` depends only on `edge-node/core/types` and
   `edge-node/core/supervision`, and nothing in `edge-node/core/*` depends on adapters or binaries
   (no upward edges).
5. **A new workspace crate must be classified deliberately** in
   `scripts/check-layers` (and placed on this map) — an unclassified crate
   fails CI.
6. **`iotkit-ingest-client`'s workspace dependencies are exactly
   `edge-node/core/collector` + the contract** — never adapters, binaries, or
   `edge-node/core/engine`.
7. **The non-dev dependent set of `edge-node/core/supervision` is pinned exactly** — the
   frozen supervision vocabulary gains no new dependents without a corpus
   decision (D4/D12 decision 8). Dev-dependencies remain exempt.
8. **`INGRESS` is separate from the control API** — `iotkit-ingest-http` must not
   depend on `iotkit-edge-node`; its internal allowlist is the ingest contract,
   collector boundary, storage/auth services (`edge-node/core/ops`), and no other workspace
   crate. IoTKit Edge Node composes it, never the reverse.
9. **IoTKit Edge consumes the wire contract, not Edge Node internals** — IoTKit Edge shares JSON
   fixtures and schema semantics only. It never imports Edge Node packages or opens the Edge Node DB.
10. **Input-adapter supervision reachability is checked transitively** —
   RPi-local and the polling runtime must not reach `edge-node/core/supervision` through
   helper crates. Only BravePI may reach it for its frozen, separate care path.

Rule numbers match the `scripts/check-layers` error messages. Edge Node binaries may compose their
allowed layers — with one exception: rule 7 pins the
`edge-node/core/supervision` dependent set, so even a binary (today: `iotkit-edge-nodectl`)
cannot pick up the frozen vocabulary without a deliberate rule-7 + canon update.
Dev-dependencies are exempt (tests may cross layers); build-dependencies are
checked.

### Deliberate exceptions (do not "fix" these)

- **`edge-node/core/registry` → `edge-node/core/collector`** looks inverted but is by design:
  the *collector* owns the `RegistryPolicy` trait (the port), and the registry
  implements it. Dependency inversion, not layering drift.
- **`iotkit-ingest-client` reaches the ingest data plane.** Its one normal core
  dependency is `edge-node/core/collector` (behind the default `inproc` feature), which
  *transitively* pulls ledger, publish, storage, and timeseries — that is the
  in-proc binding (D4): official adapters get durable ingest behind the
  contract without carrying an HTTP stack. (Its other `edge-node/core/*` entries are
  dev-dependencies for tests.)
- **BravePI-owned code remains grouped intentionally:** `bravepi-codec` stays
  nested under `edge-node/adapters/bravepi-mainboard/`; the hardware-only PoC
  lives under `edge-node/tools/bravepi-poc/` so it is not confused with shipped
  adapter code.
- **Some custody-critical SQL lives in IoTKit Edge Node, deliberately.** The
  retention purge (`edge-node/apps/node/src/retention.rs`) and exit-record
  materialization (`src/record.rs`) join tables owned by *different* `edge-node/core/*`
  crates, so no single core crate could own them today. Graduation triggers: a
  `edge-node/core/retention` crate when retention gains its next feature (active
  back-pressure); `record.rs` → `edge-node/core/publish` when the D9 MQTT exit binding
  needs shared materialization (update `docs/exit-contract.md`'s
  implementation pointers at the same time). Separately, the epoch-start read
  (`src/epoch_start.rs`) touches only a `edge-node/core/ledger`-owned table — a
  temporary raw read queued to become `edge-node/core/ledger::last_epoch_renewal()`
  (deferred ledger D-11), not a cross-crate join.
- **`edge-node/core/ledger` is a deliberate single-crate aggregate** (devices, series,
  sightings, audit events, epochs share identity rules and transactions).
  Splitting its `store.rs` into modules is fine; splitting it into crates is
  rejected — it would break transaction ownership.

## Placement rules — "where does new code go?"

Choose the integration boundary before choosing a crate:

1. If a device already emits the versioned Envelope/Ack contract, connect it to
   `edge-node/ingest/http`; no Rust Input Adapter is needed.
2. If a new sensor IC fits the existing direct-I2C transport, polling lifecycle,
   identity, and configuration model, extend `edge-node/adapters/rpi-local`.
3. If discovery, wire protocol, security, lifecycle, or identity differs, create
   a sibling family under `edge-node/adapters/` and implement the host contract.

| You are adding… | It goes in… |
|---|---|
| Datasheet conversion reusable across acquisition paths | `edge-node/input/hardware/sensor-drivers` |
| A new IC using the same I2C transport, polling lifecycle, positional identity recipe, and config shape | The typed supported-device catalog in `edge-node/adapters/rpi-local`; deployment config selects catalog models and settings, while Edge Node treats those fields opaquely and must not learn the IC model. |
| A device family with different discovery, wire protocol, security, lifecycle, identity recipe, or southbound model | A new sibling crate under `edge-node/adapters/`. Never inside `edge-node/core/` or an app composition root. |
| A change to the ingest wire (envelope fields, ack semantics, reason codes) | `edge-node/ingest/contract` **only**, with its conformance tests; consumers adapt. The wire is the contract — the Rust types follow it, not vice versa. |
| A new Edge Node operator / AI / UI operation that changes state | A descriptor in `edge-node/core/ops` `standard_catalog()` + R14 dispatch. Never a new SQL mutation path, never a bespoke API handler with its own writes. |
| A new IoTKit Edge operator / UI operation that changes state | A typed operation in the IoTKit Edge application-service dispatcher, the IoTKit Edge implementation of R14. HTTP, HTML, and CLI remain thin adapters and never write SQL directly. |
| A new table / column | A migration in the **owning** `edge-node/core/*` crate's version slice (the binaries concatenate the slices; the `edge-node/core/storage` harness applies them by set difference). |
| A new control-plane HTTP API route | `edge-node/apps/node/src/api/` as a thin layer; the logic lives in the owning `edge-node/core/*` crate. |
| An authenticated measurement-ingress HTTP binding | `iotkit-ingest-http` in the `INGRESS` layer; never place it in the control-plane API module. |
| A new CLI command | `iotkit-edge-nodectl`, calling `edge-node/core/*` (state changes go through the R14 catalog, audit actor `local_cli`). |
| IoTKit Edge acceptance, query, sensor semantic mapping, or application-export behavior | `edge/`; communicate through versioned MQTT contracts and shared fixtures, never Edge Node internals. Business masters, production records, OEE, and alarms stay in applications. |
| Raw bus/pin access | `rpi4b-transport`. |
| An Edge Node module that has grown its own tables, is needed by both binaries, or holds more than one responsibility | **Graduate it to a new `edge-node/core/<name>` crate.** IoTKit Edge Node is a composition root, not a home for domain logic. |

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
  Slice-1 fenced-candidate restore does not mint or activate a new epoch: the
  candidate cannot collect or publish. A production same-ID box swap and its
  new-epoch/stale-cursor handling are later permit and reconciliation contract
  behavior, not a shipped restore operation. The custody contract never promises
  anything it cannot keep across that future cutover.

## Concurrency model

- **One `Arc<Mutex<Connection>>`** for the whole process (`edge-node/core/storage/DbHandle`).
  Every subsystem (collector, push, retention, health, API) serializes through it
  via `spawn_blocking`. SQLite has exactly one writer anyway, so a connection pool
  would be over-engineering. WAL + `synchronous=FULL` (D8 amendment:
  custody-critical transactions must not lose an acked commit on power loss;
  NORMAL is reserved for reconstructable metadata, which today shares the same
  connection, so the whole connection runs FULL).
- **The publisher never holds the DB lock across MQTT.** It builds the batch under the lock,
  publishes and waits for application acknowledgement without the lock, then advances the cursor
  under the lock. A slow broker or IoTKit Edge cannot hold the ingest DB mutex.
- **The custody-critical retention purge is one Immediate transaction** (readings
  delete + outbox prune + dedup purge + quarantine expiry + audit), internally
  chunked so a large batch doesn't build an oversized SQL statement. Housekeeping
  that must never be able to roll back that work — the `sightings` TTL/cap purge —
  runs in a **separate best-effort transaction after** the critical one commits
  (its failure is logged and retried next pass, never aborting a readings purge).

## Migrations & compatibility

`edge-node/core/storage/migrate.rs` applies migrations by **set difference** of applied
versions (not a `MAX(version)` watermark), because the version-number space is
split across crates (each `edge-node/core/*` owns a slice; the binaries concatenate and
sort them). It refuses to run an older binary against a newer on-disk schema
(`SchemaVersionAhead`). This is the "don't corrupt the user's data on a
downgrade" discipline.

## Where to go next

- The complete reading path and authority order: [English documentation](../index.md).
- The Edge Node -> IoTKit Edge custody details: [Edge Node custody contract](../contracts/edge-node-custody-v1.md).
- The IoTKit Edge -> application boundary: [Output Adapter contract](../contracts/output-adapter-v1.md).
- Historical rationale remains outside this public current corpus and is for deep dives only.
