# Rust IoTKit Edge Replacement Design

Status: approved for implementation on 2026-07-24  
Issue: #83

## 1. Purpose

IoTKit lets deployment teams add Input Adapters and Output Adapters for the
hardware and applications used at each installation. Input Adapters and the
Edge Node are currently Rust, while Output Adapters and IoTKit Edge are Go.
That split forces adapter authors and maintainers to learn two languages,
dependency ecosystems, test styles, and release paths.

This change replaces the Go IoTKit Edge with Rust so the whole repository uses
one Cargo workspace and one extension-development toolchain. The rewrite is
not a product redesign. Existing user-visible contracts, operator journeys,
security boundaries, and custody guarantees remain authoritative.

## 2. Scope

The replacement includes:

- process configuration and lifecycle;
- SQLite and PostgreSQL storage profiles;
- Edge Node discovery, activation, MQTT record intake, durable raw storage,
  cursor handling, and `accepted-through`;
- semantic calibration, rules, preview, current state, cumulative values, and
  alarm state;
- generic MQTT JSON and Pinikiet Output Adapters;
- durable external MQTT outbox and PUBACK convergence;
- local accounts, password hashing, sessions, CSRF, roles, and audit;
- HTTP API, SSR Console, TypeScript assets, CSV export, and history;
- encrypted backup, restore, storage migration, diagnostics, capacity status,
  and local CLI;
- Docker, Compose, Caddy-facing HTTP, CI, release scripts, and documentation.

The replacement does not include:

- reading a Go-era SQLite or PostgreSQL database;
- restoring a Go-era `.iotkit-backup` artifact;
- Wasm, dynamic plugins, shared-library plugins, FFI, or a separate adapter
  host process;
- a Console redesign, SPA conversion, URL redesign, or API cleanup;
- new MQTT topics or payload schemas;
- target-hardware capacity measurement;
- moving the existing Input Adapter directories;
- webhook output or speculative third-party plugin APIs.

Rust IoTKit Edge accepts only a fresh Rust schema. A Go schema is rejected with
an actionable error rather than interpreted or mutated. Rust-created backup
artifacts remain round-trip compatible across Rust releases governed by the
normal pre-release migration policy.

## 3. Product and wire compatibility

The following remain stable:

- executable name `iotkit-edge`;
- Console URLs, form actions, redirects, and main DOM hooks;
- OpenAPI routes, JSON fields, HTTP statuses, headers, and error codes;
- cookie names, cookie attributes, session invalidation, CSRF behavior, and
  role permissions;
- Edge Node MQTT topics, descriptor and activation messages, record batches,
  QoS, retain flags, and `accepted-through`;
- Output Adapter IDs, configuration schema versions, Observation identity,
  MQTT topic and payload bytes, QoS, and retain flags;
- CLI command names, flags, JSON output fields, standard error behavior, and
  exit statuses;
- backup command journeys, encryption algorithm, tamper detection, private
  file modes, and restore safety for Rust-created artifacts;
- Caddy being the LAN-facing HTTPS endpoint while Edge serves configured
  loopback development HTTP.

Internal Go package types, SQL text, row layout, migration history, and
implementation-specific error strings are not compatibility surfaces.

## 4. Repository architecture

IoTKit Edge stays in `edge/`. The production server is one Rust package with a
library target and a thin binary target. Internal responsibilities are modules,
not a microcrate graph.

```text
edge/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── config/
│   ├── application/
│   ├── storage/
│   │   ├── sqlite/
│   │   └── postgres/
│   ├── mqtt/
│   │   ├── ingest/
│   │   └── output/
│   ├── semantics/
│   ├── auth/
│   ├── web/
│   ├── backup/
│   ├── diagnostics/
│   ├── cli/
│   └── composition/
│       └── output_adapters.rs
├── migrations/
│   ├── sqlite/
│   └── postgres/
├── output-adapters/
│   ├── README.md
│   ├── README.ja.md
│   ├── api/
│   ├── testkit/
│   ├── example/
│   ├── generic-mqtt-json-v1/
│   └── pinikiet-mqtt-v1/
├── frontend/
├── openapi/
└── tests/
```

The workspace initially contains both Go and Rust Edge implementations. Rust
uses a temporary `edge/src` and Cargo target while Go remains in `edge/cmd` and
`edge/internal`. The final cutover commit deletes Go and its toolchain files.
There is no production dual-write, shared database, FFI, or mixed process.

### 4.1 Dependency direction

`main.rs` composes concrete storage, MQTT, HTTP, and task supervision.
Application operations depend on domain types and narrow internal ports, never
on Axum extractors, SQLx rows, rumqttc packets, or template view models.

