# Edge Node

[English](README.md)

このtreeはRust製の収集側productです。Deviceを読み、正規化した観測値を
耐久保存し、IoTKit Edgeへ保管責任を移します。工場固有の意味付け、
application出力、Consoleは所有しません。

## 最初に見る場所

- Runtime構成: `apps/node`
- Operator CLI: `apps/nodectl`
- 耐久収集domain: `core`
- Envelope/Ack境界: `ingest`
- 共有Input Adapter基盤: `input`
- 具体的なsensor family統合: `adapters`
- 実機開発専用tool: `tools`

対象packageだけを検査する場合:

```bash
cargo test -p <package-name>
```

Codeを編集する前に接続方法を選びます。

1. Envelope/Ackを直接送れるdeviceは認証付きHTTP ingestを使う。
2. 既存direct-I2C modelに合うsensor ICは`adapters/rpi-local`へ追加する。
3. Protocolやlifecycleが異なる場合は`adapters`配下にsiblingを作る。

正本の境界と依存ruleは[Architecture](../docs/product/ja/architecture/system-overview.md)と
[Input Adapter契約](../docs/product/ja/contracts/input-adapter-v1.md)を参照してください。
