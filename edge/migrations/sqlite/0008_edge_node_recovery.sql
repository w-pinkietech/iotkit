CREATE TABLE edge_node_recovery_cases (
    recovery_id TEXT PRIMARY KEY,
    state TEXT NOT NULL CHECK(state IN ('prepared', 'authorized', 'completed', 'recovery_hold')),
    edge_node_id TEXT NOT NULL,
    backup_id TEXT NOT NULL,
    old_ledger_epoch TEXT NOT NULL,
    new_ledger_epoch TEXT NOT NULL,
    broker_fence_id TEXT NOT NULL,
    broker_credential_generation INTEGER NOT NULL CHECK(broker_credential_generation > 0),
    backup_created_at INTEGER NOT NULL CHECK(backup_created_at >= 0),
    broker_fenced_at INTEGER NOT NULL CHECK(broker_fenced_at >= 0),
    device_auth_generation INTEGER CHECK(device_auth_generation >= 0),
    candidate_instance_id TEXT,
    snapshot_accepted_through INTEGER NOT NULL CHECK(snapshot_accepted_through >= 0),
    snapshot_allocation_high_water INTEGER NOT NULL CHECK(snapshot_allocation_high_water >= snapshot_accepted_through),
    snapshot_epoch_start_publication_seq INTEGER
        CHECK(snapshot_epoch_start_publication_seq IS NULL OR
              snapshot_epoch_start_publication_seq BETWEEN 1 AND snapshot_allocation_high_water),
    edge_accepted_through INTEGER NOT NULL CHECK(edge_accepted_through >= snapshot_accepted_through),
    request_json BLOB CHECK(request_json IS NULL OR json_valid(request_json)),
    result_json BLOB CHECK(result_json IS NULL OR json_valid(result_json)),
    completion_json BLOB CHECK(completion_json IS NULL OR json_valid(completion_json)),
    replayed_records INTEGER CHECK(replayed_records >= 0),
    last_new_publication_seq INTEGER CHECK(last_new_publication_seq >= 1),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    updated_at INTEGER NOT NULL CHECK(updated_at >= created_at),
    completed_at INTEGER,
    UNIQUE(edge_node_id, backup_id),
    UNIQUE(edge_node_id, new_ledger_epoch)
);

CREATE UNIQUE INDEX one_open_recovery_per_edge_node
ON edge_node_recovery_cases(edge_node_id)
WHERE state IN ('prepared', 'authorized');

CREATE TABLE recovery_command_outbox (
    recovery_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('request', 'completion')),
    topic TEXT NOT NULL,
    payload_json BLOB NOT NULL CHECK(json_valid(payload_json)),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
    last_attempt_at INTEGER,
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    completed_at INTEGER,
    PRIMARY KEY(recovery_id, kind),
    FOREIGN KEY(recovery_id) REFERENCES edge_node_recovery_cases(recovery_id)
);
