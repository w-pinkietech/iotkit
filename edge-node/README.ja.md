# Edge Node

[English](README.md)

このtreeが製品そのもので、端末1台につきRustバイナリ1つで動きます。Input Adapterで
センサーを読み、端末内のpipelineでObservationへ変換し、MQTT Output Adapter契約 v1で
標準のMQTT Brokerへ公開します。業務上の意味（製品、工程、OEE、alarm）はBrokerを
購読するapplication側に置きます。

## 最初に見る場所

- Runtime構成: `apps/node`
- Operator CLI: `apps/nodectl`
- 端末内のdomain（`pipeline`、`collector`、`ops`、`storage`など）: `core`
- Input Adapterとcollectorの間のEnvelope/Ack境界: `ingest`
- 共有Input Adapter基盤: `input`
- 具体的なsensor family統合: `adapters`
- 実機開発専用tool: `tools`

対象packageだけを検査する場合:

```bash
cargo test -p <package-name>
```

Codeを編集する前に接続方法を選びます。

1. 既存direct-I2C modelに合うsensor ICは`adapters/rpi-local`へ追加する。
2. Protocolやlifecycleが異なる場合は`adapters`配下にsiblingを作る。

正本の境界と依存ruleは[Architecture](../docs/product/ja/architecture/system-overview.md)と
[Input Adapter契約](../docs/product/ja/contracts/input-adapter-v1.md)を参照してください。
