# IoTKit English documentation

## Concepts

* [Product model](concepts/product-model.md) - Defines the value, responsibilities, and exclusions of IoTKit.
* [Terminology](concepts/terminology.md) - Defines Device, Edge Node, IoTKit Edge, and related terms.

## Architecture

* [System overview](architecture/system-overview.md) - Explains deployment, data flow, and custody.

## Public contracts

* [Ingest contract v1](contracts/ingest-v1.md) - Complete contract for delivering observations from a device to an Edge Node.
* [Edge Node custody contract v1](contracts/edge-node-custody-v1.md) - Complete durable-delivery contract from an Edge Node to IoTKit Edge.
* [Input Adapter contract v1](contracts/input-adapter-v1.md) - Complete boundary that separates sensor integration from the core.
* [Output Adapter contract v1](contracts/output-adapter-v1.md) - Complete transformation contract for external applications.

## Operations

* [Installation and recovery](operations/installation-and-recovery.md) - Installation, checks, certificates, backup, and recovery.
* [Storage capacity](operations/storage-capacity.md) - Repeatable SQLite and PostgreSQL capacity regression smoke.
