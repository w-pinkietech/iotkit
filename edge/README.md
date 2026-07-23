# IoTKit Edge

[日本語](README.ja.md)

This Go component durably accepts Edge Node records, advances custody cursors,
stores raw and semantic observations, exports through Output Adapters, and
serves the authenticated Console. It does not read Edge Node databases or
import Rust internals.

The filesystem path is now `edge/`. The Go module identity intentionally remains
`github.com/w-pinkietech/iotkit-next/iotkit-edge` for compatibility; do not treat
that difference as an incomplete rename.

## Start here

- Process entry point: `cmd/iotkit-edge`
- Application operations: `internal/edgeapp`
- MQTT custody boundary: `internal/mqttedge`
- Durable stores: `internal/store`
- Output Adapter boundary: `internal/outputadapter`
- Console HTTP/SSR: `internal/edgehttp`
- Browser behavior: `frontend/src`
- Browser JSON schema: `openapi/edge-console-v1.yaml`

Run focused tests from this directory:

```bash
go test ./internal/<package>
npm run check --prefix frontend
```

Canonical behavior is documented in the
[architecture map](../docs/okf/en/architecture/system-overview.md),
[Edge Node custody contract](../docs/okf/en/contracts/edge-node-custody-v1.md),
and [Output Adapter contract](../docs/okf/en/contracts/output-adapter-v1.md).
