# IoTKit

An on-premises-first, data-integrity-focused IoT collection platform. Add a focused
sensor adapter to IoTKit Edge Node and IoTKit supplies durable collection, retry, and an
explicit transfer of storage responsibility to IoTKit Edge: data is not purge-eligible
until IoTKit Edge has durably stored it.

> **Status: v1 release candidate.** The complete path—BravePI temperature/contact,
> one or more Rust IoTKit Edge Nodes, a standard MQTT broker, authenticated IoTKit Console,
> future-only semantic mapping, and durable YokaKit MQTT output—is implemented. APIs, the
> on-disk schema, and the wire contract may
> still change. See
> [Roadmap](#roadmap).

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
such as YokaKit own products, processes, OEE, alarms, business UI, and notifications.

## What it does today

```
 BravePI Mainboard ──UART──▶ IoTKit Edge Node ──MQTT──▶ MQTT Broker ──▶ IoTKit Edge
                              │                                      │
                              └─ SQLite readings + outbox             ├─ durable raw records
                                                                     ├─ Edge Node cursors
                                                                     ├─ direct raw/semantic query
                                                                     └─ semantic MQTT outbox
```

- **Durable ingest** with crash consistency (power loss is a normal event, not an error).
- **Series identity** that survives device rename and hardware swap (history isn't cut).
- **Measurement registry** (standard vocabulary + deployment overrides) and row/series quarantine for unknown or out-of-range data.
- **Exit contract (R10):** MQTT delivery through a standard Broker to IoTKit Edge, at-least-once, with a per-target cursor; IoTKit Edge's durable `accepted-through` is what authorizes retention to purge. Unacknowledged originals are protected even when old. See [docs/exit-contract.md](docs/exit-contract.md).
- **Authenticated HTTP ingest (Plan 6):** a separate, default-off local-network TLS listener accepts JSON envelopes with per-device bearer credentials, bounded admission, positional item results, duplicate retry, and side-effect-free validation. See [docs/ingest-contract.md](docs/ingest-contract.md).
- **Operator CLI** (`iotkit-edge-nodectl`) for the device ledger, measurement registry, snapshots/restore, and the IoTKit Edge target.
- **IoTKit Edge operations** for bounded history/CSV, storage diagnostics, and encrypted backup/new-path restore.
- Fresh or restored state requires local ownership/recovery; it does not expose a network setup route. Device tokens and operator authority are rechecked after recovery.
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
  --edge-publish-topic 'yokakit/v1/sources/iotkit-01/signals/press-count/observations' \
  --edge-publish-topic 'yokakit/v1/sources/iotkit-01/status'
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml up --build --detach
```

上記は小規模・standalone向けの`embedded` profile（SQLite）である。同じConsoleとMQTT契約を
維持したまま、常設運用向けの管理対象PostgreSQL profileを選ぶ場合はbootstrapへ
`--storage-profile postgres`を追加し、起動時にもoverlayを追加する。

```bash
docker compose --env-file "$install_root/edge.env" \
  -f deploy/compose.edge.yaml -f deploy/compose.edge-postgres.yaml \
  up --build --detach
```

profileは導入directoryの`storage-profile.json`へ固定され、起動flagと異なる場合は停止する。
接続失敗時にSQLiteへfallbackせず、両DBへの二重書込みもしない。SQLiteからの切替手順は
[IoTKit Edge installation and recovery](docs/edge-operations.md)を参照する。

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
  --display-name 'システム管理者' --password-file /run/iotkit/admin-password
rm "$install_root/secrets/initial-admin-password"
```

Open `IOTKIT_EDGE_ORIGIN` from a Windows browser. Caddy is the only LAN-facing
HTTPS endpoint; IoTKit Edge's HTTP listener remains on `127.0.0.1`. All screens require
login. `viewer` can inspect, `admin` can configure devices/signals/meaning/output,
and `system_admin` can additionally issue accounts.

Run the commissioning smoke commands above after startup. A later bootstrap
invocation refuses to replace its output directory. See
[IoTKit Edge installation and recovery](docs/edge-operations.md) for diagnosis,
password recovery, certificate renewal, and rollback behavior.

### IoTKit Edge semantics and application output

Use the Console's **信号** screen to set correction, threshold/hysteresis, boolean
state, cumulative-value counting, or alarm behavior. A five-minute live preview
uses only observations received after preview start and never writes a mapping or
output event. Saving starts a new future-only revision; old raw records are not
silently recalculated.

Use **出力** to bind a generic semantic definition to a YokaKit `source-id`,
`signal-id`, and purpose (`production`, `onoff`, `gantt_chart`, or `alarm`).
IoTKit's cumulative value becomes YokaKit `kind=production` only inside this
adapter. Output is QoS 1 and remains in the IoTKit Edge outbox until broker PUBACK.
YokaKit status is published separately as a retained source status.

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

# Docker Mosquittoによる外部Output Adapter/PUBACK/再接続ゲート
scripts/test-edge-output.sh

# SQLite/PostgreSQL共通契約と短時間capacity回帰smoke
scripts/test-edge-postgres.sh
scripts/test-edge-capacity.sh

# OpenAPIから生成したConsole型、TypeScript、埋め込みJavaScriptの同期
npm ci --prefix iotkit-edge/frontend
scripts/test-edge-console-frontend.sh

# Chromiumによるlogin、Edge Node登録、センサー設定、意味付け、外部出力、権限導線
scripts/test-edge-console-e2e.sh

# 隣接するYokaKit checkoutとのconsumer contractゲート
scripts/test-yokakit-consumer-contract.sh
```

IoTKit ConsoleはGoのserver-side renderingを維持し、ブラウザ動作だけを
`iotkit-edge/frontend/src/`のTypeScriptで実装する。JSON APIの型は
`iotkit-edge/openapi/edge-console-v1.yaml`から生成し、配布物にはesbuild済みの
`static/console.js`を埋め込むため、IoTKit Edgeの実行環境にNode.jsは不要である。

CI additionally checks the crate layer rules (`scripts/check-layers`) and runs
all of the above on every PR (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml));
`scripts/verify.sh` runs the fmt / layer-rule / test / clippy checks locally.

## Repository layout

| Path | What |
|------|------|
| `core/*` | The domain, one responsibility per crate: storage, ledger (device identity), timeseries, registry, collector (ingest), publish (outbox), ops (R14 typed operations & auth), types, engine (supervision) |
| `iotkit-ingest-contract` / `iotkit-ingest-client` | The ingest wire contract (Envelope/Ack) and the client adapters use |
| `*-adapter*` / `iotkit-sensor-drivers` / `rpi4b-transport` | Sensor adapters (BravePI mainboard, rpi-local), shared sensor-IC drivers and polling runtime, raw bus transport |
| `iotkit-edge-node` / `iotkit-edge-nodectl` | IoTKit Edge Node daemon and its operator CLI |
| `iotkit-edge` | IoTKit Edge MQTT consumer, durable raw/semantic store, cursor manager, application exporter, authenticated SSR Console, and query/configuration CLI |

The full crate map, layer rules, and "where does new code go" placement table
live in [docs/architecture.md](docs/architecture.md).

Development can be resumed from a single clone, including in Codex Cloud. See
[docs/cloud-development.md](docs/cloud-development.md) for the restart order and
context-authority rules.

## Architecture & contracts

- [docs/architecture.md](docs/architecture.md) — who this serves, crate map & placement rules, data flow, the custody loop, concurrency model.
- [docs/ingest-contract.md](docs/ingest-contract.md) — normative device-builder HTTP envelope, authentication, acknowledgement, retry, validation, limits, and pinned-TLS journey.
- [docs/exit-contract.md](docs/exit-contract.md) — what IoTKit Edge receives and must do (record schema, ack, cursor, epochs).

The authoritative design corpus (decision records **D1–D13**, the **R1–R23**
responsibility ledger) lives in [docs/redesign/](docs/redesign/) and is currently
Japanese-only. It travels with every clone so local and Codex Cloud work use the
same design context. You do **not** need it to build, run, or make a routine change —
it's the "why", for deep dives.

## Roadmap

- **Wave 0 — "runs in our own environment":** ingest, registry, ledger, retention, snapshot/restore, operator CLI. **Done.**
- **First implementation gate:** one paired BravePI temperature sensor → BLE Long Range → BravePI Mainboard → UART → IoTKit Edge Node → standard MQTT Broker → IoTKit Edge → raw SQLite → direct CLI query. The real-hardware path, restart/outage matrix, storage failure injection, bounded-capacity behavior, and application `accepted-through` are verified. Purge eligibility advances only after validated `accepted-through`. **Done.**
- **IoTKit Edge semantic and output slice:** generic numeric/boolean/cumulative/alarm meaning, live preview, no backfill, durable Output Adapter boundary, and the accepted YokaKit source/signal observation contract. **Implemented.**
- **Wave 1 — "distributable to others":** onboarding, calibration, configuration authority, and other distribution hardening. Existing HTTP ingress and control-plane work remain available but are not current completion criteria.
- **Wave 2 — "public OSS":** client libraries, A/B updates, OS image.

## License

Not yet licensed for public use — a license will be added before the public
(Wave 2) release. Until then this is source-available for review, not for
redistribution.
