# Output Adapter

Output Adapterは、IoTKitの汎用Observationを外部application向けのexact MQTT
publication 1件へ変換します。Broker接続、secret、storage、sensor rule評価、retryは
担当しません。

最初に[`example/`](example/)を読み、
[`iotkit-output-adapter-api`](api/)へ依存してください。Adapterのtestから共通
conformance suiteを呼び、実装crateをroot workspaceへ追加し、
[`edge/src/composition/output_adapters.rs`](../src/composition/output_adapters.rs)
のstatic registry 1か所へ登録します。それ以外のIoTKit Edge fileへprovider名を
追加しません。

```bash
cargo test -p iotkit-output-adapter-example
cargo test -p YOUR_PACKAGE
cargo test -p iotkit-edge --test output_registry
scripts/test-edge-output.sh
```

Adapterはsandbox pluginではなく、IoTKitと同じ権限で動くtrusted compile-time Rust
codeです。Filesystem、environment、network、secret、thread、clockへアクセスする
sourceや依存を入れません。製品境界の正本は
[Output Adapter v1契約](../../docs/okf/ja/contracts/output-adapter-v1.md)です。
