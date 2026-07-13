# IoTKit

An on-premises-first, data-integrity-focused IoT collection platform. Add a focused
sensor adapter to IoTKit Edge and IoTKit supplies durable collection, retry, and an
explicit transfer of storage responsibility to IoTKit Site: data is not purge-eligible
until Site has durably stored it.

> **Status: pre-1.0, not yet a public release.** The current milestone is one
> paired BravePI temperature sensor, one Rust IoTKit Edge, one standard
> MQTT broker, and one Go IoTKit Site.
> Broader Wave 1 work is frozen until this real-hardware path proves collection,
> outage recovery, and storage-responsibility transfer. APIs, the on-disk schema,
> and the wire contract may still change. See
> [Roadmap](#roadmap).

## Why

Industrial sites need to connect varied sensors without rebuilding reliability for
each one. An adapter owns only sensor communication, configuration, reading, and
measurement mapping; it does not own SQLite, MQTT, retry, retention, or
authentication. IoTKit supplies those concerns once, keeps collecting through
network outages, and recovers safely after power loss without silently losing data.

IoTKit Edge is deliberately boring about this: **one Rust binary + SQLite + systemd**,
with no on-Edge container orchestration, ML platform, or central rules engine. Edge is
a *buffer, not a warehouse* — it holds data until Site confirms durable storage. MQTT
PUBACK alone is not that confirmation. Site durably accepts records, advances each Edge
Node's `accepted-through` only after commit, and provides direct raw query today. It is
also the IoTKit-side boundary for later site-local registry, semantic mapping such as
`production`, and application export; those later capabilities are not implemented yet.

## What it does today

```
 BravePI Mainboard ──UART──▶ IoTKit Edge ──MQTT──▶ MQTT Broker ──▶ IoTKit Site
                              │                                      │
                              └─ SQLite readings + outbox             ├─ durable raw records
                                                                     ├─ Edge Node cursors
                                                                     └─ direct raw query
```

- **Durable ingest** with crash consistency (power loss is a normal event, not an error).
- **Series identity** that survives device rename and hardware swap (history isn't cut).
- **Measurement registry** (standard vocabulary + site overrides) and row/series quarantine for unknown or out-of-range data.
- **Exit contract (R10):** MQTT delivery through a standard Broker to IoTKit Site, at-least-once, with a per-target cursor; Site's durable `accepted-through` is what authorizes retention to purge. Unacknowledged originals are protected even when old. See [docs/exit-contract.md](docs/exit-contract.md).
- **Authenticated HTTP ingest (Plan 6):** a separate, default-off site-LAN TLS listener accepts JSON envelopes with per-device bearer credentials, bounded admission, positional item results, duplicate retry, and side-effect-free validation. See [docs/ingest-contract.md](docs/ingest-contract.md).
- **Operator CLI** (`iotkit-edgectl`) for the device ledger, measurement registry, snapshots/restore, and the Site target.
- Fresh or restored state requires local ownership/recovery; it does not expose a network setup route. Device tokens and operator authority are rechecked after recovery.
- The control-plane API is intended for private LAN reachability only. Use SSH port forwarding for Tailscale/CGNAT direct-access scenarios.

> **Current Plan 6 boundary:** HTTP measurement ingest is implemented as a separate authenticated
> listener and is disabled by default. An unowned, recovering, restore/reset-fenced, or TLS-invalid
> Edge keeps network listeners unbound. Plan 6.5 is still required for encrypted replacement
> backup containers and restore-fence carriage; MQTT, pairing windows, and batch provisioning are
> future/separate deliverables.

## Build & test

Requires the pinned toolchain in [`rust-toolchain.toml`](rust-toolchain.toml)
(Rust 1.95.0; `rustup` installs it automatically).

```bash
cargo build --workspace
cargo test  --workspace      # ~530 tests; 2 hardware-only tests are #[ignore]d
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

CI additionally checks the crate layer rules (`scripts/check-layers`) and runs
all of the above on every PR (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml));
`scripts/verify.sh` runs the fmt / layer-rule / test / clippy checks locally.

## Repository layout

| Path | What |
|------|------|
| `core/*` | The domain, one responsibility per crate: storage, ledger (device identity), timeseries, registry, collector (ingest), publish (outbox), ops (R14 typed operations & auth), types, engine (supervision) |
| `iotkit-ingest-contract` / `iotkit-ingest-client` | The ingest wire contract (Envelope/Ack) and the client adapters use |
| `*-adapter*` / `iotkit-sensor-drivers` / `rpi4b-transport` | Sensor adapters (BravePI mainboard, rpi-local), shared sensor-IC drivers and polling runtime, raw bus transport |
| `iotkit-edge` / `iotkit-edgectl` | IoTKit Edge daemon and its operator CLI |
| `iotkit-site` | IoTKit Site MQTT consumer, durable raw store, cursor manager, and query CLI |

The full crate map, layer rules, and "where does new code go" placement table
live in [docs/architecture.md](docs/architecture.md).

Development can be resumed from a single clone, including in Codex Cloud. See
[docs/cloud-development.md](docs/cloud-development.md) for the restart order and
context-authority rules.

## Architecture & contracts

- [docs/architecture.md](docs/architecture.md) — who this serves, crate map & placement rules, data flow, the custody loop, concurrency model.
- [docs/ingest-contract.md](docs/ingest-contract.md) — normative device-builder HTTP envelope, authentication, acknowledgement, retry, validation, limits, and pinned-TLS journey.
- [docs/exit-contract.md](docs/exit-contract.md) — what IoTKit Site receives and must do (record schema, ack, cursor, epochs).

The authoritative design corpus (decision records **D1–D13**, the **R1–R23**
responsibility ledger) lives in [docs/redesign/](docs/redesign/) and is currently
Japanese-only. It travels with every clone so local and Codex Cloud work use the
same design context. You do **not** need it to build, run, or make a routine change —
it's the "why", for deep dives.

## Roadmap

- **Wave 0 — "runs at our own site":** ingest, registry, ledger, retention, snapshot/restore, operator CLI. **Done.**
- **Current implementation gate:** one paired BravePI temperature sensor → BLE Long Range → BravePI Mainboard → UART → IoTKit Edge → standard MQTT Broker → IoTKit Site → raw SQLite → direct CLI query. This complete path, including application `accepted-through`, is verified on real hardware; failure injection and the remaining restart/outage matrix are still in progress. Purge eligibility advances only after validated `accepted-through`. **In progress.**
- **After the gate:** run a BravePI contact-input sensor as the second real sensor type, then choose adapter tooling and broader Wave 1 work from observed needs.
- **Wave 1 — "distributable to others":** onboarding, calibration, configuration authority, and other distribution hardening. Existing HTTP ingress and control-plane work remain available but are not current completion criteria.
- **Wave 2 — "public OSS":** client libraries, A/B updates, OS image.

## License

Not yet licensed for public use — a license will be added before the public
(Wave 2) release. Until then this is source-available for review, not for
redistribution.
