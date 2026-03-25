# ui-web — Domain Overview

## Responsibility
Operator-facing web UI: dashboard with realtime charts, device registration forms, router/module/transmitter command screens, settings management, sensor log viewer.

## Legacy Source
- ダッシュボード tab (85 nodes, 48 UI nodes): realtime visualization
- デバイス登録 tab (UI parts): device management forms
- static/ directory: CSS, JS (RealtimeLineChart.js, RealtimeGpioChart.js, RealtimeHeatmap.js)
- Tab Transition subflow (screen navigation across 7 tabs)

## Key Business Rules
- 8+ operator screens with forms, tables, realtime charts
- AngularJS dashboard framework (legacy)
- Screen-state gating tied to device command busy state
- Type2Config subflow for DTO enrichment in display

## Design Defect D3-2
Provider-specific details (BravePI/BraveJIG) currently leak into UI templates. Should consume capability-neutral DTOs from device-config-service.

## Dependencies
- core-domain, device-config-service, timeseries-service, device-command-orchestrator
