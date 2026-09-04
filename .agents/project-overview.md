# Project overview

`iotkit-next` is an on-premises-first IoT data collection foundation rebuilt from
the former IoTKit. Since the redesign in
[#232](https://github.com/w-pinkietech/iotkit/issues/232) it consists of:

- **IoTKit Edge Node:** Rust + Tokio software for Raspberry Pi-class computers,
  one instance per device. Sensor-specific behavior stays in Input Adapters;
  device-local pipelines turn inputs into Observations; the MQTT Output Adapter
  publishes them under the MQTT Output Adapter contract v1.
- **MQTT Broker:** standard infrastructure between the Edge Node and external
  applications (for example Pinkiet). IoTKit does not implement its own Broker.

```text
sensor -> Input Adapter -> pipeline -> MQTT Output Adapter -> MQTT Broker -> consumer
          |<---------------- IoTKit Edge Node (one per device) ---------------->|
```

The central `iotkit-edge` service, its custody contract, and application-facing
Output Adapters were deleted in #251; the remaining old Edge Node paths (custody
publish, readings, recovery, device ledger, HTTP ingest) are removed in the rest
of [#250](https://github.com/w-pinkietech/iotkit/issues/250).

Input Adapters use `iotkit-ingest-client`; they do not depend on `edge-node/core/engine`.
`AdapterEvent` is a frozen engine/supervision vocabulary, not a new adapter API.
The complete and enforced dependency map is in the architecture document and
`scripts/check-layers`.

Return to [`AGENTS.md`](../AGENTS.md).
