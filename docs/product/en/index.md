# IoTKit English documentation

## Concepts

* [Product model](concepts/product-model.md) - Defines the value, responsibilities, and exclusions of IoTKit.
* [Terminology](concepts/terminology.md) - Defines Device, Edge Node, IoTKit Edge, and related terms.

## Architecture

* [System overview](architecture/system-overview.md) - Explains deployment, data flow, and custody.

## Public contracts

* [Ingest contract v1](contracts/ingest-v1.md) - Complete contract for delivering observations from a device to an Edge Node.
* [Input Adapter contract v1](contracts/input-adapter-v1.md) - Complete boundary that separates sensor integration from the core.
* [MQTT Output Adapter contract v1](contracts/mqtt-output-adapter-v1.md) - How the device-local redesign publishes Observations and status to a standard MQTT Broker.
* [v1 compatibility policy](contracts/compatibility-policy-v1.md) - Defines the compatibility promise that begins with product 1.0.0.

## Operations

* [Trial profile](operations/trial-profile.md) - Start a loopback-only sample journey without certificate or Broker design.
* [Installation and recovery](operations/installation-and-recovery.md) - Installation, checks, certificates, backup, and recovery.
* [Storage capacity](operations/storage-capacity.md) - Repeatable SQLite and PostgreSQL capacity regression smoke.
* [Optional OKF provenance metadata](operations/okf-optional-meta.md) - When to add `sources` / `generated` / `verified` (optional; not required).
