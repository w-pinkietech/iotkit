CREATE TABLE semantic_signals (
    signal_ref TEXT PRIMARY KEY,
    edge_node_id TEXT NOT NULL,
    series_key TEXT NOT NULL,
    calibration_revision INTEGER NOT NULL DEFAULT 1 CHECK(calibration_revision > 0),
    scale REAL NOT NULL DEFAULT 1,
    calibration_offset REAL NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    UNIQUE(edge_node_id, series_key)
);

CREATE TABLE semantic_rules (
    rule_id TEXT PRIMARY KEY,
    signal_ref TEXT NOT NULL REFERENCES semantic_signals(signal_ref),
    display_name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('numeric', 'boolean', 'cumulative_counter', 'alarm')),
    series_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    spec_json BLOB NOT NULL CHECK(json_valid(CAST(spec_json AS TEXT))),
    active INTEGER NOT NULL CHECK(active IN (0, 1)),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    retired_at INTEGER,
    UNIQUE(series_id)
);
CREATE UNIQUE INDEX ux_semantic_rule_active_name
    ON semantic_rules(signal_ref, display_name) WHERE active = 1;

CREATE TABLE semantic_rule_revisions (
    rule_id TEXT NOT NULL REFERENCES semantic_rules(rule_id),
    revision INTEGER NOT NULL CHECK(revision > 0),
    series_id TEXT NOT NULL,
    spec_json BLOB NOT NULL CHECK(json_valid(CAST(spec_json AS TEXT))),
    created_at INTEGER NOT NULL,
    PRIMARY KEY(rule_id, revision)
);

CREATE TABLE semantic_rule_starts (
    rule_id TEXT NOT NULL REFERENCES semantic_rules(rule_id),
    revision INTEGER NOT NULL,
    ledger_epoch TEXT NOT NULL,
    start_after_pub_seq INTEGER NOT NULL CHECK(start_after_pub_seq >= 0),
    PRIMARY KEY(rule_id, revision, ledger_epoch)
);

CREATE TABLE semantic_rule_ends (
    rule_id TEXT NOT NULL,
    ledger_epoch TEXT NOT NULL,
    end_at_pub_seq INTEGER NOT NULL CHECK(end_at_pub_seq >= 0),
    PRIMARY KEY(rule_id, ledger_epoch)
);

CREATE TABLE semantic_calibration_revisions (
    signal_ref TEXT NOT NULL REFERENCES semantic_signals(signal_ref),
    revision INTEGER NOT NULL CHECK(revision > 0),
    scale REAL NOT NULL,
    calibration_offset REAL NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(signal_ref, revision)
);

CREATE TABLE semantic_calibration_starts (
    signal_ref TEXT NOT NULL,
    revision INTEGER NOT NULL,
    ledger_epoch TEXT NOT NULL,
    start_after_pub_seq INTEGER NOT NULL CHECK(start_after_pub_seq >= 0),
    PRIMARY KEY(signal_ref, revision, ledger_epoch)
);

CREATE TABLE semantic_rule_runtime (
    rule_id TEXT PRIMARY KEY REFERENCES semantic_rules(rule_id),
    initialized INTEGER NOT NULL DEFAULT 0 CHECK(initialized IN (0, 1)),
    detector_active INTEGER NOT NULL DEFAULT 0 CHECK(detector_active IN (0, 1)),
    counter INTEGER NOT NULL DEFAULT 0 CHECK(counter >= 0),
    pending INTEGER NOT NULL DEFAULT 0 CHECK(pending IN (0, 1)),
    pending_active INTEGER NOT NULL DEFAULT 0 CHECK(pending_active IN (0, 1)),
    pending_since INTEGER NOT NULL DEFAULT 0 CHECK(pending_since >= 0),
    applied_revision INTEGER NOT NULL DEFAULT 0,
    applied_calibration_revision INTEGER NOT NULL DEFAULT 0,
    applied_ledger_epoch TEXT NOT NULL DEFAULT '',
    applied_series_id TEXT NOT NULL DEFAULT '',
    next_sequence INTEGER NOT NULL DEFAULT 1 CHECK(next_sequence > 0)
);

CREATE TABLE semantic_projection_receipts (
    rule_id TEXT NOT NULL,
    ledger_epoch TEXT NOT NULL,
    pub_seq INTEGER NOT NULL CHECK(pub_seq > 0),
    revision INTEGER NOT NULL,
    calibration_revision INTEGER NOT NULL,
    observation_id TEXT,
    PRIMARY KEY(rule_id, ledger_epoch, pub_seq)
);

CREATE TABLE semantic_observations (
    observation_row_id INTEGER PRIMARY KEY AUTOINCREMENT,
    observation_id TEXT NOT NULL UNIQUE,
    rule_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    calibration_revision INTEGER NOT NULL,
    series_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    kind TEXT NOT NULL CHECK(kind IN ('numeric', 'boolean', 'cumulative_counter', 'alarm')),
    value_json BLOB NOT NULL CHECK(json_valid(CAST(value_json AS TEXT))),
    reading REAL,
    signal_ref TEXT NOT NULL,
    edge_node_id TEXT NOT NULL,
    ledger_epoch TEXT NOT NULL,
    source_pub_seq INTEGER NOT NULL CHECK(source_pub_seq >= 0),
    observed_at INTEGER NOT NULL CHECK(observed_at >= 0),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    UNIQUE(series_id, sequence)
);
CREATE INDEX ix_semantic_observation_rule_row
    ON semantic_observations(rule_id, observation_row_id);

