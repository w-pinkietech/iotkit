# IoTKit

English | [Japanese](README.ja.md)

[Contributing](CONTRIBUTING.md) | [日本語の開発参加ガイド](CONTRIBUTING.ja.md)

An on-premises-first, data-integrity-focused IoT collection platform. Add a focused
sensor adapter to IoTKit Edge Node and IoTKit supplies durable collection, retry, and an
explicit transfer of storage responsibility to IoTKit Edge: data is not purge-eligible
until IoTKit Edge has durably stored it.

> **Current product version: 0.3.0 (pre-1.0).** IoTKit is available as an
> early source release. APIs, the on-disk schema, and wire contracts may change
> during the 0.x series. See [GitHub Releases](https://github.com/w-pinkietech/iotkit/releases)
> and the [Roadmap](#roadmap).

Current product knowledge is also available as an OKF v0.2 bundle in
[Japanese](docs/product/ja/index.md) and [English](docs/product/en/index.md).

## Try IoTKit on this PC

On a Linux host with Git, Python 3.11+, and Docker Compose, the repository's
two-line [`iotkit.toml`](iotkit.toml) starts a loopback-only trial:

```bash
./scripts/iotkit trial validate
./scripts/iotkit trial up
```

Choose the trial administrator password when prompted, then open
`http://127.0.0.1:8080` and sign in as `admin`. Changing illuminance (triangle wave)
and contact-state (square wave) samples travel through an Input Adapter, Edge Node
custody, a standard MQTT Broker, and IoTKit Edge; they are not seeded into the
database or Console. See the
[trial profile guide](docs/product/en/operations/trial-profile.md) for review and
cleanup. The trial is not a field deployment.

## Why

Industrial sites need to connect varied sensors without rebuilding reliability for
each one. An adapter owns only sensor communication, configuration, reading, and
measurement mapping; it does not own SQLite, MQTT, retry, retention, or
authentication. IoTKit supplies those concerns once, keeps collecting through
network outages, and recovers safely after power loss without silently losing data.

IoTKit Edge Node is deliberately boring about this: **one Rust binary + SQLite + systemd**,
with no on-node container orchestration, ML platform, or central rules engine. Edge Node is
a *buffer, not a warehouse* — it holds data until IoTKit Edge confirms durable storage. MQTT
PUBACK alone is not that confirmation. IoTKit Edge durably accepts records, advances each Edge Node's
`accepted-through` only after commit, and provides direct raw query today. It is
also the IoTKit-side boundary for archive query, configurable sensor meaning, and application
export. IoTKit Edge maps a stored signal to generic numeric, boolean, cumulative-value, or
alarm semantics, then a separate Output Adapter converts observations to an
application-facing MQTT contract. Applications
such as Pinikiet own products, processes, OEE, alarms, business UI, and notifications.

## What it does today

```
 vendor/protocol device ──▶ Input Adapter ──┐
 contract-native device ──▶ HTTPS ingest ───┴─▶ IoTKit Edge Node
                                                   │ SQLite readings + outbox
                                                   ▼
                                            internal MQTT Broker
                                                   │
                                                   ▼
                                              IoTKit Edge
                                                   ├─ durable raw records and Edge Node cursors
                                                   ├─ generic semantics
                                                   └─ Output Adapter ──▶ external Broker ──▶ application
```

- **Durable ingest** with crash consistency (power loss is a normal event, not an error).
- **Series identity** that survives device rename and hardware swap (history isn't cut).
- **Measurement registry** (standard vocabulary + deployment overrides) and row/series quarantine for unknown or out-of-range data.
- **Edge Node custody contract:** MQTT delivery through a standard Broker to IoTKit Edge, at-least-once, with a per-target cursor; IoTKit Edge's durable `accepted-through` is what authorizes retention to purge. Unacknowledged originals are protected even when old. See the [Edge Node custody contract](docs/product/en/contracts/edge-node-custody-v1.md).
- **Authenticated HTTP ingest:** a separate, default-off local-network TLS listener accepts JSON envelopes with per-device bearer credentials, bounded admission, positional item results, duplicate retry, and side-effect-free validation. See the [authenticated ingest contract](docs/product/en/contracts/ingest-v1.md).
- **Operator CLI** (`iotkit-edge-nodectl`) for the device ledger, measurement registry, snapshots/restore, and the IoTKit Edge target.
- **IoTKit Edge operations** for bounded history/CSV, storage diagnostics, and encrypted backup/new-path restore.
- Fresh or restored state requires local ownership/recovery; it does not expose a network setup route. Device tokens and operator authority are rechecked after recovery.

For a failed Edge Node host or hardware replacement, start with the
[Edge Node hardware recovery quick guide](docs/product/en/operations/edge-node-hardware-recovery.md).
- The control-plane API is intended for private LAN reachability only. Use SSH port forwarding when the deployment's private routed path does not provide direct client reachability.

### Edge Node initialization

Create a fresh Edge Node database and print its generated identity. The command refuses an existing
database instead of modifying it:

```bash
iotkit-edge-nodectl --db edge-node.db init
iotkit-edge-nodectl --db edge-node.db identity
iotkit-edge-nodectl --db edge-node.db mqtt-binding
```

The latter two commands are read-only. `mqtt-binding` reports the username, client ID, topics,
QoS, and retain flag used by Edge Node, but never creates or displays a credential.

After Broker, IoTKit Edge, and Edge Node are running, enqueue a synthetic commissioning record and check the
durable IoTKit Edge acknowledgement without reading either SQLite schema:

```bash
smoke=$(iotkit-edge-nodectl --db edge-node.db smoke enqueue)
iotkit-edge-nodectl --db edge-node.db smoke status \
  --ledger-epoch "$(jq -r .ledger_epoch <<<"$smoke")" \
  --pub-seq "$(jq -r .pub_seq <<<"$smoke")"
```

`delivered` means the normal MQTT record reached IoTKit Edge durable raw storage and its correlated
`accepted-through` advanced the Edge Node cursor. The smoke record is not a sensor measurement and is
excluded from semantic projection.

### IoTKit Edge deployment bootstrap

The production-shaped deployment runs Edge Node natively on its Raspberry Pi and runs the standard
Broker plus IoTKit Edge with Docker Compose on a Linux IoTKit Edge host. Prepare an existing full-chain server
certificate, private key, and root trust bundle; certificate issuance, DNS, firewall rules, and any
optional VPN remain the IoTKit Edge operator's responsibility. The Broker hostname must resolve on the
IoTKit Edge host to the explicit bind address, and the certificate must cover that hostname.

First export the non-secret binding from the initialized Edge Node and transfer that JSON to the IoTKit Edge
operator:

```bash
iotkit-edge-nodectl --db /var/lib/iotkit/edge-node.db mqtt-binding > edge-node-mqtt-binding.json
```

From a repository clone on the IoTKit Edge host, run the bootstrap as the non-root account that will run
Compose. The output directory must be new, outside the Git repository, and below an existing
operator-owned parent directory:

```bash
install_root="$HOME/.local/share/iotkit/edge-01"
mkdir -p "$(dirname "$install_root")"
scripts/bootstrap-edge.sh \
  --binding ./edge-node-mqtt-binding.json \
  --output-dir "$install_root" \
  --broker-host mqtt.edge.example \
  --broker-bind 192.0.2.10 \
  --tls-cert /secure/path/server-fullchain.pem \
  --tls-key /secure/path/server.key \
  --tls-ca /secure/path/broker-ca.pem \
  --edge-publish-topic 'pinikiet/v1/sources/iotkit-01/sensors/press-sensor/observations' \
  --edge-publish-topic 'pinikiet/v1/sources/iotkit-01/status'
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml up --build --detach
```

The example above uses the `embedded` profile (SQLite) for smaller, standalone
deployments. To select the managed PostgreSQL profile for a larger permanent
deployment while keeping the same Console and MQTT contracts, add
`--storage-profile postgres` during bootstrap and include the overlay at startup.

```bash
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml -f deploy/compose.edge-postgres.yaml \
  up --build --detach
```

The profile is pinned in the installation directory's `storage-profile.json`.
IoTKit Edge stops if startup flags disagree with it. It neither falls back to
SQLite on connection failure nor dual-writes both databases. See
[IoTKit Edge installation and recovery](docs/product/en/operations/installation-and-recovery.md) for the offline
SQLite-to-PostgreSQL migration procedure.

The generator creates an anonymous-disabled Broker configuration, an Edge Node-specific ACL and hashed
password database, the IoTKit Edge secret, and `edge-handoff/`. Securely transfer the three handoff files
to the Edge Node. Install `mqtt-password` and `broker-ca.pem` at the paths named by
`edge-mqtt.toml`, owned by the Edge Node service account with mode `0600`, and merge the TOML fragment
into the Edge Node configuration before restarting Edge Node. Remove the IoTKit Edge host's `edge-handoff/` copy
after successful transfer. Credentials never belong in argv, environment variables, logs, or Git.

Create the first IoTKit Edge owner from the IoTKit Edge host using an owner-only temporary file:

```bash
install -m 600 /dev/null "$install_root/secrets/initial-admin-password"
# Write the initial password without putting it in shell history.
docker compose --env-file "$install_root/edge.env" -f deploy/compose.edge.yaml run --rm \
  -v "$install_root/secrets/initial-admin-password:/run/iotkit/admin-password:ro" \
  edge account bootstrap --storage-profile "$(sed -n 's/^IOTKIT_STORAGE_PROFILE=//p' "$install_root/edge.env")" \
  --db /data/edge.db \
  --postgres-config "$(sed -n 's/^IOTKIT_POSTGRES_CONFIG=//p' "$install_root/edge.env")" \
  --storage-metadata /run/iotkit/storage-profile.json --login-id admin \
  --display-name 'System Administrator' --password-file /run/iotkit/admin-password
rm "$install_root/secrets/initial-admin-password"
```

Open `IOTKIT_EDGE_ORIGIN` from a Windows browser. Caddy is the only LAN-facing
HTTPS endpoint; IoTKit Edge's HTTP listener remains on `127.0.0.1`. All screens require
login. `viewer` can inspect, `admin` can configure devices/signals/meaning/output,
and `system_admin` can additionally issue accounts.

Run the commissioning smoke commands above after startup. A later bootstrap
invocation refuses to replace its output directory. See
[IoTKit Edge installation and recovery](docs/product/en/operations/installation-and-recovery.md) for diagnosis,
password recovery, certificate renewal, and rollback behavior.

### IoTKit Edge semantics and application output

Use the Console's **Signals** screen to set correction, threshold/hysteresis, boolean
state, cumulative-value counting, or alarm behavior. A five-minute live preview
uses only observations received after preview start and never writes a mapping or
output event. Saving starts a new future-only revision; old raw records are not
silently recalculated.

Use **Output** to publish through an Edge-owned `source-id`, a sensor-level
`sensor-id`, and a Pinikiet purpose (`production`, `onoff`, `gantt_chart`, or
`alarm`). All purposes derived from one sensor share one registered topic while
their `series-id` and `sequence` values remain independent.
IoTKit's cumulative value becomes Pinikiet `kind=production` only inside this
adapter. Output is QoS 1 and remains in the IoTKit Edge outbox until broker PUBACK.
Pinikiet status is published separately as a retained source status.

The internal Edge Node/IoTKit Edge broker and external application broker may be different.
Install their endpoint, trust bundle, client ID, and credential as deployment
configuration. Pass the external profile with the `--output-*` `serve` flags;
the Console intentionally displays status but cannot change broker credentials.

## Build & test

Requires the pinned toolchain in [`rust-toolchain.toml`](rust-toolchain.toml)
(Rust 1.95.0; `rustup` installs it automatically).

```bash
cargo build --workspace
cargo test  --workspace      # ~530 tests; 2 hardware-only tests are #[ignore]d
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check

# External Output Adapter, PUBACK, and reconnect gate with Docker Mosquitto
scripts/test-edge-output.sh
IOTKIT_TEST_STORAGE_PROFILE=postgres scripts/test-edge-output.sh

# Shared SQLite/PostgreSQL contract and short capacity regression smoke
scripts/test-edge-postgres.sh
scripts/test-edge-capacity.sh

# Generated Console types, TypeScript, and embedded JavaScript synchronization
npm ci --prefix edge/frontend
scripts/test-edge-console-frontend.sh

# Chromium journey: login, Edge Node registration, sensors, semantics, output, and roles
scripts/test-edge-console-e2e.sh
IOTKIT_TEST_STORAGE_PROFILE=postgres scripts/test-edge-console-e2e.sh

# v1 host integration gate (provide a new report directory)
scripts/test-edge-host-release-gate.sh /secure/report/iotkit-v1-YYYYMMDD
```

The IoTKit Console uses typed server-side rendering in Rust and implements
browser behavior in TypeScript under `edge/frontend/src/`. JSON API types are
generated from `edge/openapi/edge-console-v1.yaml`. The distribution embeds
the esbuild output as `static/console.js`, so the IoTKit Edge runtime does not
require Node.js.

CI checks the crate layer rules, Rust unit tests, generated Console assets, and the embedded
browser journey on every PR (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml)). Run
`test-edge-host-release-gate.sh` once before release for integration coverage including Docker,
PostgreSQL, and Broker failures.
`scripts/verify.sh` runs the fmt / layer-rule / test / clippy checks locally.

## Repository layout

| Path | What |
|------|------|
| `edge-node/apps/` | Rust Edge Node daemon and operator CLI composition roots |
| `edge-node/core/` | Durable collection domain, one responsibility per crate |
| `edge-node/ingest/` | Envelope/Ack contract plus in-process and authenticated HTTP bindings |
| `edge-node/input/` | Adapter host API, conformance testkit, polling runtime, transports, and reusable sensor drivers |
| `edge-node/adapters/` | Concrete sensor-family integrations such as BravePI Mainboard and direct Raspberry Pi I2C |
| `edge/` | Rust IoTKit Edge service, Console, raw/semantic storage, cursor management, and application output |
| `docs/`, `deploy/`, `scripts/`, `testdata/`, `review/` | Shared contracts, deployment, automation, cross-component fixtures, and review policy |

The full crate map, layer rules, and "where does new code go" placement table
live in the [architecture documentation](docs/product/en/architecture/system-overview.md).
Start collection-side work at [`edge-node/README.md`](edge-node/README.md), concrete
adapter work at [`edge-node/adapters/README.md`](edge-node/adapters/README.md), and
Edge/Console work at [`edge/README.md`](edge/README.md).

Development can be resumed from a single clone, including in Codex Cloud. See
[docs/cloud-development.md](docs/cloud-development.md) for the restart order and
context-authority rules.

## Architecture & contracts

- [Documentation index](docs/README.md) — the reading path and source-of-truth order.
- [Product model](docs/product/en/concepts/product-model.md) — what IoTKit owns, its component boundaries, and what stays in external applications.
- [Architecture](docs/product/en/architecture/system-overview.md) — crate map, placement rules, data flow, custody, and concurrency.
- [Contracts](docs/product/en/index.md#contracts) — device ingest, Input Adapter, Edge transfer, and Output Adapter boundaries.

Historical redesign decisions and completed implementation plans remain in the
repository for rationale and traceability, but they do not override current
executable contracts or the documentation index.

## Roadmap

- **Wave 0 — "runs in our own environment":** ingest, registry, ledger, retention, snapshot/restore, operator CLI. **Done.**
- **First implementation gate:** one paired BravePI temperature sensor → BLE Long Range → BravePI Mainboard → UART → IoTKit Edge Node → standard MQTT Broker → IoTKit Edge → raw SQLite → direct CLI query. The real-hardware path, restart/outage matrix, storage failure injection, bounded-capacity behavior, and application `accepted-through` are verified. Purge eligibility advances only after validated `accepted-through`. **Done.**
- **IoTKit Edge semantic and output slice:** generic numeric/boolean/cumulative/alarm meaning, live preview, no backfill, durable Output Adapter boundary, and the accepted Pinikiet source/signal observation contract. **Implemented.**
- **Wave 1 — "distributable to others":** onboarding, calibration, configuration authority, and other distribution hardening. Existing HTTP ingress and control-plane work remain available but are not current completion criteria.
- **Wave 2 — "public OSS":** client libraries, A/B updates, OS image.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
