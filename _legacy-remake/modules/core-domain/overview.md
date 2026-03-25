# core-domain — Domain Overview

## Responsibility
Foundation types for the entire system: device aggregate model, access-config union types (BLE, I2C, GPIO, USB, HTTP, MQTT), sensor type definitions, and projection/read-model shape.

## Legacy Source
- Type2Config subflow (392-line function node enriching device DTOs)
- All Devices subflow (36 nodes rebuilding read-model from relational joins)
- flow.devices / global.sensorTypes / global.sensors cache shapes

## Key Business Rules
- 6 access types with distinct config schemas
- Sensor type range: built-in + custom external (70000-70009)
- Device projection merges config + telemetry + display values

## Dependencies
None (foundation module).

## Downstream Consumers
All other modules depend on core-domain types.
