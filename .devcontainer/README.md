# Dev Container

This container is for normal IoTKit development and non-hardware tests.

It installs the native dependencies needed by the Rust workspace, including
`libudev-dev`, `pkg-config`, and build tools. It also includes
`mosquitto-clients` for MQTT smoke checks.

## Usage

Open the repository in a Dev Containers compatible editor and choose
`Reopen in Container`.

Inside the container:

```bash
cargo test --workspace -- --test-threads=1
cargo build -p iotkit-rpi-local
```

Real Raspberry Pi hardware tests should still be run on the `iotkit` node:

```bash
cargo test -p rpi-local-adapter --test integration -- --ignored
```
