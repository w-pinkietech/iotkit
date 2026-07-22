---
type: Concept
title: "IoTKit product model"
description: "Defines the complete product scope, component responsibilities, authority flow, deployment choices, and extension boundaries."
language: en
translation_key: concepts.product-model
status: stable
revision: 2
---

# IoTKit product model

Status: Current product concept.

IoTKit is the reusable layer below factory and business applications. It collects
sensor observations, preserves them through failures, lets an operator assign
generic meaning, and exports versioned application-specific messages. It does not
own products, work orders, processes, OEE, business alarms, or a factory hierarchy.

## Components

| Component | Responsibility | Does not own |
|---|---|---|
| Device | Measures or detects a physical condition | IoTKit durability or business meaning |
| Input Adapter | Translates a vendor/protocol-specific input into the generic Edge Node ingest boundary | Storage, retry policy, custody, semantic rules, or external output |
| Authenticated HTTP ingest | Accepts bounded contract-native device envelopes without an in-process Input Adapter | Device firmware, vendor decoding, or business meaning |
| IoTKit Edge Node | Collects, normalizes, durably buffers, and retries records close to devices | Cross-Edge aggregation or business logic |
| Internal MQTT Broker | Transports Edge Node records and control messages | Durable application custody or data authority |
| IoTKit Edge | Accepts raw custody, discovers and activates Edge Node incarnations, stores device/signal descriptor replicas, owns Edge-scoped display/semantic/output settings, serves the Console, and owns durable output delivery | Edge Node device identity/inventory/desired-configuration authority, factory/business masters, or application workflows |
| Output Adapter | Deterministically converts a generic semantic observation into one external application's topic and payload | Broker credentials, retry scheduling, durable outbox state, or semantic evaluation |
| External application | Uses IoTKit observations for its own domain | IoTKit raw custody and Edge Node purge authority |

An IoTKit Edge can manage multiple Edge Nodes. IoTKit itself does not require a
factory concept; an `edge_id` identifies one IoTKit Edge scope. Aggregation across
multiple `edge_id` values belongs to an optional fleet or application layer.

## Data and authority flow

1. A vendor/protocol device reports through an in-process Input Adapter, or a
   contract-native device sends an authenticated HTTP envelope. Both paths converge
   at the Edge Node collector.
2. The Edge Node resolves stable IoTKit identity and stores the observation.
   Only an active Edge Node incarnation and a non-quarantined record that passes
   publication admission receive publication state in the same transaction and are
   published through the internal Broker. Pre-activation readings remain local and
   are never replayed into the custody stream. Quarantined readings have no outbox
   while quarantined; a later release may enqueue them only through the durable
   activation and publication-admission gate defined by the custody contract.
3. IoTKit Edge atomically stores the validated raw record and contiguous cursor in
   its selected authoritative storage profile.
4. Only IoTKit Edge's application-level `accepted-through` transfers custody and
   makes the corresponding Edge Node records purge-eligible. MQTT PUBACK alone does
   not do this.
5. IoTKit Edge evaluates operator-defined generic semantic rules without changing
   the accepted raw record.
6. A selected Output Adapter creates the exact external MQTT topic and payload. Its
   delivery lifecycle is independent of raw custody acceptance.

## Deployment choices

Edge Node uses local SQLite as its durable buffer. IoTKit Edge selects exactly one
authoritative storage profile:

- `embedded`: SQLite on the IoTKit Edge host, suited to standalone and lower-
  concurrency deployments within a measured capacity envelope;
- `postgres`: PostgreSQL, suited to deployments that need a separate database host,
  higher concurrency, or a larger measured capacity envelope.

Both profiles implement the same product contract and are valid for production
inside a verified capacity envelope. They are not feature or reliability tiers;
deployment measurements determine which profile is appropriate.

IoTKit Edge never dual-writes both profiles and never silently falls back to an
empty backend. Moving from SQLite to PostgreSQL is an explicit offline migration
with identity, cursor, and outbox verification.

The Brokers may be co-located with IoTKit Edge or run on separate hosts. Hostname,
network, certificate, and credential provisioning are deployment responsibilities;
the Console selects configured Output Adapters but does not provision Broker
infrastructure.

## Extension boundaries

- Add a sensor or vendor protocol through an Input Adapter and, when useful, a
  reusable driver.
- Or implement the authenticated HTTP ingest contract directly on a capable
  contract-native device.
- Add a destination application through an Output Adapter.
- Keep vendor-specific and application-specific identifiers inside those adapters.
- Add a wire field or record family only through a versioned contract change with
  cross-language conformance fixtures.

BravePI and YokaKit are the first verified integrations. Neither defines IoTKit's
generic core model.
