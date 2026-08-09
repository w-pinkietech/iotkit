---
type: Contract
title: "IoTKit v1 compatibility policy"
description: "Defines product-1.x compatibility commitments, independent version domains, mixed-version operation, and migration boundaries."
language: en
translation_key: contracts.compatibility-policy-v1
status: stable
revision: 4
---

# IoTKit v1 compatibility policy

Status: **normative when the product reaches 1.0.0**. It defines the
compatibility promise for product major version 1; it does not retroactively
turn any 0.x release into a compatible release series.

## 1. Release artifact and version domains

One product version covers the Cargo workspace. The release tag and its
GitHub-generated source archive are the release artifact. The tag's
`workspace.package.version` binds the artifact to the independent contract and
storage versions recorded in
`testdata/compatibility/v1/release-manifest.json`; that manifest intentionally
does not repeat the product version. Its checker rejects unknown keys, missing
required domains, duplicate IDs, empty required evidence categories, unsafe or
symlink-escaping paths, paths absent from the source tree, and a recorded
storage schema version that differs from the maximum numeric migration version
in its listed directories. A moving `master` link is not release evidence.

The following versions are independent. A product minor release need not
change any of them, and a change to one does not silently change another.

| Domain | Version unit | Public authority and evidence |
| --- | --- | --- |
| Device ingest | `/api/v1` and the JSON `Envelope` / `EnvelopeAck` contract | `ingest-v1.md`, `iotkit-ingest-contract`, its fixtures and tests |
| Edge Node custody | MQTT topic major `iotkit/v1` plus each payload `schema_version` | `edge-node-custody-v1.md`, publisher/receiver types, fixtures, and tests |
| Descriptor snapshot | Its payload `schema_version` | The v1 topic currently carries descriptor body schema 2; topic and body versions are independent |
| Console JSON | `edge/openapi/edge-console-v1.yaml` | The OpenAPI-described operations and their generated browser types |
| Input Adapter | `adapter_api_major` and `config_schema_version` | `input-adapter-v1.md` and the compile-time host API |
| Output Adapter | Versioned Adapter ID, configuration schema, and payload schema | `output-adapter-v1.md`, including `iotkit.mqtt-json.v1` and `pinikiet.mqtt.v1` |
| Persistent storage | Per-store migration/schema number | Node SQLite, IoTKit Edge SQLite, and IoTKit Edge PostgreSQL evidence in the release manifest |

The manifest is an index, not a second contract authority. A disagreement
between the paired product documents, code/schema, fixtures, and conformance
tests is a contract defect.

## 2. Public and same-release surfaces

For product 1.x, these are public compatibility surfaces:

- authenticated HTTP ingest at `/api/v1`;
- the Edge Node MQTT custody v1 topics and exact supported payload schemas,
  including the independently versioned descriptor body schema;
- only the Console JSON operations and schemas described by
  `edge/openapi/edge-console-v1.yaml`;
- Input Adapter API major 1 and its stated configuration schemas; and
- versioned Output Adapter IDs, route configurations, and payloads.

The following are intentionally **same-release only** and are not independent
client contracts: Console JSON routes absent from the OpenAPI document,
server-rendered HTML, DOM, CSS, form actions, Edge Node private control API,
human-oriented CLI display text, and private Rust types. They may change when
their matching product release changes. A new public contract requires its own
versioned authority and evidence before it receives a compatibility promise.

## 3. Product-1.x support window

Starting with 1.0.0:

- a product **minor** release may add backward-compatible public behavior and
  may include compatible fixes;
- a product **patch** release contains compatible fixes and does not
  intentionally add features; and
- both preserve supported public contract majors; additions are compatible only
  under the evolution rules below;
- public contract major v1 remains supported through the whole product 1.x
  series; removal or an incompatible replacement happens no earlier than the
  next product major;
- the latest product release receives fixes. IoTKit makes no calendar support
  window, backport, or security-fix SLA for superseded 1.x binaries; and
- deprecated v1 behavior remains documented and tested until its announced
  next-major removal.

This is different from the current pre-1.0 rule: `0.MINOR.0` may intentionally
change compatibility and `0.MINOR.PATCH` is for compatible fixes. See
`RELEASING.md` for both periods.

## 4. Evolution and unknown input

Version 1 is not a promise that every decoder is extensible in the same way.

