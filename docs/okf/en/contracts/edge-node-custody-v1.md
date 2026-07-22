---
type: Contract Overview
title: "Edge Node custody contract v1 overview"
description: "Entry point for the MQTT contract that transfers custody of raw records from an Edge Node to IoTKit Edge."
language: en
translation_key: contracts.edge-node-custody-v1
status: stable
revision: 1
---

# Edge Node custody contract v1 overview

This document is an orientation guide to the boundary. In the same Git revision, `docs/exit-contract.md`, shared egress fixtures, and matching Rust and Go validators form the detailed contract artifact set for record schemas, topics, cursors, acknowledgements, and validation order.

This contract delivers raw records held by an Edge Node to IoTKit Edge through a standard MQTT Broker with at-least-once semantics and explicitly transfers responsibility for durable preservation. It is not the contract consumed by external applications such as YokaKit.

Each Edge Node incarnation is fenced by `edge_node_id` and ledger epoch. Publishable records receive monotonically increasing publication sequences, with global identity `(edge_node_id, ledger_epoch, pub_seq)`. IoTKit Edge accepts only an exact replay with the same content fingerprint idempotently. Different content under the same identity is a custody conflict: IoTKit Edge rejects the whole batch and returns no `accepted-through`.

Broker enrollment grants transport access through credentials and ACLs only; it does not authorize the custody stream. Activation has three stages. A Console operation durably enqueues a request for the exact `(edge_node_id, ledger_epoch)` in IoTKit Edge. The Edge Node validates and durably applies that request, fixes the boundary, and only then opens publication admission for future records. IoTKit Edge stores and acknowledges records only after it commits the matching result and marks the incarnation active.

IoTKit Edge validates the whole batch and commits both raw records and the contiguous cursor in one database transaction. Only after that commit does it return an application-level `accepted-through` matching the incarnation and batch bound. MQTT PUBACK, message arrival, and the start of validation do not transfer custody.

The Edge Node retains originals beyond `accepted-through`. Normal retention may delete only the range made purge-eligible by the contract. Deleting an unacknowledged original is the last explicit data-loss stage and must never occur without an audit record and gap annotation.

Pre-activation observations have no publication sequence and are never replayed later. Quarantined records also have no outbox state while quarantined. If released later, a record can be delivered only after passing the current activation and publication-admission gate in a durable transaction.