Storage, MQTT, and web modules depend inward on application/domain types.
Output Adapter API is a leaf and must not depend on Tokio, SQLx, Axum,
rumqttc, filesystem APIs, or Edge internals. Adapter implementations depend
only on the API and serialization helpers allowed by the layer checker.

Most internal items remain private or `pub(crate)`. `anyhow` is limited to the
binary composition root. Domain and operation failures use closed typed errors.

## 5. Technology choices

- Rust edition and resolver: the root workspace values;
- asynchronous runtime: Tokio;
- process task ownership: `tokio_util::sync::CancellationToken` and owned
  `JoinSet`/task tracking;
- HTTP and middleware: Axum, Tower, and tower-http;
- SSR templates: Askama with automatic escaping and typed view models;
- database access: SQLx with separate SQLite and PostgreSQL implementations;
- MQTT: rumqttc with Rustls;
- CLI: Clap;
- serialization: Serde and serde_json;
- password hashing: argon2/password-hash;
- secret wrappers and clearing: secrecy and zeroize where plaintext must exist;
- backup encryption: Argon2id-derived key and XChaCha20-Poly1305;
- tracing: tracing with a secret-safe field policy.

The design does not use `sqlx::Any`, an ORM, a custom query DSL, a global
service locator, a generic event bus, or an actor framework.

## 6. Storage

SQLite and PostgreSQL expose the same operation-oriented storage facade but
retain backend-specific SQL, transactions, locks, and migrations.

Examples of atomic operations are:

- accept an Edge Node record batch and advance the contiguous raw cursor;
- persist an activation request/result transition;
- append semantic results and enqueue all resulting output publications;
- mutate configuration and append its audit event;
- claim, mark, or release durable outbox work;
- create or revoke an account/session;
- persist backup/restore history.

Each operation starts and commits its own transaction. SQLx transaction values
never escape into HTTP handlers or application services. Common tests execute
the same operation vectors against temporary SQLite and a real PostgreSQL
instance.

SQLite uses WAL, `synchronous=FULL`, foreign keys, busy timeout, and initially a
single connection/writer. PostgreSQL uses explicit row/advisory locks and the
existing singleton deployment rule. Unique constraints and database locks,
not Tokio task scheduling, enforce record ordering and idempotency.

SQLite and PostgreSQL have separate migration directories. The initial Rust
baseline represents only the final supported logical schema. Once committed,
Rust migrations are append-only and checksum checked.

## 7. MQTT custody and delivery

For Edge Node intake:

1. validate identity, topic, epoch, sequence, bounds, and payload;
2. transactionally insert raw records and advance the contiguous cursor;
3. commit;
4. publish `accepted-through`.

Gap, conflicting replay, storage failure, cancellation, or capacity refusal
must not produce a custody acknowledgement.

For external output:

1. create an exact MQTT publication and durable outbox row in the semantic
   transaction;
2. send one bounded in-flight QoS 1 publication;
3. correlate rumqttc EventLoop PUBACK with the claimed outbox identity;
4. transactionally mark the outbox row published.

`AsyncClient::publish().await` means queued to the client, not delivered.
Only EventLoop PUBACK permits the durable state transition. A crash before
PUBACK resends the same topic/payload/identity. A crash after PUBACK but before
the database mark can duplicate delivery and remains safe under the documented
at-least-once contract.

Ingest/control and external-output MQTT use separate client IDs and EventLoops.
Broker PUBACK and IoTKit custody acknowledgement use distinct types, metrics,
and log names.

## 8. Output Adapter extension model

Output Adapters are trusted Rust source compiled into IoTKit Edge. They are not
sandboxed. Project guidance forbids filesystem, environment, network, secret,
thread, and clock access in adapters, and layer checks enforce the available
dependencies.

Each Adapter is its own package. Adding an Adapter changes its package,
workspace membership, and one static registry file only.

The API has two independent extension roles:

1. **Transform API** validates versioned non-secret route configuration and
   deterministically converts a typed generic Observation into one exact MQTT
   publication.
2. **Profile Policy API** describes supported modes, non-secret setup fields,
   whether external confirmation is required, and deterministically expands
   generic semantic-rule inventory into route configuration proposals.

The policy never writes storage or renders HTML. Edge persists generic profile,
binding, route, and confirmation state and renders generic typed form metadata.
There are no provider-ID switches in storage, application, HTTP, or Console
modules.

The API uses typed Observation variants rather than an unvalidated raw JSON
value while preserving the existing fixture wire representation. Errors are a
closed enum. The testkit verifies descriptors, modes, closed configuration,
kind compatibility, identity and time bounds, topic validity, QoS 1, valid JSON
payloads, panic freedom, and byte-for-byte determinism.

