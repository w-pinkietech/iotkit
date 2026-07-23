# IoTKit Edge

[English](README.md)

このGo componentはEdge Node recordを耐久受理し、custody cursorを進め、
raw/semantic observationを保存し、Output Adapterから出力し、認証付きConsoleを
提供します。Edge NodeのDBを読まず、Rust内部実装にも依存しません。

Filesystem上のpathは`edge/`です。Go module identityの
`github.com/w-pinkietech/iotkit-next/iotkit-edge`は互換性のため意図的に維持します。
移動漏れとして変更しないでください。

## 最初に見る場所

- Process entry point: `cmd/iotkit-edge`
- Application operation: `internal/edgeapp`
- MQTT custody境界: `internal/mqttedge`
- 耐久store: `internal/store`
- Output Adapter境界: `internal/outputadapter`
- Console HTTP/SSR: `internal/edgehttp`
- Browser behavior: `frontend/src`
- Browser JSON schema: `openapi/edge-console-v1.yaml`

このdirectoryから対象testを実行します。

```bash
go test ./internal/<package>
npm run check --prefix frontend
```

正本のbehaviorは[Architecture](../docs/okf/ja/architecture/system-overview.md)、
[Edge Node保管責任契約](../docs/okf/ja/contracts/edge-node-custody-v1.md)、
[Output Adapter契約](../docs/okf/ja/contracts/output-adapter-v1.md)を参照してください。
