---
type: Contract
title: "IoTKit Input Adapter host contract v1"
description: "Defines the complete Input Adapter identity, authority, host API, configuration, lifecycle, and conformance contract."
language: en
translation_key: contracts.input-adapter-v1
status: stable
revision: 5
---

# IoTKit northbound Input Adapter host contract v1

Status: Accepted and implemented (2026-07-20).

## 1. Scope

This contract defines the northbound extension boundary for official
in-process sensor adapters compiled into IoTKit Edge Node. It keeps the generic host
and core independent of BravePI while allowing the Edge Node composition root to
link selected vendor adapter crates.

The only measurement contract is the existing
`iotkit-ingest-contract::Envelope` / `EnvelopeAck`. This document does not add
another payload, Ack, measurement vocabulary, or device identity.

This is not the complete D4 adapter contract. A complete D4 adapter also has a
separate D12 care-servicer channel. Passing this contract does not claim
capability-declaration convergence, care-servicer completion, or D12 form-①
completion.

V1 uses a compile-time catalog. Adding an installed adapter type requires
rebuilding Edge Node. Dynamic libraries, runtime plugin discovery, and Console-based
adapter installation are excluded.

## 2. Identity and authority

| Identity | Owner | Meaning |
|---|---|---|
| `adapter_type_id` | adapter package/build | Software type, e.g. `bravepi-mainboard` |
| `adapter_instance_id` | deployment config | Stable configured instance on one Edge Node |
| `configured_source` | Edge Node composition | Diagnostic envelope source |
| `principal_id` | Edge Node collector boundary | Receiver-owned admission/dedup authority |
| `subject_hint` | observation | Hardware/protocol identity of the observed device |
| `system_id` | Edge Node ledger | IoTKit-owned stable device identity |

These values MUST remain distinct. A transport path is configuration, not
instance identity. Type and instance IDs remain stable across restart and
device-path aliases.

Validation is closed:

- type: 1–63 ASCII bytes,
  `[a-z][a-z0-9]*(?:-[a-z0-9]+)*`;
- instance: 1–63 ASCII bytes,
  `[a-z][a-z0-9]*(?:[-_][a-z0-9]+)*`;
- source: 1–128 ASCII bytes,
  `[A-Za-z0-9][A-Za-z0-9._:/-]*`.

No trimming, case folding, Unicode normalization, or automatic suffixing is
performed. A collision is a startup error.

For v1 official adapters, Edge Node preserves the current `OfficialDiscovery`
subject scope and creates `principal:{configured_source}`. Source grants no
authority. Edge Node injects a source-bound submission facade, so adapter code
cannot choose `Envelope.source` or principal scope.

For positional device identities, `subject_namespace` is exactly
`configured_source`; it is not separately configurable. Adapters with globally
stable hardware identifiers receive the namespace but need not include it in
their `subject_hint`.

## 3. Responsibilities

The northbound package remains the D4 composition:

```text
physical device
  -> transport backend
  -> device driver / codec
  -> shared adapter runtime
  -> adapter-package composition glue
  -> SourceBoundIngest / IngestClient
  -> receiver-owned principal
  -> Edge Node collector / ledger / timeseries
```

Driver and runtime modules know neither the ingest contract nor the client.
Package composition glue owns the host API. Adapter crates do not access Edge Node
SQLite or depend directly on collector, ledger, registry, timeseries, publish,
ops, engine, or IoTKit Edge.

Edge Node owns principal creation, configuration authorization, inventory mutation,
start/stop/restart policy, backoff, exhaustion, health aggregation, and the
static type catalog. The same principal-bound client survives adapter restarts.

## 4. Host API

`iotkit-input-adapter-host-api` contains supervision-free composition
primitives:

```text
AdapterStartContext {
  instance_id,
  configured_source,
  subject_namespace,
  ingest: SourceBoundIngest,
}

SourceBoundIngest::try_submit(items)
  -> EnqueuedEnvelope { envelope_id, delivery: DeliveryReceipt }
  | QueueSubmitError::{Full(RetryHandle), Closed(RetryHandle)}

SourceBoundIngest::try_retry(RetryHandle)
  -> EnqueuedEnvelope
  | RetryQueueError::{Full(RetryHandle), Closed(RetryHandle), SourceMismatch(RetryHandle)}

DeliveryOutcome {
  Final(EnvelopeAck),
  AbandonedBeforeFinal {
    reason: SpoolOverflow | ClientShutdown | CollectorClosed,
    retry: RetryHandle,
  },
}

RunningInputAdapter {
  instance_id,
  activity,
  diagnostics,
  completion,
  shutdown,
}
```

