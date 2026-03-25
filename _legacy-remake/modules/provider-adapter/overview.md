# provider-adapter — Domain Overview

## Responsibility
Protocol abstraction layer encapsulating BravePI and BraveJIG serial communication. Translates provider-specific binary protocols into capability-neutral commands and events.

## Legacy Source
- PI・JIG・I2C・GPIO tab (serial codec nodes)
- Function nodes: JSONデコード (BraveJIG: 0e8090cf, 758 lines — accumulate frames, decode by response family, 15 output branches)
- BravePI Output subflow (command payload shaping)

## Key Business Rules
- BravePI: binary frame accumulation, sensor payload decode with device/type/RSSI/flags
- BraveJIG: serial frame accumulation by destination, incremental decode, 15 response families
- Capability commands: ReadParameters, SetOutputState, DFU operations

## Design Defect D3-2
Protocol details currently leak into UI templates and state management. This module must encapsulate all provider-specific logic behind a clean adapter interface.

## Dependencies
- core-domain (device types)

## Downstream Consumers
- sensor-ingest (decoded sensor events)
- device-command-orchestrator (command encoding/decoding)
