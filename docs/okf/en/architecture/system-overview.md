---
type: Architecture
title: "IoTKit system overview"
description: "Explains IoTKit deployment, input paths, custody, semantic mapping, and external output."
language: en
translation_key: architecture.system-overview
status: stable
revision: 1
---

# IoTKit system overview

```text
vendor-specific device -> Input Adapter --+
contract-native device -> HTTPS ingest ----+-> IoTKit Edge Node
  -> internal MQTT Broker
  -> IoTKit Edge
  -> generic semantic Observation
  -> Output Adapter
  -> external MQTT Broker
  -> external application
```

An Input Adapter owns device communication and translation from a vendor-specific format. A capable device can instead implement authenticated HTTPS ingest directly. Both paths converge at the same Edge Node collector, and the receiver determines the authenticated sender.

The Edge Node stores an observation and its pending publication state in one SQLite transaction. Pre-activation observations stay local and are never replayed into the custody stream. Quarantined observations have no publication state while quarantined; after release, only records admitted through the activation and publication gate can be delivered.

An MQTT PUBACK proves transport receipt by the Broker only. Custody transfers when IoTKit Edge commits the raw record and contiguous cursor to the selected authoritative database and returns the matching `accepted-through`. Only then may the Edge Node make the covered data purge-eligible.

IoTKit Edge does not mutate stored raw data. A separate stage maps it to generic meaning configured by the operator. An Output Adapter creates the application-specific topic and payload from that Observation. Failure of external output does not block raw custody.

Each IoTKit Edge installation selects exactly one authoritative database: `embedded` (SQLite) or `postgres` (PostgreSQL). Both implement the same product contract and are used within a measured capacity envelope. IoTKit neither dual-writes nor silently falls back to an empty backend.

Related contracts: [Ingest](../contracts/ingest-v1.md), [Custody](../contracts/edge-node-custody-v1.md), [Input Adapter](../contracts/input-adapter-v1.md), and [Output Adapter](../contracts/output-adapter-v1.md).
