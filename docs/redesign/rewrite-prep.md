# IoTKit / YokaKit Rewrite Prep (historical)

Date: 2026-07-01

This is a historical workspace survey, not current project or workflow authority.
Everything below is a dated snapshot. For current IoTKit decisions use
`docs/README.md`, `docs/okf/`, and `AGENTS.md`.

## Local Repositories

Cloned under `/home/kenta/dev/iot`:

- `iotkit-next`: current Rust IoTKit rewrite prototype
- `yokakit-next`: Go + Vue YokaKit rewrite
- `YokaKit`: legacy Laravel YokaKit
- `iot3_node-red`: legacy Node-RED IoTKit runtime
- `iotkit-reverse-extracted`: reverse-engineered IoTKit requirements/spec workspace
- `monojoh-authority`: binding ADR/contract architecture for IoTKit rewrite direction

Keeping these repositories locally is acceptable. They should not all be treated
as active context. Use this file as the small catalog, then open only the
specific repo and files needed for the current question.

## Repository Catalog

| Repository | What It Is | Default Use | Avoid |
|---|---|---|---|
| `monojoh-authority` | Binding architecture, ADRs, contracts, and downstream implementation plan for the IoTKit rewrite | Start here for new design decisions | Do not treat it as code implementation |
| `iotkit-reverse-extracted` | Reverse-engineered requirements/specification workspace from the old Node-RED IoTKit runtime | Use when checking legacy behavior, protocol facts, or compatibility evidence | Do not reopen `flows.json` unless a specific fact is missing |
| `iotkit-next` | Rust prototype and design notes from the current IoTKit rewrite attempt | Use as implementation seed and proof of prior design direction | Do not let prototype structure override binding architecture |
| `yokakit-next` | Go + Vue rewrite of YokaKit | Use as the readable upstream application/reference receiver | Do not couple IoTKit core vocabulary to YokaKit tables or payloads |
| `YokaKit` | Legacy Laravel YokaKit | Use only for legacy UI/business behavior parity checks | Do not use as architecture guidance |
| `iot3_node-red` | Raw legacy Node-RED IoTKit runtime | Keep as raw artifact fallback | Do not browse broadly; prefer `iotkit-reverse-extracted` summaries |

## Context Hygiene

- Do not run broad searches from `/home/kenta/dev/iot` unless the task is explicitly cross-repo.
- Prefer setting `workdir` to one repository at a time.
- Prefer `rewrite-prep.md` and repo-level docs before reading source files.
- Treat large legacy artifacts as evidence stores, not active working context.
- If a design question can be answered from `monojoh-authority`, avoid loading old runtime files.
- If a compatibility question cannot be answered from `iotkit-reverse-extracted`, then inspect the raw legacy repo narrowly.

## Current Read

### YokaKit

`yokakit-next` is the most readable implementation reference for the YokaKit side.

- Stack: Go single binary, Vue 3 frontend, PostgreSQL, Mosquitto, Docker Compose
- Docs are usable: `docs/PLAN.md`, `docs/architecture.md`, `docs/PROGRESS.md`, `docs/HANDOFF.md`
- Progress file says backend phases through CRUD/API/user/history/reporting are mostly complete.
- Legacy `YokaKit` remains useful mainly for UI parity and business behavior comparison.

### IoTKit

There are three different authority layers:

- `iot3_node-red`: old runtime artifact; useful as evidence, not as a design target
- `iotkit-reverse-extracted`: reverse-engineered requirements and compatibility contracts
- `monojoh-authority`: stronger rewrite architecture baseline with ADRs and contracts
- `iotkit-next`: Rust implementation prototype based on a simpler modular-monolith direction

`iotkit-next` already contains real code for:

- `edge-node/core/types`: `AdapterEvent`, `AdapterCommand`, sensor identity/reading, device keys
- `edge-node/core/engine`: read-model/projection ingesting adapter events
- `edge-node/core/storage`: SQLite migration/storage handle
- `edge-node/core/timeseries`: SQLite time-series storage
- `bravepi-mainboard-adapter`: BravePI UART path
- `rpi-local-adapter`: direct RPi I2C path
- `iotkit-polling-adapter-runtime`: shared polling adapter runtime
- `iotkit-gateway`: composition root and adapter fan-in

`iotkit-next` also preserves useful design notes:

- `_legacy-remake/remake-plan.md`
- `_legacy-remake/open-questions-adapter-architecture.md`
- Historical implementation specs and plans remain available in Git history; they are not current authority.

## Important Design Direction

The better long-term shape is not "IoTKit directly knows YokaKit".

Recommended boundary:

- IoTKit core runtime owns normalized sensor/device events, command outcomes, state, bounded history, and outbox.
- YokaKit integration is a publisher/projection adapter.
- YokaKit payload names must not become IoTKit core vocabulary.

This matches `monojoh-authority/docs/adr/0028-yokakit-publisher-as-projection-adapter.md`.

## IoTKit Architecture Direction

Keep these decisions:

- Provider-neutral core. BravePI/BraveJIG are adapters, not domain primitives.
- Separate input pipeline and command pipeline.
- Keep host/hardware access behind a host-agent or adapter boundary.
- Support same-device and split-device topology through explicit contracts.
- Use bounded config and typed contracts, not Node-RED-style open behavior graphs.
- Keep adapter extensibility in-repo first; avoid early dynamic plugin systems.

Potential conflict to resolve:

- `iotkit-next` is currently a modular monolith with in-process `mpsc` adapter fan-in.
- `monojoh-authority` wants explicit transport-capable seams for same-device and split-device deployment.

Pragmatic resolution:

- Keep `iotkit-next` in-process for first implementation speed.
- Define the adapter-to-collector envelope and host-control API now, so the in-process path is one transport binding rather than the only architecture.

## Near-Term Work Plan

1. Establish authority order.
   - Binding architecture: `monojoh-authority`
   - Legacy behavior/spec evidence: `iotkit-reverse-extracted`
   - Current implementation seed: `iotkit-next`
   - YokaKit behavior reference: `yokakit-next` and legacy `YokaKit`

2. Reconcile `iotkit-next` with `monojoh-authority`.
   - Map existing Rust crates to ADR package boundaries.
   - Decide what stays, what is renamed, and what needs a new seam.

3. Define first executable IoTKit slice.
   - BravePI-only compatibility path
   - RPi local I2C path as second adapter
   - Collector envelope
   - State projection
   - SQLite local storage
   - Minimal `gatewayctl` or JSON CLI surface
   - YokaKit publisher stub

4. Use `yokakit-next` as the upstream receiver model.
   - Do not couple IoTKit core to YokaKit tables.
   - Define an explicit publisher contract for production/sensor/alert/status records.

5. Fix test hygiene before expanding.
   - `iotkit-next` config tests mutate process env and fail under parallel test execution.
   - Current workaround: `RUST_TEST_THREADS=1 cargo test --workspace`
   - Better fix: serialize env-mutating tests or refactor config loading tests to avoid global env mutation.

## Verification

`iotkit-next`:

- `RUST_TEST_THREADS=1 cargo test --workspace` passes.
- Plain `cargo test --workspace` is flaky/failing because config tests mutate environment variables in parallel.

`yokakit-next`:

- `go test ./...` could not be run because `go` is not installed in this environment.
- Docker is available, so a containerized Go test path is possible if needed.