CREATE TABLE semantic_projection_failures (
    rule_id TEXT NOT NULL,
    ledger_epoch TEXT NOT NULL,
    pub_seq INTEGER NOT NULL,
    error_code TEXT NOT NULL,
    attempts INTEGER NOT NULL CHECK(attempts > 0),
    last_failed_at INTEGER NOT NULL,
    PRIMARY KEY(rule_id, ledger_epoch, pub_seq)
);

CREATE TABLE semantic_counter_resets (
    reset_id TEXT PRIMARY KEY,
    rule_id TEXT NOT NULL,
    requested_at INTEGER NOT NULL,
    applied_at INTEGER,
    zero_observation_id TEXT
);

CREATE TABLE semantic_counter_reset_boundaries (
    reset_id TEXT NOT NULL REFERENCES semantic_counter_resets(reset_id),
    ledger_epoch TEXT NOT NULL,
    apply_after_pub_seq INTEGER NOT NULL CHECK(apply_after_pub_seq >= 0),
    PRIMARY KEY(reset_id, ledger_epoch)
);

CREATE TABLE export_profiles (
    profile_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    adapter_id TEXT NOT NULL,
    adapter_schema_version INTEGER NOT NULL CHECK(adapter_schema_version > 0),
    setup_json BLOB NOT NULL CHECK(json_valid(CAST(setup_json AS TEXT))),
    state TEXT NOT NULL CHECK(state IN ('preparing', 'active', 'draining', 'stopped')),
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at INTEGER NOT NULL,
    stopped_at INTEGER
);
CREATE UNIQUE INDEX ux_export_profile_live_adapter
    ON export_profiles(adapter_id) WHERE state IN ('preparing', 'active', 'draining');

CREATE TABLE output_identities (
    output_identity_id TEXT PRIMARY KEY,
    adapter_id TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    external_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    confirmed_at INTEGER,
    UNIQUE(adapter_id, scope_key),
    UNIQUE(adapter_id, external_id)
);

CREATE TABLE output_bindings (
    binding_id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES export_profiles(profile_id),
    rule_id TEXT NOT NULL REFERENCES semantic_rules(rule_id),
    output_identity_id TEXT,
    mode TEXT,
    state TEXT NOT NULL CHECK(state IN (
        'needs_configuration', 'prepared', 'active', 'ineligible', 'draining', 'stopped'
    )),
    ineligible_reason TEXT NOT NULL DEFAULT '',
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at INTEGER NOT NULL,
    activated_at INTEGER,
    stopped_at INTEGER,
    UNIQUE(profile_id, rule_id)
);

CREATE TABLE output_binding_starts (
    binding_id TEXT NOT NULL REFERENCES output_bindings(binding_id),
    ledger_epoch TEXT NOT NULL,
    start_after_pub_seq INTEGER NOT NULL CHECK(start_after_pub_seq >= 0),
    PRIMARY KEY(binding_id, ledger_epoch)
);

CREATE TABLE output_binding_ends (
    binding_id TEXT NOT NULL REFERENCES output_bindings(binding_id),
    ledger_epoch TEXT NOT NULL,
    end_at_pub_seq INTEGER NOT NULL CHECK(end_at_pub_seq >= 0),
    PRIMARY KEY(binding_id, ledger_epoch)
);

CREATE TABLE output_routes (
    route_id TEXT PRIMARY KEY,
    binding_id TEXT NOT NULL UNIQUE REFERENCES output_bindings(binding_id),
    rule_id TEXT NOT NULL,
    adapter_id TEXT NOT NULL,
    config_schema_version INTEGER NOT NULL,
    config_json BLOB NOT NULL CHECK(json_valid(CAST(config_json AS TEXT))),
    active INTEGER NOT NULL CHECK(active IN (0, 1)),
    lifecycle_state TEXT NOT NULL CHECK(lifecycle_state IN ('active', 'draining', 'stopped')),
    last_transform_error_code TEXT CHECK(last_transform_error_code IS NULL OR
        last_transform_error_code IN (
            'adapter_unavailable', 'config_version_mismatch',
            'invalid_observation', 'transform_failed'
        )),
    last_transform_error_at INTEGER,
    last_transform_success_at INTEGER,
    created_at INTEGER NOT NULL
);

CREATE TABLE output_outbox (
    export_id TEXT PRIMARY KEY,
    route_id TEXT NOT NULL REFERENCES output_routes(route_id),
    observation_id TEXT NOT NULL REFERENCES semantic_observations(observation_id),
    topic TEXT NOT NULL,
    qos INTEGER NOT NULL CHECK(qos = 1),
    retain INTEGER NOT NULL CHECK(retain IN (0, 1)),
    payload_json BLOB NOT NULL CHECK(json_valid(CAST(payload_json AS TEXT))),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
    created_at INTEGER NOT NULL,
    published_at INTEGER,
    claim_token TEXT,
    claimed_at INTEGER,
    claim_until INTEGER,
    UNIQUE(route_id, observation_id)
);
CREATE INDEX ix_output_outbox_pending
    ON output_outbox(published_at, claim_until, created_at);

CREATE TABLE output_route_attempts (
    route_id TEXT NOT NULL REFERENCES output_routes(route_id),
    observation_id TEXT NOT NULL REFERENCES semantic_observations(observation_id),
    attempts INTEGER NOT NULL CHECK(attempts > 0),
    last_attempt_at INTEGER NOT NULL,
    error_code TEXT NOT NULL,
    PRIMARY KEY(route_id, observation_id)
);
