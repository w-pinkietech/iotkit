# Architecture

IoTKit is one Rust binary (`iotkit-gateway`) plus an operator CLI
(`iotkit-gatewayctl`), backed by a single SQLite database. It runs unattended on
a Raspberry Pi under systemd. This document is the "get oriented in 10 minutes"
map **and the canon for code placement**; the authoritative *why* is the
Japanese design corpus under `../docs/redesign/` (decision records D1–D13,
responsibility ledger R1–R23).

## Who this serves

Every structural choice below is graded against five audiences. When a change
makes life worse for one of them, that is a review finding, not a taste issue.

| Audience | What they touch | What "good" means for them |
|---|---|---|
| **Site installers & operators** | The install story, `gatewayctl`, the API/UI, error messages | They can tell *what to install on which device and how to assemble a site* from one page. Errors name the thing to check next. Defaults are safe; nothing silently loses data; a Pi is fast enough. |
| **Self-made-device builders** (ESP32 hobbyists, plant engineers — often not Rust readers) | The ingest **wire contract only** | Onboarding stays a curl-3-lines experience. Contract docs never require reading Rust. Rejections come back with a reason code they can act on. |
| **Adapter developers** (Rust) | `core/types`, `iotkit-ingest-client`, `iotkit-polling-adapter-runtime`, an existing adapter as template | The adapter boundary is obvious; a new sensor family means a new adapter crate, not core surgery. No knowledge of storage/ledger internals needed. |
| **Core contributors** (Rust) | `core/*`, the gateway, tests | The crate map fits in one screen. Layer rules are machine-checked, not tribal. Each crate has one responsibility; tests read as the executable spec. |
| **Data consumers** (dashboards, Node-RED, analytics) | The **exit contract** | The record schema, ack rules, and cursor semantics are documented and versioned; no schema surprises. |

## Site anatomy — what runs where

The design corpus describes four tiers of a deployment (terminology.md):
**[1] devices** (sensors/actuators in the field, incl. third-party self-made
hardware) → **[2] the IoT gateway** (Raspberry Pi, this repo) → **[3] the site
server** (per-site aggregation; may be hosted off-premises) → **[4] cloud**.

**This repository ships software for tier [2] only**: the `iotkit-gateway`
daemon and the `iotkit-gatewayctl` CLI, both installed on the gateway Pi.
Tier [1] is hardware plus (for network devices, Wave 1) the ingest wire
contract; tiers [3]/[4] are *consumers* of the exit contract
([exit-contract.md](exit-contract.md)) — external software, not built here.
So a minimal site install is: flash a Pi, run the `iotkit-gateway` daemon
under systemd, keep `gatewayctl` on the same Pi as a hand-run CLI, wire
adapters to sensors. A standalone site can stop there (D8: an upstream is
optional); pointing an archive consumer at the exit contract is how a site
gains one. Anything that complicates this story needs a strong reason.

## Data flow

```
  ┌─────────────┐    Envelope       ┌───────────────┐
  │  adapters   │ ────────────────▶ │   collector   │  R8: dedup, series resolution,
  │ (BravePI,   │  (in-process      │   (ingest)    │      quarantine decision, and —
  │  rpi-local) │   ingest client)  └──────┬────────┘      in the SAME tx — outbox enqueue
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
                   │ HTTPS POST (per-target token, at-least-once)
                   ▼
            archive consumer  ── ack (cursor) ──▶ authorizes purge
```

Adapters speak the **ingest contract** (`Envelope`/`Ack`, crate
`iotkit-ingest-contract`) through `iotkit-ingest-client`. Today the only
binding is in-process; Wave 1 adds a network ingress (HTTP) speaking the same
contract with per-device tokens. `AdapterEvent`/`AdapterCommand` are a
*frozen* legacy vocabulary, **not** the ingest path: `AdapterEvent` carries
adapter-lifecycle supervision into `core/engine`'s device-state projection,
and `AdapterCommand` carries shutdown plus legacy southbound commands into
the adapter runtimes. Both are defined in `core/types` (moving them to a
dedicated home is queued structural homework, D12 decision 8); because they
sit inside a crate everyone may use, the freeze is **review-enforced**, not
machine-enforced: new code must not grow new uses of them (D4).

## The custody loop (the core idea)

The gateway is a **buffer, not a warehouse**. A measurement's lifecycle:

1. **Ingest** — the collector writes a `readings` row and, in the *same* SQLite
   transaction, enqueues an outbox row in `publication_log` (only for
   non-quarantined measurements). Crash-consistent: you never get a reading
   without its outbox row, or vice versa.
