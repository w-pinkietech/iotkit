# Input Adapter

[English](README.md)

このdirectoryの各childは、具体的なacquisition familyを一つ所有します。
Discovery、transport、decode、device identity、ingest Envelopeへの変換が責務です。
Storage、MQTT custody、semantic rule、application payloadは所有しません。

Data path:

```text
Transport -> sensor/protocol Driver -> Input Adapter -> ingest client
```

既存direct-I2C polling modelに合うsensor ICは`rpi-local`へ追加します。
BravePI Mainboard UART protocolは`bravepi-mainboard`を使います。
Discovery、wire protocol、security、lifecycle、identityが異なる場合だけ
新しいsiblingを作ります。

V1 Adapterはprivateなcompile-time catalogが選択するtrusted Rust crateです。
RuntimeまたはConsoleからinstallしません。新しいsiblingを追加する手順:

1. `edge-node/adapters/<adapter>`を作り、root `Cargo.toml` workspaceへ追加する。
2. `edge-node/apps/node/Cargo.toml`からそのcrateへ依存する。
3. `edge-node/apps/node/src/config.rs`のclosedな`RawInputAdapterInstance`へ、
   Adapterの非secret top-level設定fieldとstrict deserialize testを追加する。
   このcentral schema editは現在のcompile-time architectureの一部である。
4. `edge-node/apps/node/src/input_adapters.rs`へprivateな
   `InputAdapterFactory`を一つ追加し、`parse_and_validate`、`start`、inventory glueを
   実装して`catalog()`へ登録する。Provider名はこのcomposition rootと対象Adapter
   crateより内側へ漏らさない。
5. `scripts/check-layers`でcrateを分類し、日英architecture map、package fixture、
   Edge Node catalog/config testを更新する。
6. `iotkit-input-adapter-testkit`のprovider-neutralかつproduction-shapedな
   `ReferenceAdapter`をdescriptor/config/start/shutdown lifecycleの例にする。
   これはtest-onlyであり`catalog()`へ登録しない。

Repository rootから実行する。

```bash
cargo test -p your-adapter-package
cargo test -p iotkit-input-adapter-testkit
cargo test -p iotkit-edge-node input_adapters
scripts/check-layers
scripts/check-source-layout
```

必須behaviorとconformanceの所有範囲は
[Input Adapter契約](../../docs/okf/ja/contracts/input-adapter-v1.md)を参照してください。
