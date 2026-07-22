# IoTKit documentation

This is the entry point for current IoTKit product documentation. Start with the
[product model](product-model.md), then follow the document for your role.

The curated public knowledge bundle is available in
[日本語](okf/ja/index.md) and [English](okf/en/index.md). It follows Open Knowledge Format v0.1
and intentionally excludes historical design and process records.

## Product path

```text
vendor/protocol device -> Input Adapter --+
contract-native device -> HTTPS ingest ---+-> IoTKit Edge Node
  -> internal MQTT Broker
  -> IoTKit Edge
  -> generic semantic observation
  -> Output Adapter
  -> external MQTT Broker
  -> application such as YokaKit
```

The two MQTT-facing boundaries have different purposes:

- [Edge Node custody contract](exit-contract.md): Edge Node -> IoTKit Edge. It defines
  raw record transfer, durable custody acknowledgement, replay, and purge authority.
- [Output Adapter contract](output-adapter-contract.md): IoTKit Edge -> external
  application. It defines deterministic transformation of generic semantic
  observations into an application-specific publication.

YokaKit and other business applications use the Output Adapter boundary. They do
not consume the raw custody stream and do not authorize Edge Node purging.

## Where to start

| Reader | Start here | Boundary or purpose |
|---|---|---|
| Installer or operator | [IoTKit Edge installation and recovery](edge-operations.md) | Deployment, accounts, certificates, backup, restore, and storage migration |
| Core contributor | [Architecture](architecture.md) | Component map, code placement, data flow, and invariants |
| HTTP device builder | [Authenticated ingest contract v1](ingest-contract.md) | Device -> Edge Node HTTP ingestion |
| Rust sensor integration developer | [Input Adapter host contract v1](input-adapter-contract.md) | Sensor/vendor code -> generic Edge Node host |
| Edge Node custody implementer | [Edge Node custody contract](exit-contract.md) | Edge Node -> IoTKit Edge raw custody over MQTT |
| External application integrator | [Output Adapter contract v1](output-adapter-contract.md) | IoTKit Edge -> application-specific MQTT |
| Capacity evaluator | [Storage capacity regression smoke](edge-capacity.md) | Evidence-based sizing for embedded and PostgreSQL profiles |

## Source-of-truth order

Contract authority is a set, not a single file:

1. A versioned contract consists of its machine-readable schema or exported wire
   types, shared fixtures and conformance tests, and its current contract document.
   The document owns authority, custody, retry, and relationships that schemas
   cannot express by themselves. None of these artifacts silently overrides the
   others; a disagreement is a contract defect that must be resolved explicitly.
2. The [product model](product-model.md) and [Architecture](architecture.md) for
   scope, components, and code-placement rules, then operational runbooks for
   procedures.
3. [`redesign/`](redesign/README.md) terminology, responsibility ledger, and
   decisions for rationale and invariants that current documents still cite.
4. Git history, redesign inputs/reviews, and archived implementation plans.

Each boundary has an explicit owner and derived checks:

| Boundary | Primary contract authority | Derived implementation or verification |
|---|---|---|
| Authenticated HTTP ingest | `ingest-contract.md`; exported `iotkit-ingest-contract` types are its shipped Rust representation | HTTP handlers and ingest tests |
| Edge Node custody | `exit-contract.md` plus shared egress fixtures and the matching Rust/Go validators; neither language definition wins alone | MQTT/store integration tests |
| Input Adapter host | Exported `iotkit-input-adapter-host-api` types plus `input-adapter-contract.md` semantics | Adapter testkit and host integration tests |
| Output Adapter | Exported Go adapter/route types plus `output-adapter-contract.md`; shared fixtures cover the explicitly listed cases | Adapter and publisher tests |
| IoTKit Edge Console API | OpenAPI for the endpoints it currently covers | Generated TypeScript and Console tests; uncovered routes remain a documented contract gap |
| Rust layer placement | `architecture.md` placement rules and `scripts/check-layers` classifications together | `scripts/check-layers` CI result |
| Database evolution | Ordered Rust/Go migrations, schema constraints, and profile metadata | Migration, backup, restore, and profile tests |

Tests prove conformance to the owning contract; an arbitrary test is not by itself
a new product contract.

The IoTKit Edge Console OpenAPI currently covers only the endpoints used to
generate shared Console types. It must not be treated as a complete inventory of
all HTTP routes until that coverage is completed.

## Historical material

- [`redesign/`](redesign/README.md) separates still-cited decision rationale from
  time-bound inputs, reviews, and migration records. Its implementation-status
  statements do not override the current contract set or architecture.
- [`superpowers/`](superpowers/README.md) preserves completed design and
  implementation process records. Those files are not current work instructions.
- [`cloud-development.md`](cloud-development.md) is an optional internal automation
  guide, not a product contract or an installation requirement.

The whole `docs/` tree is not an OKF bundle: historical material intentionally
remains here. The isolated [`docs/okf/`](okf/index.md) bundle contains only a small,
mirrored Japanese/English current corpus. A clean public snapshot can publish it
alongside the detailed contract artifacts without classifying or translating this
archive.
