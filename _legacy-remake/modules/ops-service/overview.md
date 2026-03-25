# ops-service — Domain Overview

## Responsibility
System administration: time synchronization, reboot/shutdown, Node-RED restart, storage usage reporting, camera streaming (MJPG Streamer), and Swagger UI hosting.

## Legacy Source
- その他 tab (92 nodes): system init, time API, exec nodes for OS commands
- .node-red/swagger/ (Express OpenAPI server)

## Key Business Rules
- Time sync: GET/POST UTC time, OS date command
- Reboot: sudo reboot, Shutdown: sudo shutdown -h now
- Storage: df query for /dev/mmcblk0p2
- Camera: MJPG Streamer at port 51890 (/dev/video0)
- Passwordless sudo for user iotkit

## Design Defect D3-3
Privileged OS operations currently run in the same process as business logic. Should be isolated into a separate privilege boundary.

## Dependencies
- (minimal — mostly standalone)