The vendor-neutral example is built and tested on every API change but is not
registered in the production binary.

## 9. HTTP, Console, and security

Existing TypeScript, CSS, SVG, OpenAPI, routes, and browser journey are reused.
The rewrite does not redesign navigation or styling.

Axum handlers:

- parse and bound transport input;
- authenticate and enforce CSRF/origin rules;
- invoke one application operation;
- map typed results to the existing response contract.

Handlers do not contain SQL. Askama templates use typed view models and
automatic escaping. Compatibility compares routes, statuses, redirects,
security headers, cookies, significant DOM hooks, form behavior, JSON, and CSV,
not byte-identical HTML whitespace.

Password and session behavior is specified by existing fixtures rather than a
new session framework's defaults. Passwords, tokens, private keys, connection
credentials, and session secrets are never included in `Debug`, logs, audit
details, errors, or fixtures.

## 10. Lifecycle and shutdown

Every long-running task is owned. Detached `tokio::spawn` is forbidden.
Unexpected termination of the HTTP listener, ingest MQTT EventLoop, output MQTT
EventLoop, or critical storage worker initiates a failed process shutdown.

Graceful shutdown order:

1. mark readiness false;
2. stop accepting HTTP and drain in-flight requests;
3. stop claiming new MQTT and outbox work;
4. let active database operations finish;
5. drain bounded PUBACK/mark work for a configured finite interval;
6. leave unfinished rows pending;
7. stop EventLoops and close database pools.

Cancellation is observed between atomic operations, not by dropping a commit
future inside `select!`.

## 11. Migration strategy

Issue #83 and one draft PR contain the replacement, but the branch remains
green through reviewable commits:

1. design, plan, Rust composition skeleton, and external parity harness;
2. Output Adapter API, testkit, example, and built-ins;
3. backend-specific fresh storage and store contract;
4. MQTT custody, descriptor, activation, and acknowledgement;
5. authentication, session, API security, and audit;
6. semantic evaluation and durable output;
7. API, SSR Console, history, and CSV;
8. backup, restore, diagnostics, capacity, and CLI;
9. Docker, Compose, scripts, CI, and documentation cutover;
10. full release gate, independent reviews, and Go deletion.

Go is an oracle, not a library. Go and Rust run in isolated directories with
independent databases and MQTT client IDs. The differential suite normalizes
only injected time, UUID, request IDs, and password-hash salts. It compares:

- MQTT topics, payloads, QoS, retain, order, acceptance, and rejection;
- HTTP status, JSON, headers, cookies, redirects, and closed errors;
- primary DOM hooks and browser journeys;
- CSV bytes;
- CLI flags, stdout, stderr, JSON fields, and exit statuses;
- Output Adapter bytes;
- diagnostics and audit public fields;
- backup/restore outcomes;
- restart and failure convergence.

The final deployment switch and Go deletion happen together after the Rust
binary passes all gates.

## 12. Verification and completion

Required before Go deletion:

- Rust unit tests and Clippy with `-D warnings`;
- Output Adapter API/testkit/example and implementation matrix;
- SQLite/PostgreSQL store contract and transaction fault matrix;
- Go/Rust black-box differential suite;
- actual Mosquitto QoS, reconnect, PUBACK, crash-window, TLS, auth, and ACL
  tests;
- Axum API and security tests;
- browser E2E using the existing journey;
- `test-edge-bootstrap.sh`;
- `test-edge-output.sh`;
- `test-edge-resilience.sh`;
- `test-mqtt-security.sh`;
- `test-edge-postgres.sh`;
- `test-edge-capacity.sh`;
- encrypted backup/restore negative matrix;
- complete `test-edge-host-release-gate.sh`;
- Rust Docker image build, start, health, login, and non-root execution;
- stale Go source, dependency, CI, cache, and command reference scan;
- English/Japanese OKF and operator documentation checks;
- independent custody, security, operations, and adapter-author reviews.

Target-hardware capacity measurement remains outside this Issue. Existing host
capacity smoke and regression reports remain mandatory.

## 13. Rejected alternatives

- Continuing permanent Go/Rust split: preserves the adapter-author problem.
- A one-shot clean-room rewrite without an oracle: hides semantic regressions
  until the end.
- Go/Rust FFI or production dual-write: adds disposable architecture and new
  failure modes.
- Wasm now: solves runtime plugin installation, which is not a current
  requirement.
- Tauri: IoTKit Edge is a LAN server accessed from Windows browsers, not a
  desktop application.
- Microcrates for every internal layer: raises onboarding and build overhead
  without an external boundary.
- Console redesign during the rewrite: makes parity impossible to judge.
