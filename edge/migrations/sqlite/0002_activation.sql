CREATE TABLE edge_meta (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    edge_id TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL CHECK(created_at >= 0)
);

CREATE TABLE edge_descriptor_state (
    edge_node_id TEXT PRIMARY KEY,
    ledger_epoch TEXT NOT NULL,
    descriptor_revision INTEGER NOT NULL CHECK(descriptor_revision > 0),
    content_sha256 BLOB NOT NULL CHECK(length(content_sha256) = 32),
    updated_at INTEGER NOT NULL CHECK(updated_at >= 0)
);

CREATE TABLE descriptor_devices (
    edge_node_id TEXT NOT NULL,
    system_id TEXT NOT NULL,
    identifier TEXT,
    state TEXT NOT NULL CHECK(state IN ('quarantined', 'active', 'retired')),
    presence TEXT NOT NULL CHECK(presence IN ('current', 'stale')),
    descriptor_revision INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    model_id TEXT,
    PRIMARY KEY(edge_node_id, system_id)
);

CREATE TABLE descriptor_signals (
    edge_node_id TEXT NOT NULL,
    series_key TEXT NOT NULL,
    system_id TEXT NOT NULL,
    measurement_key TEXT NOT NULL,
    channel_index INTEGER,
    variant TEXT NOT NULL,
    unit TEXT,
    value_type TEXT NOT NULL CHECK(value_type IN ('float', 'int', 'bool', 'record')),
    presence TEXT NOT NULL CHECK(presence IN ('current', 'stale')),
    descriptor_revision INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(edge_node_id, series_key)
);

CREATE TABLE edge_node_activations (
    edge_node_ref TEXT PRIMARY KEY,
    edge_node_id TEXT NOT NULL UNIQUE,
    ledger_epoch TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('discovered', 'activating', 'active', 'recovery_hold')),
    activation_id TEXT UNIQUE,
    grant_revision INTEGER NOT NULL DEFAULT 0 CHECK(grant_revision >= 0),
    request_json BLOB CHECK(request_json IS NULL OR json_valid(request_json)),
    result_json BLOB CHECK(result_json IS NULL OR json_valid(result_json)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    last_descriptor_at INTEGER,
    last_result_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE activation_command_outbox (
    activation_id TEXT PRIMARY KEY,
    topic TEXT NOT NULL,
    payload_json BLOB NOT NULL CHECK(json_valid(payload_json)),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
    last_attempt_at INTEGER,
    created_at INTEGER NOT NULL,
    completed_at INTEGER
);
