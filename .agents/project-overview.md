# Project overview

`iotkit-next` is an on-premises-first IoT data collection foundation rebuilt from
the former IoTKit. It consists of:

- **IoTKit Edge Node:** Rust + Tokio collection software for Raspberry Pi-class
  computers. Sensor-specific behavior stays in Input Adapters.
- **MQTT Broker:** standard infrastructure between Edge Nodes, IoTKit Edge, and
  external applications. IoTKit does not implement its own Broker.
- **IoTKit Edge:** Rust service that accepts durable raw records, applies generic
  meanings, serves the Console, and invokes Output Adapters.

```text
sensor -> Input Adapter -> IoTKit Edge Node -> MQTT Broker
       -> IoTKit Edge -> Output Adapter -> external application

contract-native device -> authenticated HTTP ingest -> IoTKit Edge Node
```

Input Adapters use `iotkit-ingest-client`; they do not depend on `edge-node/core/engine`.
`AdapterEvent` is a frozen engine/supervision vocabulary, not a new adapter API.
The HTTP ingest listener is a separate, default-off path for contract-native
devices. Both paths converge at the Edge Node collector. The complete and
enforced dependency map is in the architecture document and `scripts/check-layers`.

Return to [`AGENTS.md`](../AGENTS.md).
