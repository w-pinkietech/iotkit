-- Plan 6 Task 6: principal-owned, byte-accounted, pinnable unknown-subject staging.
ALTER TABLE staged_readings ADD COLUMN principal_id TEXT NOT NULL DEFAULT 'legacy:unknown';
ALTER TABLE staged_readings ADD COLUMN payload_bytes INTEGER NOT NULL DEFAULT 0 CHECK(payload_bytes >= 0);
ALTER TABLE staged_readings ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0, 1));
UPDATE staged_readings SET payload_bytes = length(CAST(payload_json AS BLOB));
CREATE INDEX idx_staged_principal_age
    ON staged_readings(principal_id, received_at, id);
CREATE INDEX idx_staged_global_age
    ON staged_readings(received_at, id);

CREATE TABLE ingest_dedup_maintenance (
    id                 INTEGER PRIMARY KEY CHECK(id = 1),
    degraded           INTEGER NOT NULL DEFAULT 0 CHECK(degraded IN (0, 1)),
    episode_started_at INTEGER,
    last_failure_at    INTEGER,
    last_success_at    INTEGER
);
INSERT INTO ingest_dedup_maintenance(id) VALUES(1);
