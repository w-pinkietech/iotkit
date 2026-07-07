# Architecture

IoTKit is one Rust binary (`iotkit-gateway`) plus an operator CLI
(`iotkit-gatewayctl`), backed by a single SQLite database. It runs unattended on
a Raspberry Pi under systemd. This document is the "get oriented in 10 minutes"
map; the authoritative *why* is the Japanese design corpus under
`../docs/redesign/` (decision records D1–D7, responsibility ledger R1–R23).

## Data flow

```
  ┌─────────────┐   AdapterEvent    ┌───────────────┐
  │  adapters   │ ────────────────▶ │   collector   │  R8: dedup, series resolution,
  │ (BravePI,   │  (in-process,     │   (ingest)    │      quarantine decision, and —
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
  Every subsystem (collector, push, retention, health) serializes through it via
  `spawn_blocking`. SQLite has exactly one writer anyway, so a connection pool
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
- The authoritative rationale: `../docs/redesign/` (D1–D7, R-ledger) — Japanese,
  for deep dives only.
