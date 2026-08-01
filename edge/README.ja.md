# IoTKit Edge

[English](README.md)

このRust serviceはEdge Node recordを耐久受理し、custody cursorを進め、
raw/semantic observationを保存し、Output Adapterから出力し、認証付きConsoleを
提供します。Edge NodeのDBは読みません。

## 最初に見る場所

- Process entry point・CLI: `src/main.rs`、`src/cli/`
- Application operation: `src/application/`
- MQTT custody・外部出力: `src/mqtt/`
- Backend別の耐久store: `src/storage/`
- 汎用semantic: `src/semantics/`
- Output Adapter開発境界: `output-adapters/`
- Console HTTP/SSR: `src/web/`
- Browser behavior: `frontend/src/`
- Browser JSON schema: `openapi/edge-console-v1.yaml`
- Backup・diagnostics: `src/backup/`、`src/diagnostics/`

repository rootから対象testを実行します。

```bash
cargo test -p iotkit-edge --test <contract-test>
cargo test -p iotkit-output-adapter-testkit
npm run check --prefix edge/frontend
```

Rust schemaは新規baselineです。以前のGo実装が作成したDBと暗号化backup artifactは
意図的に非対応です。

正本のbehaviorは[Architecture](../docs/product/ja/architecture/system-overview.md)、
[Edge Node保管責任契約](../docs/product/ja/contracts/edge-node-custody-v1.md)、
[Output Adapter契約](../docs/product/ja/contracts/output-adapter-v1.md)を参照してください。
