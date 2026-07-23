# Edge Node

[日本語](README.ja.md)

This tree contains the Rust collection-side product. It reads devices, durably
stores normalized observations, and transfers custody to IoTKit Edge. It does
not own factory semantics, application output, or the Console.

## Start here

- Runtime composition: `apps/node`
- Operator CLI: `apps/nodectl`
- Durable collection domain: `core`
- Envelope/Ack boundaries: `ingest`
- Shared Input Adapter infrastructure: `input`
- Concrete sensor-family integrations: `adapters`
- Hardware-only development utilities: `tools`

For a focused Rust check, run:

```bash
cargo test -p <package-name>
```

Choose an integration path before editing code:

1. A device that already speaks Envelope/Ack uses authenticated HTTP ingest.
2. A sensor IC matching the existing direct-I2C model extends `adapters/rpi-local`.
3. A different protocol or lifecycle becomes a sibling under `adapters`.

The normative boundaries and dependency rules are in the
[architecture map](../docs/okf/en/architecture/system-overview.md) and
[Input Adapter contract](../docs/okf/en/contracts/input-adapter-v1.md).
