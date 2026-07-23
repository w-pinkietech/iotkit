# Input Adapters

[日本語](README.ja.md)

Each directory here owns one concrete acquisition family: its discovery,
transport, decoding, device identity, and mapping into ingest Envelopes. An
adapter does not own storage, MQTT custody, semantic rules, or application
payloads.

The data path is:

```text
Transport -> sensor/protocol Driver -> Input Adapter -> ingest client
```

Use `rpi-local` for a sensor IC that fits the existing direct-I2C polling model.
Use `bravepi-mainboard` for the BravePI Mainboard UART protocol. Create a new
sibling only when discovery, wire protocol, security, lifecycle, or identity
really differs.

Run the adapter package tests and the shared conformance testkit. See the
[Input Adapter contract](../../docs/okf/en/contracts/input-adapter-v1.md) for
the required behavior.
