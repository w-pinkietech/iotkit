# device-config-service — Domain Overview

## Responsibility
Device configuration CRUD, DTO assembly, and read-model rebuild. Manages the relational device catalog in MariaDB and maintains the in-memory projection (flow.devices).

## Legacy Source
- デバイス登録 tab (70 nodes): device registration UI + backend
- Init Config subflow (25 nodes): new device persistence + child-record init
- All Devices subflow (36 nodes): read-model rebuild from relational joins
- Update Sensor / Update Device subflows

## Key Business Rules
- Device registration: select access type → create device + access-config + sensor channels
- 6 access-config tables: ble_device_configs, i2c_device_configs, gpio_device_configs, http_device_configs, mqtt_device_configs, usb_device_configs
- Read-model assembly: join device/sensor/topic/mail/GPIO records into denormalized projection
- Projection refresh on CRUD operations

## Design Defect D3-1
SQL templates currently embedded directly in flow nodes. This module must introduce a repository/data-access abstraction.

## Dependencies
- core-domain (device/sensor types)

## Downstream Consumers
- api-service, ui-web, notification-service, sensor-ingest