The facade constructs an immutable envelope with the bound source, the existing
ID recipe, and `declaration_version=None`. Queue admission is non-durable and
transfers no custody.

`DeliveryReceipt`, `DeliveryOutcome`, and opaque `RetryHandle` are owned by
`iotkit-ingest-client`; the host API only wraps or re-exports them.
`IngestClient::try_submit_with_receipt(envelope)` keeps the existing
fire-and-forget method compatible while attaching one receipt sender to each
spool entry.

Every retained receipt resolves exactly once. Final `Accepted`, `Duplicate`,
or terminal `Rejected` returns the exact Ack. `Deferred` and no-ack leave it
pending. Drop-oldest spool eviction or client/collector termination returns
`AbandonedBeforeFinal` with the exact immutable envelope and ID in an opaque
retry handle.

`SourceBoundIngest::try_retry(handle)` validates the bound source and requeues
that unchanged envelope; a front-door `Full` error returns ownership of the
handle to the caller. Initial `try_submit` `Full`/`Closed` likewise returns a
handle for the envelope it already constructed. Package composition uses only final Ack semantics to
advance or clear an upstream cursor. Local abandonment never manufactures
custody or advances that cursor.

Activity is a coalescing latest-value snapshot with process-monotonic
timestamps for successful physical decode and client-queue admission, a
dropped-diagnostics counter, and the interface state. The interface state is
`Unreported`, `Open`, or `OpenFailed` (a `std::io::ErrorKind` and a short
description); an adapter that opens its hardware interface itself (serial, I2C,
GPIO) reports it through `interface_opened` / `interface_open_failed`. The host
publishes it as the status fault `interface-open-failed`
([MQTT Output Adapter contract v1](mqtt-output-adapter-v1.md), section 7.1) and
clears it on `Open`. For an adapter whose `start` returns a `std::io::Error`
because it cannot open its interface, that error kind becomes the reason of the
same fault. Diagnostics are bounded, best-effort, redacted,
and use generic kinds:

```text
Transport | Protocol | Decode | MeasurementMapping
ClientQueueFull | ClientClosed | DeviceUnavailable
```

Optional adapter-specific codes are namespaced by type ID. Diagnostics are
facts, not authoritative health.

Completion is lossless and resolves only after every adapter-owned async task
and blocking reader thread stops:

```text
RequestedStop
UnexpectedExit { TransportClosed | WorkerReturned | ClientClosed | InternalInvariant }
Panic
```

Shutdown only requests idempotent graceful stop; Edge Node owns the completion
future and bounded timeout. A start error MUST leave no live task, thread, or
open transport.

The host API MUST NOT depend on `edge-node/core/supervision`, expose
`AdapterEvent`/`AdapterCommand`, create principals, access storage, authorize
configuration, define restart policy, or assert health.

## 5. Type catalog and configuration

Each built-in supplies a non-secret `InputAdapterTypeDescriptor`:

- `adapter_type_id`;
- `adapter_api_major`;
- `config_schema_version`;
- diagnostic-only `implementation_version`;
- `display_name`;
- `physical_transport_kind`.

Ingest-contract version, adapter API version, config version, implementation
version, and device `declaration_version` are separate domains.

Factories are private to `iotkit-edge-node` and expose only:

```text
descriptor()
parse_and_validate(raw_config)
start(edge_context, validated_config) -> RunningInputAdapter
```

Adapter parsers strictly reject unknown fields and unsupported config versions.
Edge Node validates every enabled instance, identity collision, source binding, and
inventory intent before starting any instance. Factories do not own
`restart()` or `health()`.

In the current compile-time architecture, the raw instance shape is the closed
`RawInputAdapterInstance` in `edge-node/apps/node/src/config.rs`. A new adapter
with new top-level configuration vocabulary must extend that central structure
and its strict deserialization tests. The private factory in
`edge-node/apps/node/src/input_adapters.rs` then validates which fields belong
to its adapter and converts them into the adapter package's typed configuration.
V1 does not yet pass an opaque raw table directly to an adapter-owned parser.

New configuration names instances explicitly:

```toml
[adapters.instances.bravepi_main]
type = "bravepi-mainboard"
enabled = true
config_schema_version = 1
source = "input:bravepi-mainboard:bravepi_main"
port = "/dev/serial0"

[adapters.instances.local_i2c]
type = "rpi-local"
enabled = true
config_schema_version = 1
source = "input:rpi-local:local_i2c"
bus_path = "/dev/i2c-1"
poll_interval_ms = 1000

[[adapters.instances.local_i2c.devices]]
model = "mcp9600"
address = 0x60
thermocouple_type = "K"

[[adapters.instances.local_i2c.devices]]
model = "opt3001"
address = 0x44
```