| Boundary | Compatible evolution within v1 | Breaking evolution |
| --- | --- | --- |
| Tolerant HTTP ingest objects | An optional field may be added only when released v1 readers demonstrably ignore it and sender/receiver behavior stays unambiguous. | New required data, new enum/tag value, changed meaning, or a new URL major requires a new contract version. |
| Strict MQTT custody payloads and Adapter configuration/payloads | None by implicit field addition. | A field, record family, enum value, or schema change requires a new explicit payload/configuration/Adapter version. |
| Console OpenAPI schemas | Add only an optional documented field when the OpenAPI schema and all supported readers permit it. | A closed schema change, required field, enum change, or removed operation requires a new public version. |

The current ingest v1 Rust objects deliberately ignore otherwise-valid unknown
object members; they do not preserve or re-emit them. Unknown enum or tagged
variant values fail decoding. `/api/v1` has no request-body schema-version
negotiation, and an unsupported API-version path is not an alternate ingest
contract. This tolerance is a specific ingest behavior, not a blanket rule for
other surfaces.

Unknown explicit contract versions fail closed. A receiver must not guess a
field, enum, topic, payload, configuration, or database interpretation from a
future version.

## 5. Mixed-version operation

| Combination during product 1.x | Commitment / operator action |
| --- | --- |
| Existing ingest v1 client → newer 1.x Edge Node | Supported. |
| Existing Edge Node emitting a supported custody v1 payload → newer 1.x IoTKit Edge | Supported. |
| Newer Edge Node → older IoTKit Edge | Not guaranteed. Upgrade IoTKit Edge first, then Edge Nodes. |
| Existing v1 Output Adapter consumer → newer 1.x IoTKit Edge | Supported for the same versioned Adapter ID and exact payload contract. |
| Existing Console JSON v1 API client → newer 1.x IoTKit Edge | Supported for the OpenAPI-described public subset. |
| Browser assets from an older Console → newer IoTKit Edge | Not supported. Console assets are a matched, no-store same-release surface. |
| Older `nodectl` or direct database access → newer Node schema | Not supported. Use the matching release's tooling; direct database mutation is never a compatibility path. |
| Input Adapter compiled for API major 1 → newer Edge Node | Its supported source/configuration contract remains v1, but adapters rebuild with the Edge Node release. |

MQTT PUBACK remains Broker receipt only. It never establishes durable IoTKit
acceptance or makes a mixed-version path safe by itself.

## 6. Storage migration and rollback

Each release manifest records the exact Node SQLite, IoTKit Edge SQLite, and
IoTKit Edge PostgreSQL schema evidence for that source archive. Every released
1.x database schema must have a tested forward migration to later 1.x releases.
There are no down migrations and no image-only rollback promise.

For a failed update, stop the new binary, preserve the encrypted pre-update
backup, restore that backup into a new candidate, and switch to the matching
old binary as described by the current recovery runbook. Do not open a
migrated database with an older binary. SQLite-to-PostgreSQL migration accepts
the current release schema only; first start the current IoTKit Edge to migrate
an older SQLite source, stop it, then copy to an empty PostgreSQL target.

Pre-1.0 databases and backup artifacts are covered only by their currently
tested runbooks. They are not a product-1.x preservation promise.

## 7. Breaking-change and emergency process

Before a planned incompatible change, maintainers must:

1. record the motivation, affected version domain, supported old/new range,
   upgrade order, and removal target in an issue and the current authority;
2. introduce a new explicit version, path, topic, Adapter ID, or schema rather
   than reinterpret old input;
3. ship paired English/Japanese documentation, types/schemas, fixtures,
   conformance tests, migration/dual-read evidence where applicable, and an
   updated release manifest; and
4. publish migration and deprecation notices in the release notes before the
   old supported version is removed.

An urgent security or data-loss prevention change may fail closed before that
normal period. It must state the affected versions, preserve data where safe,
provide the recovery or migration action, and be recorded in the release notes
and product authority. It is an exception to availability, never permission to
silently reinterpret or discard customer data.

## 8. Explicit non-goals

This policy does not preserve every unpublished pre-v1 behavior forever. It
does not by itself make every Console route an OpenAPI API, unify independent
descriptor validators, add an external consumer gate, or create historical
golden databases. Those need their own issue and compatibility evidence before
they can extend this promise.