2. **Push** — the push task batches undelivered outbox rows, POSTs them to the
   archive consumer over HTTPS with a per-target bearer token, and waits for an
   ack. The DB lock is **not** held across the network round-trip.
3. **Ack → cursor** — a valid ack (matching publication id, exact batch end)
   advances the per-target cursor. The cursor is the consumer's durable
   watermark: "I have taken custody up to here."
4. **Purge** — retention deletes readings that are (a) old enough (past a
   retention floor) **and** (b) already acknowledged. Un-acknowledged originals
   are *protected* even when old — losing them would break custody. Quarantined,
   never-enqueued, and old-epoch rows are floor-purged normally.

If the consumer is down, the cursor stops advancing, the backlog grows, and disk
fills — at which point *new writes fail loudly* (`ENOSPC`). The gateway never
silently drops stored data to make room. (Graceful active back-pressure is future
work; today the contract is "safe, not graceful" under sustained pressure.)

## Control plane

Since plan 5 the gateway also runs an **HTTPS API server** (axum + rustls,
self-signed certificate whose SHA-256 fingerprint is surfaced for pinning;
private-address clients only). State changes ride the **R14 typed operation
catalog** (`core/ops`): each operation is a descriptor with a permission tier
(`Routine` / `Daily` / `Construction`), dry-run support, and unconditional
audit into `ledger_events`. Today the API's mutation routes go through catalog
dispatch. The plan-5 CLI commands reuse the same `core/ops` functions —
`token issue|revoke` call exactly what the catalog descriptors call, and
`passphrase reset` is a deliberate out-of-band recovery path — each audited
as `local_cli`. Migrating the older CLI mutation paths (targets, snapshots,
replace-hardware) onto R14 is queued across plans 7–8. Two API endpoints
change state outside R14 by design:
initial passphrase setup and session login. Authentication is an admin
passphrase (argon2id) plus operator tokens (issued once in plaintext, stored as
SHA-256; AI tokens are structurally capped at `Routine`). A fresh database
starts in **setup mode** until the passphrase is set. The prescriptive rule:
**a new mutation surface is an R14 descriptor — never a fresh SQL mutation
path.**

## Crate map

Twenty crates, four layers. The table is topologically ordered — a crate only
ever depends on crates in *earlier* rows. `scripts/check-layers` enforces the
layer rules below mechanically (in `verify.sh` and CI).

| Crate | Path | Responsibility (one line) |
|---|---|---|
| `iotkit-core-types` | `core/types` | Domain entity types (no protocol specifics). Also still hosts the *frozen* `AdapterEvent`/`AdapterCommand` vocabulary (supervision / legacy southbound) — a known mix; the split is queued (D12 decision 8). Leaf. |
| `iotkit-ingest-contract` | `iotkit-ingest-contract` | Ingest wire contract v1: `Envelope`/`Ack`/reason codes. The wire is normative; runtime deps = serde only (serde_json appears only in dev-dependencies). Leaf. |
| `iotkit-core-storage` | `core/storage` | SQLite handle (`DbHandle`) + cross-crate migration harness. Leaf. |
| `iotkit-core-engine` | `core/engine` | In-memory device-state projection consuming the frozen `AdapterEvent` vocabulary (defined in `core/types`). Depends on `types` only; adapters must never depend on it. |
| `iotkit-core-ledger` | `core/ledger` | Device ledger: `system_id` issuance, series identity, sightings, epochs, audit events. |
| `iotkit-core-timeseries` | `core/timeseries` | `readings` + staged readings persistence, event-time derivation, queries. |
| `iotkit-core-publish` | `core/publish` | Exit-contract data layer: `publication_log` (outbox), `target_registry`, cursors. |
| `iotkit-core-collector` | `core/collector` | Ingest actor: dedup, series resolution, quarantine decision, same-tx outbox enqueue. Owns the `RegistryPolicy` trait. |
| `iotkit-core-registry` | `core/registry` | D6 measurement registry (standard catalog + site overrides); implements `RegistryPolicy`. |
| `iotkit-core-ops` | `core/ops` | R14 operation catalog, permission tiers, auth store (passphrase/tokens), dispatch + audit. |
| `iotkit-ingest-client` | `iotkit-ingest-client` | The ingest-contract client adapters use (D4). In-proc binding today; HTTP/MQTT are future feature flags. |
| `iotkit-polling-adapter-runtime` | `iotkit-polling-adapter-runtime` | Shared scaffolding for I2C-bus polling sensor adapters. |
| `rpi4b-transport` | `rpi4b-driver/transport` | Raw bus access (serial/I2C/GPIO/SPI/PWM/USB). Bytes and pin states, zero protocol knowledge. |
| `bravepi-sensors` | `bravepi-mainboard-adapter/sensors` | Per-sensor-IC conversion drivers, input-source-agnostic (shared by multiple adapters). |
| `bravepi-codec` | `bravepi-mainboard-adapter/codec` | BravePI frame encoding/decoding. |
| `bravepi-mainboard-adapter` | `bravepi-mainboard-adapter` | BravePI-protocol adapter: transport + codec + sensors → Envelopes. |
| `rpi-local-adapter` | `rpi-local-adapter` | On-Pi I2C sensor adapter; thin wrapper over the polling runtime. |
| `bravepi-poc` | `bravepi-mainboard-adapter/poc` | Hardware proof-of-concept harness for BravePI (dev tool, not shipped). |
| `iotkit-gateway` | `iotkit-gateway` | **Binary.** Composition root: adapter supervision, push task, retention, health, HTTPS API. |
| `iotkit-gatewayctl` | `iotkit-gatewayctl` | **Binary.** Operator CLI: ledger, registry, snapshots, targets, tokens (audited; plan-5 commands reuse the `core/ops` functions; older mutation paths migrate to R14 in plans 7–8). |

