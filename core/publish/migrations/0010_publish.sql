-- 出口 publication log(outbox)。pub_seq は readings.seq と別採番(D7決定4)
CREATE TABLE publication_log (
    pub_seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    epoch           TEXT    NOT NULL,
    kind            TEXT    NOT NULL,              -- measurement | annotation | commissioning_smoke
    subtype         TEXT,                          -- annotation: epoch_start。その他は NULL
    reading_seq     INTEGER,                       -- measurement: readings.seq 参照。その他は NULL
    annotation_json TEXT,                          -- annotation / commissioning_smoke inline payload
    created_at      INTEGER NOT NULL
);
-- epoch_start の二重 enqueue を DB 制約で排除(spec §5.2/§8 冪等)
CREATE UNIQUE INDEX ux_publog_annotation_epoch
    ON publication_log(epoch, subtype) WHERE kind = 'annotation';
-- retention/push の batch/prune 用
CREATE INDEX ix_publog_epoch_seq ON publication_log(epoch, pub_seq);
CREATE INDEX ix_publog_reading   ON publication_log(reading_seq) WHERE reading_seq IS NOT NULL;

-- 出口配送先。MVE は1行のみ運用(spec §4.2)
CREATE TABLE target_registry (
    target_id           TEXT PRIMARY KEY,
    endpoint_url        TEXT NOT NULL,             -- https:// のみ(§11で強制)
    credential_token    TEXT NOT NULL,             -- per-target bearer。秘密(snapshot 非含有)
    archive_responsible INTEGER NOT NULL DEFAULT 0,
    schema_version      INTEGER NOT NULL,
    cursor_epoch        TEXT,                      -- 最後に ack された epoch(初期 NULL)
    cursor_pub_seq      INTEGER NOT NULL DEFAULT 0,
    created_at          INTEGER NOT NULL
);
