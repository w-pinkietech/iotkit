# notification-service — Domain Overview

## Responsibility
Threshold detection and notification dispatch via MQTT and email. Manages notification destination configuration (brokers, topics, SMTP servers, recipients).

## Legacy Source
- 設定 tab (partial: MQTT/mail CRUD sections)
- Count Up route (threshold crossing detection)
- sensor_mqtt_pivots / sensor_mail_pivots (routing tables)

## Key Business Rules
- Edge detection: payload.toggle (no change) vs payload.signal (threshold crossed)
- MQTT publish to configured topics via localhost:51883
- Email via configured SMTP server
- Pivot tables: 1:N between sensors and notification destinations

## Dependencies
- core-domain (sensor types)
- device-config-service (sensor-to-destination mappings)

## Downstream Consumers
- (terminal — dispatches to external systems)
