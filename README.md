# IoTKit Gateway

A small, data-integrity-first IoT gateway for the Raspberry Pi. It ingests
measurements from local sensor adapters, stores them durably through power loss,
and publishes them to an external archive consumer under an **explicit custody
contract**: data is not purged until the consumer has acknowledged receipt.

> **Status: pre-1.0, not yet a public release.** Wave 0 ("runs at our own first
> site") is done; Wave 1 ("distributable to others") is in progress. APIs, the
> on-disk schema, and the wire contract may still change. See
> [Roadmap](#roadmap).

## Why

Industrial sites need a box that keeps collecting and never silently loses data —
even when the network is down, the power blinks, or the SD card is aging. IoTKit
is deliberately boring about this: **one Rust binary + SQLite + systemd**, no
container orchestration, no on-gateway ML, no central rules engine. The gateway
is a *buffer, not a warehouse* — it holds data only until an archive consumer has
durably taken custody of it.

## What it does today

```
 sensor adapters ──▶ collector ──▶ SQLite (readings + outbox) ──▶ push task ──▶ archive consumer
   (BravePI,          (dedup,        (durable, crash-safe)         (HTTPS,        (acks a cursor;
    rpi-local)         normalize,                                   per-target     ack authorizes
                       quarantine,                                  token, at-      purge)
                       series id)                                   least-once)
```

- **Durable ingest** with crash consistency (power loss is a normal event, not an error).
- **Series identity** that survives device rename and hardware swap (history isn't cut).
- **Measurement registry** (standard vocabulary + site overrides) and row/series quarantine for unknown or out-of-range data.
- **Exit contract (R10):** outbound HTTPS push to one archive consumer, at-least-once, with a per-target cursor; the consumer's ack is what authorizes retention to purge. Unacknowledged originals are protected even when old. See [docs/exit-contract.md](docs/exit-contract.md).
- **Operator CLI** (`gatewayctl`) for the device ledger, measurement registry, snapshots/restore, and the archive target.
- Fresh-DB restore re-enters setup mode; the admin passphrase must be set again after restore.
- The control-plane API is intended for private LAN reachability only. Use SSH port forwarding for Tailscale/CGNAT direct-access scenarios.

## Build & test

Requires the pinned toolchain in [`rust-toolchain.toml`](rust-toolchain.toml)
(Rust 1.95.0; `rustup` installs it automatically).

```bash
cargo build --workspace
cargo test  --workspace      # ~460 tests; 2 hardware-only tests are #[ignore]d
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

CI runs all four on every PR (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml)).

## Repository layout

| Path | What |
|------|------|
| `core/storage` | SQLite handle, migration harness (cross-crate, set-difference versioning) |
| `core/ledger` | Device ledger, series identity, epochs, audit events |
| `core/timeseries` | `readings` storage, event-time derivation |
| `core/registry` | Measurement registry (catalog + site overrides), quarantine policy |
| `core/collector` | Ingest actor: dedup, series resolution, quarantine, outbox enqueue |
| `core/publish` | Exit-contract data layer: `publication_log` (outbox) + `target_registry` |
| `iotkit-gateway` | The daemon: adapters, push task, custody-aware retention, health |
| `iotkit-gatewayctl` | Operator CLI |
| `iotkit-ingest-client` | In-process ingest client used by adapters |
| `*-adapter*` | Sensor adapters (BravePI mainboard, rpi-local, polling runtime) |

## Architecture & contracts

- [docs/architecture.md](docs/architecture.md) — crate map, data flow, the custody loop, concurrency model.
- [docs/exit-contract.md](docs/exit-contract.md) — what an archive consumer receives and must do (record schema, ack, cursor, epochs).

The authoritative design corpus (decision records **D1–D7**, the **R1–R23**
responsibility ledger) lives under `../docs/redesign/` and is currently
Japanese-only. You do **not** need it to build, run, or make a routine change —
it's the "why", for deep dives.

## Roadmap

- **Wave 0 — "runs at our own site":** ingest, registry, ledger, retention, snapshot/restore, operator CLI. **Done.**
- **Wave 1 — "distributable to others":** exit contract (done, MVE), then public network ingress (HTTP/MQTT) with per-device auth, onboarding/quarantine UX, calibration, config authority. **In progress.**
- **Wave 2 — "public OSS":** client libraries, A/B updates, OS image.

## License

Not yet licensed for public use — a license will be added before the public
(Wave 2) release. Until then this is source-available for review, not for
redistribution.
