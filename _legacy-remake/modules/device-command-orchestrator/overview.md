# device-command-orchestrator — Domain Overview

## Responsibility
Command lifecycle management: send commands to devices, track busy state, handle ACK/timeout/retry. Covers GPIO output, BravePI output, BraveJIG router commands, and BraveJIG module commands.

## Legacy Source
- ルーター tab (95 nodes): router pairing, scan mode, DFU
- モジュール tab (64 nodes): module parameter request/set, reset, DFU
- BLEトランスミッター tab (81 nodes): BravePI polling/config orchestration
- Function nodes: BravePI接点出力制御 (473e2d71), BraveJIG接点出力制御 (07bad945), 設定受信 (62cec941)

## Key Business Rules
- State machine: Idle → Active (set busy) → Success (clear on ACK) or Timeout (10s, retry)
- flow.busy tracks command family (parameter-request, parameter-setting, dfu-start, etc.)
- Queue-based execution for BraveJIG (one command in flight per queue)
- Multi-chunk DFU state orchestration for firmware updates

## Dependencies
- core-domain (device types)
- provider-adapter (command encoding/decoding)

## Downstream Consumers
- api-service (output control endpoints)
- ui-web (command state display)
