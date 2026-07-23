-- D5: デバイス台帳(R7)+series台帳実体化+目撃ステージング+監査+台帳メタ
CREATE TABLE devices (
    system_id            BLOB PRIMARY KEY,          -- UUIDv7 16bytes(D5決定3)
    hardware_id          TEXT NOT NULL,
    user_label           TEXT,
    parent_system_id     BLOB REFERENCES devices(system_id),
    kind                 TEXT NOT NULL CHECK (kind IN ('individual','positional')),
    state                TEXT NOT NULL CHECK (state IN ('quarantined','active','retired')),
    declaration_version  INTEGER,
    superseded_by        BLOB REFERENCES devices(system_id),
    created_at           INTEGER NOT NULL,
    retired_at           INTEGER
);
-- 生きたエントリ間でのみhardware_id一意(D5決定1: retiredは除外)
CREATE UNIQUE INDEX idx_devices_hardware_alive
    ON devices(hardware_id) WHERE state != 'retired';

CREATE TABLE series (
    series_id        INTEGER PRIMARY KEY AUTOINCREMENT,  -- 単調・再利用なし(D5決定3)
    system_id        BLOB NOT NULL REFERENCES devices(system_id),
    measurement_key  TEXT NOT NULL,
    channel_index    INTEGER NOT NULL DEFAULT -1,        -- 'na'は番兵値-1(D5決定3)
    variant          TEXT NOT NULL DEFAULT 'primary',
    quarantined      INTEGER NOT NULL DEFAULT 0,
    value_semantics  TEXT NOT NULL DEFAULT 'calibrated', -- raw_legacy|calibrated(D5)
    unit             TEXT,
    range_min        REAL,
    range_max        REAL,
    legacy_sensor_type INTEGER,
    created_at       INTEGER NOT NULL,
    UNIQUE (system_id, measurement_key, channel_index, variant)
);

-- 目撃ステージング: 有界・パージ可能(D5決定4経路A)
CREATE TABLE sightings (
    hardware_id  TEXT PRIMARY KEY,
    source       TEXT NOT NULL,
    first_seen   INTEGER NOT NULL,
    last_seen    INTEGER NOT NULL,
    observations INTEGER NOT NULL DEFAULT 1
);

-- append-only監査(R13の最小下地)
CREATE TABLE ledger_events (
    event_id   INTEGER PRIMARY KEY AUTOINCREMENT,
    at         INTEGER NOT NULL,
    kind       TEXT NOT NULL,
    system_id  BLOB,
    detail     TEXT NOT NULL
);

CREATE TABLE ledger_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