The table key is the stable instance ID. `source` is required and stable;
principal ID and positional namespace derive from it.

`rpi-local` device selection is deployment configuration, not host-platform
selection. `model` is resolved by the adapter package's compile-time catalog;
Edge Node does not match model IDs. Model-specific scalar settings cross the generic
host boundary as string, integer, float, or boolean values; the adapter catalog
owns their names, types, and validation. MCP9600 requires
`thermocouple_type` with one of
`K`, `J`, `T`, `N`, `S`, `E`, `B`, or `R`. OPT3001 accepts no model-specific
setting. An empty list, unsupported model or setting, invalid I2C address,
duplicate address, or driver-incompatible polling interval fails validation
before any adapter starts. Omitting `devices` preserves the compatibility
inventory of MCP9600 at `0x60` with K-type thermocouple and OPT3001 at `0x44`.
The explicit device list is available on the instance form; the legacy
`[adapters.rpi_local]` form retains its historical fixed inventory.
Removing an entry, disabling the instance, or removing the instance stops the
target from being polled after restart; additive inventory reconciliation does
not delete or silently retire the existing ledger device or its series, so its
descriptor state remains active until an explicit operation changes it. An
operator who is physically removing or replacing an already registered target
must use the ledger retire/replacement journey so history remains auditable.

Raspberry Pi 4B/5 is never selected in this configuration. The transport
backend checks the Linux I2C capability it needs.

Legacy and instance forms are mutually exclusive. Legacy input is converted in
memory before validation without changing behavior:

| Legacy input | Instance | Source / principal | Subject namespace |
|---|---|---|---|
| absent or `[adapters.bravepi]` | `bravepi_main` | `bravepi-mainboard:{resolved_port}` / `principal:{source}` | existing transmitter identity |
| `[adapters.rpi_local]` | `rpi_local_default` | `rpi-local:default` / `principal:rpi-local:default` | `rpi-local:default` |

Absent config still enables BravePI on `/dev/ttyAMA0`; RPi-local remains
disabled. Existing `BRAVEPI_*` and `RPI_LOCAL_*` overrides apply only to legacy
form. Documentation gives exact conversion examples that pin current source
and subject identity. No config rewriter or DB migration is added.

The current slice pins the legacy source/subject recipes in config and mapping
tests, and the R14 inventory test proves repeated reconciliation reuses the
existing `system_id`. Any future change to a source or subject recipe requires
an existing-Edge Node-DB cutover test covering principal scope, hardware identity,
`system_id`, series, and in-window dedup before that change can be accepted.

## 6. Measurements, descriptors, and inventory

Driver/protocol conversion to a physical value and adapter-package projection
to a canonical measurement are adapter-owned. Shared runtimes know neither the
device model catalog nor canonical measurement keys.
Conformance fixtures, not the production type descriptor, record:

- driver value and unit;
- finite identity/linear transform;
- canonical key and UCUM unit;
- value count, channel rule, and series variants.

Fixtures are checked against the measurement registry and exact emitted items.
Vendor codes remain in the adapter crate.

Supported mappings are not connected-device capabilities. The retained Edge Node
descriptor remains derived from actual ledger devices, materialized
non-quarantined series, and provider-neutral registry entries. Full per-device
capability declarations, care verbs, `declaration_version` mismatch, and
redescribe remain a separate D5/D12 state machine.

Positional inventory is an Edge Node-owned mutation:

1. purely validate all instances and combine inventory intents;
2. reconcile idempotently through an audited R14 system-actor operation;
3. update collector cache/generation through the D5 ownership path;
4. start adapters only after commit.

The same resolved target list drives reconciliation and runtime start.
Factories and adapter crates never mutate ledger or registry.

RPi-local's current compile-time supported-device catalog is package-owned. A
typed device entry binds model-specific validation, driver construction,
measurement projection, and inventory display metadata. Edge Node owns only the
adapter type catalog and inventory reconciliation authority; it does not match
on MCP9600, OPT3001, or later IC models. The adapter still owns the positional
subject recipe, so model IDs and host platform names never become device
identity. The deployment-selected model is nevertheless persisted beside the
positional ledger entry as a safety fence. The first reconciliation after this
metadata was introduced binds the known compatibility model to an existing
positional entry. Later configuration that assigns a different model to the
same source and locator fails the whole reconciliation before runtimes start;
it must use an explicit device replacement/cutover rather than silently
reusing history.

