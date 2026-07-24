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

V1 adapters are trusted Rust crates selected by a private compile-time catalog;
there is no runtime or Console installation. To add a new sibling:

1. Create `edge-node/adapters/<adapter>` and add it to the root
   `Cargo.toml` workspace.
2. Add the crate as a dependency of `edge-node/apps/node/Cargo.toml`.
3. Extend the closed `RawInputAdapterInstance` in
   `edge-node/apps/node/src/config.rs` with the adapter's non-secret top-level
   configuration fields and strict deserialization tests. This central schema
   edit is part of the current compile-time architecture.
4. In `edge-node/apps/node/src/input_adapters.rs`, add one private
   `InputAdapterFactory`, its `parse_and_validate`, `start`, and inventory glue,
   then add the factory to `catalog()`. Provider names stop at this composition
   root and the focused adapter crate.
5. Classify the crate in `scripts/check-layers`, update both architecture maps,
   and add package fixtures plus Edge Node catalog/config tests.
6. Use the provider-neutral, production-shaped `ReferenceAdapter` in
   `iotkit-input-adapter-testkit` as the descriptor/config/start/shutdown
   lifecycle example. It is test-only and never belongs in `catalog()`.

Run from the repository root:

```bash
cargo test -p your-adapter-package
cargo test -p iotkit-input-adapter-testkit
cargo test -p iotkit-edge-node input_adapters
scripts/check-layers
scripts/check-source-layout
```

See the [Input Adapter contract](../../docs/okf/en/contracts/input-adapter-v1.md)
for the required behavior and exact conformance ownership.