### Layer rules (machine-checked)

1. **Adapters never depend on `core/engine`** — projection machinery is the
   gateway's business (D4). Note: the frozen `AdapterEvent`/`AdapterCommand`
   vocabulary lives in `core/types`, so this gate does **not** police the
   freeze — reviews do.
2. **Adapters reach the data plane only through `iotkit-ingest-client`** —
   never directly on storage/ledger/timeseries/publish/collector/registry/ops.
3. **`iotkit-ingest-contract`'s runtime deps are serde and nothing else** —
   third-party conformance tests must be able to depend on it alone.
4. **`core/types` and `core/storage` are leaves**, `core/engine` depends only
   on `core/types`, and nothing in `core/*` depends on adapters or binaries
   (no upward edges).
5. **A new workspace crate must be classified deliberately** in
   `scripts/check-layers` (and placed on this map) — an unclassified crate
   fails CI.
6. **`iotkit-ingest-client`'s workspace dependencies are exactly
   `core/collector` + the contract** — never adapters, binaries, or
   `core/engine`.

Rule numbers match the `scripts/check-layers` error messages. Only the two
**binaries** may depend on everything. Dev-dependencies are exempt (tests may
cross layers); build-dependencies are checked.

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
- **Directory names ≠ crate names in two places** (historical): the
  `rpi4b-driver/` directory holds the `rpi4b-transport` crate, and
  `bravepi-mainboard-adapter/` hosts the shared `bravepi-sensors` /
  `bravepi-codec` subcrates. Renames/moves are queued Wave-1 structural
  homework (D12 decision 8) — don't half-fix them ad hoc.

## Placement rules — "where does new code go?"

| You are adding… | It goes in… |
|---|---|
| A new sensor-IC conversion (usable by several adapters) | `bravepi-sensors` (the shared driver crate, despite the vendor-flavored name — see exceptions above) |
| A new sensor family / device protocol | A **new top-level `*-adapter` crate**; build on `iotkit-polling-adapter-runtime` if it's bus polling. Never inside `core/*` or the gateway. |
| A change to the ingest wire (envelope fields, ack semantics, reason codes) | `iotkit-ingest-contract` **only**, with its conformance tests; consumers adapt. The wire is the contract — the Rust types follow it, not vice versa. |
| A new operator / AI / UI operation that changes state | A descriptor in `core/ops` `standard_catalog()` + R14 dispatch. Never a new SQL mutation path, never a bespoke API handler with its own writes. |
| A new table / column | A migration in the **owning** `core/*` crate's version slice (the binaries concatenate the slices; the `core/storage` harness applies them by set difference). |
| A new HTTP API route | `iotkit-gateway/src/api/` as a thin layer; the logic lives in the owning `core/*` crate. |
| A new CLI command | `iotkit-gatewayctl`, calling `core/*` (state changes go through the R14 catalog, audit actor `local_cli`). |
| Raw bus/pin access | `rpi4b-transport`. |
| A gateway module that has grown its own tables, is needed by both binaries, or holds more than one responsibility | **Graduate it to a new `core/<name>` crate.** The gateway is a composition root, not a home for domain logic. |

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
  would be over-engineering. WAL + `synchronous=NORMAL` (verified by a
  pragma-readback test).
- **The push task never holds the DB lock across HTTP.** It's three scopes:
  build the batch (lock), POST + await ack (no lock), advance the cursor (lock).
  A slow archive server cannot stall ingestion.
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
- The authoritative rationale: `../docs/redesign/` (D1–D13, R-ledger) — Japanese,
  for deep dives only.
