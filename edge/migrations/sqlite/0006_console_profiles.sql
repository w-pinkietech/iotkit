CREATE TABLE inventory_devices (
    device_ref TEXT PRIMARY KEY,
    edge_node_id TEXT NOT NULL,
    system_id TEXT NOT NULL,
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    UNIQUE(edge_node_id, system_id)
);

CREATE TABLE inventory_signals (
    signal_ref TEXT PRIMARY KEY,
    edge_node_id TEXT NOT NULL,
    series_key TEXT NOT NULL,
    system_id TEXT NOT NULL,
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    UNIQUE(edge_node_id, series_key)
);

INSERT INTO inventory_devices(device_ref,edge_node_id,system_id,created_at)
SELECT 'dev_' || lower(hex(randomblob(16))),edge_node_id,system_id,updated_at
FROM descriptor_devices;

INSERT INTO inventory_signals(signal_ref,edge_node_id,series_key,system_id,created_at)
SELECT 'sig_' || lower(hex(randomblob(16))),edge_node_id,series_key,system_id,updated_at
FROM descriptor_signals;

CREATE TABLE device_profiles (
    edge_node_id TEXT NOT NULL,
    system_id TEXT NOT NULL,
    display_name TEXT NOT NULL CHECK(length(display_name) BETWEEN 1 AND 128),
    location TEXT NOT NULL CHECK(length(location) BETWEEN 1 AND 256),
    revision INTEGER NOT NULL CHECK(revision > 0),
    updated_at INTEGER NOT NULL CHECK(updated_at >= 0),
    PRIMARY KEY(edge_node_id,system_id)
);

CREATE TABLE signal_profiles (
    edge_node_id TEXT NOT NULL,
    series_key TEXT NOT NULL,
    display_name TEXT NOT NULL CHECK(length(display_name) BETWEEN 1 AND 128),
    display_sensor_type TEXT NOT NULL,
    display_sensor_type_label TEXT NOT NULL,
    display_value_kind TEXT NOT NULL CHECK(display_value_kind IN ('numeric','boolean')),
    display_unit_mode TEXT NOT NULL CHECK(display_unit_mode IN ('unit','dimensionless')),
    display_unit TEXT NOT NULL CHECK(length(display_unit) <= 32),
    decimal_places INTEGER NOT NULL CHECK(decimal_places BETWEEN 0 AND 6),
    revision INTEGER NOT NULL CHECK(revision > 0),
    updated_at INTEGER NOT NULL CHECK(updated_at >= 0),
    PRIMARY KEY(edge_node_id,series_key)
);
