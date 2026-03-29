CREATE TABLE sensor_readings (
    adapter_id  TEXT    NOT NULL,
    device_key  TEXT    NOT NULL,
    ingested_at INTEGER NOT NULL,
    sensor_type TEXT    NOT NULL,
    values_json TEXT    NOT NULL,
    rssi        INTEGER,
    battery_pct INTEGER,
    PRIMARY KEY (adapter_id, device_key, ingested_at, sensor_type)
) WITHOUT ROWID;
