-- D6: 現場レジストリ(受理判定R8の唯一の参照先)。copy-on-enable+エントリrevision(決定4)
CREATE TABLE registry_entries (
    measurement_key   TEXT PRIMARY KEY,
    origin            TEXT NOT NULL CHECK (origin IN ('catalog','custom')),
    catalog_version   TEXT,             -- origin='catalog'のみ(エントリ単位スタンプ=決定4)
    entry_revision    TEXT NOT NULL,    -- 内容ハッシュ(決定4)
    unit_ucum         TEXT,
    unit_display      TEXT,
    value_type        TEXT NOT NULL CHECK (value_type IN ('float','int','bool','record')),
    semantic_class    TEXT NOT NULL,
    channel_mode      TEXT NOT NULL CHECK (channel_mode IN ('single','generic','fixed')),
    channel_roles_json TEXT,            -- fixedのみ(JSON配列)
    physical_min      REAL,             -- カタログ物理限界(外殻=決定7)
    physical_max      REAL,
    site_min          REAL,             -- 現場既定(外殻内)。Wave 0では設定APIなし(R14=Wave 1)
    site_max          REAL,
    enabled_at        INTEGER NOT NULL
);

-- D6決定3: エイリアス表(alias → measurement_key、多:1可)。キーと単一名前空間(決定2、
-- 相互衝突はアプリ層がenable/define時に同一トランザクション内で検査する)
CREATE TABLE registry_aliases (
    alias            TEXT PRIMARY KEY,
    measurement_key  TEXT NOT NULL REFERENCES registry_entries(measurement_key),
    alias_kind       TEXT NOT NULL CHECK (alias_kind IN ('rename','site_mapping')),
    created_at       INTEGER NOT NULL
);

-- D6決定3/11: legacy_sensor_type移行シム(ワイヤエイリアスではない型付き対応表)。
-- 播種はレガシー移行(D2 Phase 3.5)時のみ。Wave 0のEdge起動では触らない。
CREATE TABLE legacy_sensor_type_map (
    sensor_type      INTEGER PRIMARY KEY,
    measurement_key  TEXT NOT NULL,
    created_at       INTEGER NOT NULL
);
