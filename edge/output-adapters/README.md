# Output Adapters

Output Adapters turn an IoTKit generic Observation into one exact MQTT
publication for an external application. They do not connect to a Broker, read
secrets, access storage, evaluate sensor rules, or own retry.

Start with [`example/`](example/) and depend on
[`iotkit-output-adapter-api`](api/). Run the shared conformance suite from the
Adapter's tests, then add the implementation crate to the root workspace and
register one static instance in
[`edge/src/composition/output_adapters.rs`](../src/composition/output_adapters.rs).
No other IoTKit Edge file should name the provider.

```bash
cargo test -p iotkit-output-adapter-example
cargo test -p YOUR_PACKAGE
cargo test -p iotkit-edge --test output_registry
scripts/test-edge-output.sh
```

Adapters are trusted compile-time Rust code, not sandboxed plugins. Adapter
source and dependencies must not use filesystem, environment, network, secret,
thread, or clock access. The normative product boundary is the
[Output Adapter v1 contract](../../docs/okf/en/contracts/output-adapter-v1.md).
