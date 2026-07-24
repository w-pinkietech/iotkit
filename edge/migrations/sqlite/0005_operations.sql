CREATE TABLE edge_backup_events (
    backup_id TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    destination_name TEXT NOT NULL,
    database_sha256 TEXT NOT NULL CHECK(length(database_sha256) = 64),
    raw_record_count INTEGER NOT NULL CHECK(raw_record_count >= 0)
);

CREATE TABLE edge_backup_cursors (
    backup_id TEXT NOT NULL REFERENCES edge_backup_events(backup_id),
    edge_node_id TEXT NOT NULL,
    ledger_epoch TEXT NOT NULL,
    accepted_through INTEGER NOT NULL CHECK(accepted_through >= 0),
    PRIMARY KEY(backup_id, edge_node_id, ledger_epoch)
);

CREATE TABLE edge_restore_events (
    restore_id TEXT PRIMARY KEY,
    backup_id TEXT NOT NULL,
    restored_at INTEGER NOT NULL CHECK(restored_at >= 0),
    backup_created_at INTEGER NOT NULL CHECK(backup_created_at >= 0),
    backup_edge_id TEXT NOT NULL,
    backup_schema_version INTEGER NOT NULL CHECK(backup_schema_version > 0),
    backup_sha256 TEXT NOT NULL CHECK(length(backup_sha256) = 64)
);

CREATE TABLE edge_restore_cursor_checks (
    restore_id TEXT NOT NULL REFERENCES edge_restore_events(restore_id),
    edge_node_id TEXT NOT NULL,
    ledger_epoch TEXT NOT NULL,
    backup_accepted_through INTEGER NOT NULL CHECK(backup_accepted_through >= 0),
    state TEXT NOT NULL CHECK(state IN ('pending', 'matched', 'recovery_required', 'archive_lost')),
    observed_cursor_start INTEGER,
    updated_at INTEGER NOT NULL CHECK(updated_at >= 0),
    PRIMARY KEY(restore_id, edge_node_id, ledger_epoch)
);

CREATE TABLE edge_storage_samples (
    sampled_at INTEGER PRIMARY KEY,
    database_bytes INTEGER NOT NULL CHECK(database_bytes >= 0),
    raw_record_count INTEGER NOT NULL CHECK(raw_record_count >= 0)
);
