# Edge Node

[日本語](README.ja.md)

This tree is the product: one Rust binary per device. It reads sensors through
Input Adapters, turns the readings into Observations with device-local
pipelines, and publishes them to a standard MQTT Broker under the MQTT Output
Adapter contract v1. Business meaning (products, processes, OEE, alarms) stays in
the applications that subscribe to the Broker.

## Start here

- Runtime composition: `apps/node`
- Operator CLI: `apps/nodectl`
- Device-local domain (`pipeline`, `collector`, `ops`, `storage`, ...): `core`
- Envelope/Ack boundary between Input Adapters and the collector: `ingest`
- Shared Input Adapter infrastructure: `input`
- Concrete sensor-family integrations: `adapters`
- Hardware-only development utilities: `tools`

For a focused Rust check, run:

```bash
cargo test -p <package-name>
```

Choose an integration path before editing code:

1. A sensor IC matching the existing direct-I2C model extends `adapters/rpi-local`.
2. A different protocol or lifecycle becomes a sibling under `adapters`.

The normative boundaries and dependency rules are in the
[architecture map](../docs/product/en/architecture/system-overview.md) and
[Input Adapter contract](../docs/product/en/contracts/input-adapter-v1.md).
