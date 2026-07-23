PRAGMA defer_foreign_keys = ON;

ALTER TABLE output_route_attempts RENAME TO output_route_attempts_before_fanout;
ALTER TABLE output_outbox RENAME TO output_outbox_before_fanout;
ALTER TABLE output_routes RENAME TO output_routes_before_fanout;
DROP INDEX ix_output_outbox_pending;

CREATE TABLE output_routes (
    route_id TEXT PRIMARY KEY,
    binding_id TEXT NOT NULL REFERENCES output_bindings(binding_id),
    rule_id TEXT NOT NULL,
    adapter_id TEXT NOT NULL,
    config_schema_version INTEGER NOT NULL,
    config_json BLOB NOT NULL CHECK(json_valid(CAST(config_json AS TEXT))),
    start_after_observation_row_id INTEGER NOT NULL DEFAULT 0
        CHECK(start_after_observation_row_id >= 0),
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
CREATE INDEX ix_output_routes_binding
    ON output_routes(binding_id, created_at, route_id);

INSERT INTO output_routes(
    route_id,binding_id,rule_id,adapter_id,config_schema_version,config_json,
    start_after_observation_row_id,active,lifecycle_state,last_transform_error_code,
    last_transform_error_at,last_transform_success_at,created_at
)
SELECT route_id,binding_id,rule_id,adapter_id,config_schema_version,config_json,
       0,active,lifecycle_state,last_transform_error_code,last_transform_error_at,
       last_transform_success_at,created_at
FROM output_routes_before_fanout;

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
INSERT INTO output_outbox SELECT * FROM output_outbox_before_fanout;

CREATE TABLE output_route_attempts (
    route_id TEXT NOT NULL REFERENCES output_routes(route_id),
    observation_id TEXT NOT NULL REFERENCES semantic_observations(observation_id),
    attempts INTEGER NOT NULL CHECK(attempts > 0),
    last_attempt_at INTEGER NOT NULL,
    error_code TEXT NOT NULL,
    PRIMARY KEY(route_id, observation_id)
);
INSERT INTO output_route_attempts SELECT * FROM output_route_attempts_before_fanout;

DROP TABLE output_route_attempts_before_fanout;
DROP TABLE output_outbox_before_fanout;
DROP TABLE output_routes_before_fanout;
