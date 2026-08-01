# IoTKit Edge

[日本語](README.ja.md)

This Rust service durably accepts Edge Node records, advances custody cursors,
stores raw and semantic observations, exports through Output Adapters, and
serves the authenticated Console. It does not read Edge Node databases.

## Start here

- Process entry point and CLI: `src/main.rs`, `src/cli/`
- Application operations: `src/application/`
- MQTT custody and output: `src/mqtt/`
- Durable backend-specific stores: `src/storage/`
- Generic semantics: `src/semantics/`
- Output Adapter authoring boundary: `output-adapters/`
- Console HTTP/SSR: `src/web/`
- Browser behavior: `frontend/src/`
- Browser JSON schema: `openapi/edge-console-v1.yaml`
- Backup and diagnostics: `src/backup/`, `src/diagnostics/`

Run focused tests from the repository root:

```bash
cargo test -p iotkit-edge --test <contract-test>
cargo test -p iotkit-output-adapter-testkit
npm run check --prefix edge/frontend
```

The Rust schema is a fresh baseline. Databases and encrypted backup artifacts
created by the former Go implementation are intentionally unsupported.

Canonical behavior is documented in the
[architecture map](../docs/product/en/architecture/system-overview.md),
[Edge Node custody contract](../docs/product/en/contracts/edge-node-custody-v1.md),
and [Output Adapter contract](../docs/product/en/contracts/output-adapter-v1.md).
