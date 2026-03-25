# api-service — Domain Overview

## Responsibility
HTTP API layer: request validation, routing, response serialization. Exposes device CRUD, sensor type CRUD, sensor value read/write, output control, sensor log queries, and system admin endpoints.

## Legacy Source
- PI・JIG・I2C・GPIO tab (HTTP-in nodes)
- .node-red/swagger/ (Express-based OpenAPI server)
- Function nodes: リクエスト検証 (a55a48e2, ec7d3c84)

## Key Contracts
- GET/POST/DELETE /api/v2/device
- GET/POST/DELETE /api/v2/sensor
- GET/POST /api/v2/time
- POST /api/v2/reboot, /api/v2/shutdown

## Dependencies
- core-domain (types)
- device-config-service (CRUD operations)
- timeseries-service (log queries)
- device-command-orchestrator (output commands)