The persisted model ID is also the only adapter-origin metadata exported in
descriptor schema 2. It is optional, opaque, and display-only at IoTKit Edge. Adapter
type/instance IDs, configured sources, bus paths, addresses, and other physical
locators remain Edge Node-local deployment details and are not inferred from
`hardware_id`.

## 7. Lifecycle and legacy isolation

Initial start is fail-fast: if one instance fails, Edge Node stops already-started
instances in reverse order and exits non-zero. After successful initial start,
unexpected exit or restart-start failure enters the same bounded Edge Node-owned
backoff and restart budget. Exhaustion is process-lifetime degraded state in
health JSON; systemd process restart resets it.

`scripts/check-layers` checks transitive Cargo reachability for every input
adapter. After this slice only `bravepi-mainboard-adapter`, for its separate
legacy care path, may reach `edge-node/core/supervision`. New adapters may not reach it
directly or through a runtime crate.

`iotkit-polling-adapter-runtime` is migrated to a supervision-free polling
engine that emits decoded observations and lifecycle facts. RPi-local
composition glue performs pure mapping and source-bound submission. This
remains the recommended base for new bus-polling adapters.

BravePI is split at the same northbound seam: its driver/event runtime emits
decoded observations, and package glue maps/submits them. Its legacy southbound
care path may retain frozen supervision until the D12 migration. A
package-private `legacy_projection` wrapper prevents that vocabulary from
becoming the new host contract.

## 8. Conformance and developer experience

The dev-only `iotkit-input-adapter-testkit` reuses ingest types and has no
supervision dependency. It provides reusable assertions for:

- identifier/config validation and catalog uniqueness;
- source binding, subject stability, registry mappings, units, channels, and
  finite items;
- generic lifecycle, shutdown, activity, and bounded diagnostics.

Focused tests in `iotkit-ingest-client` own front-door and post-admission spool
saturation, receipt resolution, unchanged retry handles, final Ack, and
client/collector close. Host API tests own source mismatch and secret-free,
bounded diagnostic surfaces. Adapter packages own mapping fixtures and
leak-free transport cleanup.

Edge Node tests cover factory validation during pure config resolution, multiple
same-type instances, pinned legacy identity, inventory/runtime target parity,
reverse bounded shutdown, process-lifetime exhaustion behavior, generation
fencing, and activity health independent of legacy sensor events. Package
tests cover panic/stop cleanup ordering. Layer tests include a transitive
negative fixture and reject supervision dependencies for every newly
classified adapter.

Adding a third adapter requires all of the following compile-time integration:

1. create its focused crate under `edge-node/adapters/` and add it to the root
   workspace;
2. add the crate dependency to `edge-node/apps/node/Cargo.toml`;
3. extend central `RawInputAdapterInstance` fields and config tests when its
   top-level configuration differs;
4. add one private `InputAdapterFactory`, parse/start/inventory glue, and one
   `catalog()` entry in `edge-node/apps/node/src/input_adapters.rs`;
5. classify the crate in `scripts/check-layers`, update both architecture maps,
   and add package fixtures plus Edge Node catalog/config tests;
6. run package, shared testkit, composition, layer, and source-layout checks.

It does not change collector, storage, MQTT custody, IoTKit Edge, semantic, or
output-adapter code. Provider-specific types remain in the focused crate or the
Edge Node composition root.

The test-only non-catalog `ReferenceAdapter` emits two subjects and two
measurement kinds without provider types. Its production-shaped reference path
exercises descriptor validation, typed config validation, `AdapterStartContext`,
source-bound submission, `RunningInputAdapter`, shutdown, and requested
completion. It is never shipped as production adapter software.

Run the authoring checks from the repository root:

```bash
cargo test -p your-adapter-package
cargo test -p iotkit-input-adapter-testkit
cargo test -p iotkit-input-adapter-testkit \
  reference_adapter_exercises_descriptor_config_start_and_shutdown
cargo test -p iotkit-edge-node input_adapters
scripts/check-layers
scripts/check-source-layout
```

Implementation ordering and completion bookkeeping belong in the implementation
plan, not this contract.

## 9. Exclusions

- runtime plugins or a language-neutral in-process ABI;
- automatic third-party adapter trust/code signing;
- full capability/redescribe convergence;
- generic care-command redesign;
- external Connector, camera, barcode, or actuator work.
