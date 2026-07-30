CREATE TABLE edge_node_recovery_cases (
    recovery_id TEXT PRIMARY KEY,
    state TEXT NOT NULL CHECK(state IN ('prepared', 'authorized', 'completed', 'recovery_hold')),
    edge_node_id TEXT NOT NULL,
    backup_id TEXT NOT NULL,
    old_ledger_epoch TEXT NOT NULL,
    new_ledger_epoch TEXT NOT NULL,
    broker_fence_id TEXT NOT NULL,
    broker_credential_generation BIGINT NOT NULL CHECK(broker_credential_generation > 0),
    backup_created_at BIGINT NOT NULL CHECK(backup_created_at >= 0),
    broker_fenced_at BIGINT NOT NULL CHECK(broker_fenced_at >= 0),
    device_auth_generation BIGINT CHECK(device_auth_generation >= 0),
    candidate_instance_id TEXT,
    snapshot_accepted_through BIGINT NOT NULL CHECK(snapshot_accepted_through >= 0),
    snapshot_allocation_high_water BIGINT NOT NULL CHECK(snapshot_allocation_high_water >= snapshot_accepted_through),
    snapshot_epoch_start_publication_seq BIGINT
        CHECK(snapshot_epoch_start_publication_seq IS NULL OR
              snapshot_epoch_start_publication_seq BETWEEN 1 AND snapshot_allocation_high_water),
    edge_accepted_through BIGINT NOT NULL CHECK(edge_accepted_through >= snapshot_accepted_through),
    request_json BYTEA,
    result_json BYTEA,
    completion_json BYTEA,
    replayed_records BIGINT CHECK(replayed_records >= 0),
    last_new_publication_seq BIGINT CHECK(last_new_publication_seq >= 1),
    created_at BIGINT NOT NULL CHECK(created_at >= 0),
    updated_at BIGINT NOT NULL CHECK(updated_at >= created_at),
    completed_at BIGINT,
    UNIQUE(edge_node_id, backup_id),
    UNIQUE(edge_node_id, new_ledger_epoch)
);

CREATE UNIQUE INDEX one_open_recovery_per_edge_node
ON edge_node_recovery_cases(edge_node_id)
WHERE state IN ('prepared', 'authorized');

CREATE TABLE recovery_command_outbox (
    recovery_id TEXT NOT NULL REFERENCES edge_node_recovery_cases(recovery_id),
    kind TEXT NOT NULL CHECK(kind IN ('request', 'completion')),
    topic TEXT NOT NULL,
    payload_json BYTEA NOT NULL,
    attempts BIGINT NOT NULL DEFAULT 0 CHECK(attempts >= 0),
    last_attempt_at BIGINT,
    created_at BIGINT NOT NULL CHECK(created_at >= 0),
    completed_at BIGINT,
    PRIMARY KEY(recovery_id, kind)
);
