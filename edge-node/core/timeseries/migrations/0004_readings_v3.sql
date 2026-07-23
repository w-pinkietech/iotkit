-- D1フェーズ1.5: series_id FK・挿入順単調seq・時刻を一意性に使わない(旧v2の同一ms暗黙dedupはD1と矛盾)
CREATE TABLE readings (
    seq            INTEGER PRIMARY KEY AUTOINCREMENT,  -- 内部挿入順の単調seq。出口IDはpublication_log.pub_seq(D7決定4、readings.seqは出さない)
    series_id      INTEGER NOT NULL REFERENCES series(series_id),
    received_at    INTEGER NOT NULL,                   -- コレクタが必ず付与(D1)
    device_time    INTEGER,                            -- デバイス申告時刻(任意)
    time_source    TEXT NOT NULL,
    time_quality   TEXT NOT NULL DEFAULT 'unsynced',   -- R18受信側刻印。Wave 0は既定値固定(D3境界の明文化・
                                                       -- 外部レビュー第2回反映。NTP状態評価はWave 1、列だけ初日から)
    values_json    TEXT NOT NULL,
    rssi           INTEGER,
    battery_pct    INTEGER,
    quarantined    INTEGER NOT NULL DEFAULT 0          -- 値域外・未知キー等の行レベル検疫(D1/D6)
);
CREATE INDEX idx_readings_series_time ON readings(series_id, received_at);

-- D1: dedupキー=(認証済み送信者, envelope_id)。TTL+サイズ上限で有界
CREATE TABLE ingest_dedup (
    sender_id    TEXT NOT NULL,
    envelope_id  TEXT NOT NULL,
    received_at  INTEGER NOT NULL,
    PRIMARY KEY (sender_id, envelope_id)
);
CREATE INDEX idx_ingest_dedup_time ON ingest_dedup(received_at);

-- D5経路A: 目撃ステージング中のデータ保持(有界・パージ可能。承認時に本流化)
CREATE TABLE staged_readings (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    hardware_id  TEXT NOT NULL,
    received_at  INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX idx_staged_hw ON staged_readings(hardware_id, id);
