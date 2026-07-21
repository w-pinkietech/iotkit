CREATE TABLE edge_node_activation (
    singleton                    INTEGER PRIMARY KEY CHECK (singleton = 1),
    state                        TEXT NOT NULL CHECK (
        state IN ('standalone', 'discovery_only', 'active')
    ),
    edge_id                      TEXT,
    activation_id                TEXT,
    ledger_epoch                 TEXT,
    discard_through_reading_seq  INTEGER,
    cleanup_through_reading_seq  INTEGER NOT NULL DEFAULT 0,
    request_json                 TEXT,
    result_json                  TEXT,
    activated_at                 INTEGER
);

INSERT INTO edge_node_activation(singleton, state)
VALUES(
    1,
    CASE
        WHEN EXISTS(SELECT 1 FROM target_registry) THEN 'active'
        ELSE 'standalone'
    END
);
