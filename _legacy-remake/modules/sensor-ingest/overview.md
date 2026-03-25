# sensor-ingest — Domain Overview

## Responsibility
Sensor data collection and normalization: I2C polling (6 sensor types), GPIO input monitoring (5 pins), BravePI wireless reception, and BraveJIG serial/MQTT reception. Normalizes all inputs into a common sensor data event.

## Legacy Source
- PI・JIG・I2C・GPIO tab (~200 nodes: decode, I2C, GPIO sections)
- .node-red/python/ (7 I2C sensor scripts)
- Function nodes: JSONデコード (BravePI: 53016c6c, 193 lines), 登録済みデータ (f05d5101, 107 lines)

## Key Business Rules
- I2C sensors: OPT3001, MCP9600, VL53L1X, LIS2DUXS12, MCP3427, SDP810
- GPIO inputs: BCM 18/23/25/5/16, 25ms debounce, sensor type 257
- BravePI: /dev/ttyAMA0 at 38400 baud, binary frame decode
- Remote sensor TTL: 2 hours for eviction

## Dependencies
- core-domain (device/sensor types)
- provider-adapter (protocol codecs)
- device-config-service (read-model updates)
- timeseries-service (measurement writes)
- notification-service (threshold events)
