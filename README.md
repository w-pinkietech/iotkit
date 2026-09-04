# IoTKit

English | [Japanese](README.ja.md)

[Contributing](CONTRIBUTING.md) | [日本語の開発参加ガイド](CONTRIBUTING.ja.md)

An on-premises-first IoT observation platform that runs on the device. IoTKit
reads sensors through Input Adapters, turns the readings into Observations with
device-local pipelines (calibration, thresholds with hysteresis, debounce,
accumulated counting), and publishes them to a standard MQTT Broker under a fixed
public contract. Applications such as Pinkiet subscribe to the Broker and map the
Observations onto their own domain.

> **Current product version: 0.4.0 (pre-1.0).** IoTKit is available as an
> early source release. APIs, the on-disk schema, and wire contracts may change
> during the 0.x series. See [GitHub Releases](https://github.com/w-pinkietech/iotkit/releases)
> and the [Roadmap](#roadmap).
> The [v1 compatibility policy](docs/product/en/contracts/compatibility-policy-v1.md)
> takes effect at product 1.0.0 and does not change this pre-1.0 status.

Current product knowledge is also available as an OKF v0.2 bundle in
[Japanese](docs/product/ja/index.md) and [English](docs/product/en/index.md).

## Try IoTKit on this PC

On a Linux host with Git, Python 3.14+, and Docker Compose, the repository's
two-line [`iotkit.toml`](iotkit.toml) starts a loopback-only trial: the Edge Node
with the `trial-sample` Input Adapter and three pipelines, plus a Mosquitto Broker.

```bash
./scripts/iotkit trial validate
./scripts/iotkit trial up
./scripts/iotkit trial watch
```

`watch` prints every Observation and status the device publishes, as an
independent consumer would see them. See the
[trial profile guide](docs/product/en/operations/trial-profile.md) for what to
check and how to stop or reset. The trial is not a field deployment.

## Why

Industrial sites need to connect varied sensors without rebuilding reliability
for each one. An Input Adapter owns only sensor communication, configuration,
reading, and measurement mapping; it does not own SQLite, MQTT, retry, or
authentication. IoTKit supplies those once, keeps collecting through Broker
outages, and never confuses "sent" with "stored somewhere else": the MQTT PUBACK
is the boundary of its delivery responsibility, and its outbox is the only source
of retransmission.

IoTKit is deliberately boring about this: **one Rust binary + SQLite + systemd**
per device, and a standard Broker in between. Nothing central is required.

## What it does today

```text
sensor -> Input Adapter -> pipeline -> MQTT Output Adapter -> MQTT Broker -> consumer
          |<---------------- IoTKit Edge Node (one per device) ---------------->|
```

- **Input Adapters** for BravePI Mainboard (UART) and direct Raspberry Pi I2C
  sensors, plus the `trial-sample` adapter for evaluation. Adapters are hosted by
  a generic supervisor with restart backoff; an adapter that cannot open its
  hardware interface is reported, not fatal.
- **Pipelines** (`measurement`, `state`, `accumulated-count`) are stored in SQLite,
  edited through typed operations (`nodectl pipeline` today, the Console later),
  and evaluated inside the same transaction that persists their state and the
  outbox row. Tuning changes keep the series; structural changes and explicit
  resets start a new one.
- **MQTT Output Adapter contract v1**: `iotkit/v1/edge-node/{edge-node-id}/observation/{pipeline-id}/{kind}`
  with QoS 1 and retain, one publication in flight, deletion from the outbox only
  after PUBACK, and a `status` topic with `online` / `degraded` / `offline`, a
  `faults` list, a Will, and a graceful `offline`. Two clocks per payload:
  `uptime_ms` (monotonic) and `unix_epoch_ms` (only while the device trusts its
  wall clock). Schemas and fixtures under `testdata/observation/v1/` are the shared
  reference for producers and consumers.
- **Operator CLI** (`iotkit-edge-nodectl`) for pipeline import/export/update/reset,
  passphrase and token management, and health.

The central IoTKit Edge service, its custody contract, and application-facing
Output Adapters were removed in the redesign
([#232](https://github.com/w-pinkietech/iotkit/issues/232)); the remaining old
Edge Node paths are being deleted in
[#250](https://github.com/w-pinkietech/iotkit/issues/250).

## Field installation

Install the Edge Node binaries under systemd on the device, point `[output.mqtt]`
at the site's Broker (TLS with `system_roots` or `bundle_only`), and import the
pipeline definitions with `nodectl pipeline import`. The runbook is
[Installation and recovery](docs/product/en/operations/installation-and-recovery.md);
Broker certificate tooling is `scripts/iotkit-broker-cert`.

## Build & test

Toolchains are pinned with [mise](https://mise.jdx.dev/) (`mise.toml`). System
packages: `pkg-config`, `libudev-dev`, and `mosquitto` with `mosquitto-clients`
for the journey.

```bash
mise install
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check

# MQTT Output Adapter v1 contract fixtures (schema, canonical bytes, consumer side)
node scripts/check-observation-fixtures.mjs
scripts/test-observation-consumer.sh

# End-to-end journey: trial-sample -> Edge Node -> Mosquitto -> independent consumer
scripts/test-journey.sh
```

CI runs three lanes on every PR, lightweight repository checks, the full Rust
workspace (fmt, clippy, tests), and the journey lane, and publishes the stable
`required CI` aggregate (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml)).
The journey lane (`scripts/test-journey.sh`) is the acceptance evidence of the
redesign: first the minimal loop, then fault injection (Broker outage, `kill -9`,
tuning change, deletion, storage failure, graceful shutdown). The test policy is
in [`.agents/testing.md`](.agents/testing.md). Run `scripts/verify.sh --workspace`
locally for an explicit diagnosis.

## Repository layout

| Path | What |
|------|------|
| `edge-node/apps/` | Rust Edge Node daemon and operator CLI composition roots |
| `edge-node/core/` | Device-local domain, one responsibility per crate (`pipeline`, `collector`, `ops`, `storage`, `types`, ...) |
| `edge-node/ingest/` | Envelope/Ack contract and the in-process binding used by Input Adapters |
| `edge-node/input/` | Adapter host API, conformance testkit, polling runtime, transports, and reusable sensor drivers |
| `edge-node/adapters/` | Concrete sensor-family integrations such as BravePI Mainboard and direct Raspberry Pi I2C |
| `testdata/observation/v1/` | MQTT Output Adapter v1 schemas and fixtures shared by producer and consumer tests |
| `docs/`, `deploy/`, `scripts/`, `review/` | Product documentation, deployment assets, automation, and [review suite](review/README.md) perspectives |

The full crate map, layer rules, and "where does new code go" placement table
live in the [architecture documentation](docs/product/en/architecture/system-overview.md).
Start collection-side work at [`edge-node/README.md`](edge-node/README.md) and
concrete adapter work at [`edge-node/adapters/README.md`](edge-node/adapters/README.md).

Development can be resumed from a single clone, including in Codex Cloud. See
[docs/cloud-development.md](docs/cloud-development.md) for the restart order and
context-authority rules.

## Architecture & contracts

- [Documentation index](docs/README.md) — the reading path and source-of-truth order.
- [Product model](docs/product/en/concepts/product-model.md) — what IoTKit owns, the Observation model, configuration ownership, and pipeline definitions.
- [Architecture](docs/product/en/architecture/system-overview.md) — crate map, placement rules, data flow, and concurrency.
- [Contracts](docs/product/en/index.md#contracts) — MQTT Output Adapter v1, Input Adapter v1, and the v1 compatibility policy.

Historical redesign decisions and completed implementation plans remain in the
repository for rationale and traceability, but they do not override current
executable contracts or the documentation index.

## Roadmap

- **Redesign to a device-local platform ([#232](https://github.com/w-pinkietech/iotkit/issues/232)):** contract, TOML configuration, pipeline core, and the MQTT Output Adapter with the end-to-end journey as the CI gate. **Done** (child issues 1–4).
- **Removal of the central layer ([#250](https://github.com/w-pinkietech/iotkit/issues/250)):** old components, contracts, and documents; a fresh migration baseline. **In progress.**
- **Console on the Edge Node (#232 child issue 6):** pipeline editing, reset, fault display, and a short-term input buffer.
- **Distribution:** installation image, updates, and client libraries once the device-local product is stable.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
