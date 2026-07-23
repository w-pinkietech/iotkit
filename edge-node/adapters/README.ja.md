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

対象Adapterのpackage testと共有conformance testkitを実行してください。
必須behaviorは[Input Adapter契約](../../docs/okf/ja/contracts/input-adapter-v1.md)
を参照してください。
