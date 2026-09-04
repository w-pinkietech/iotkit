-- Device-local processing pipelines (#232 child issue 3).
--
-- pipeline_definition: the operator-owned definition, edited from the Console
-- through typed operations. definition_json is the authority; the input_*
-- columns are a denormalized copy used to route accepted readings.
CREATE TABLE pipeline_definition (
    pipeline_id           TEXT    PRIMARY KEY,
    definition_json       TEXT    NOT NULL,
    structural_hash       TEXT    NOT NULL,
    input_adapter         TEXT    NOT NULL,
    input_subject         TEXT,
    input_measurement_key TEXT    NOT NULL,
    input_channel_index   INTEGER,
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL
);
CREATE INDEX ix_pipeline_definition_input
    ON pipeline_definition(input_adapter, input_measurement_key);

-- pipeline_state: evaluation state, current value, series, and next sequence.
-- structural_hash is the hash the series was started with; a mismatch with the
-- definition at startup or on edit starts a new series.
CREATE TABLE pipeline_state (
    pipeline_id      TEXT    PRIMARY KEY REFERENCES pipeline_definition(pipeline_id) ON DELETE CASCADE,
    structural_hash  TEXT    NOT NULL,
    series_id        TEXT    NOT NULL,
    next_sequence    INTEGER NOT NULL,
    initialized      INTEGER NOT NULL,
    active           INTEGER NOT NULL,
    counter          INTEGER NOT NULL,
    pending          INTEGER NOT NULL,
    pending_active   INTEGER NOT NULL,
    pending_since    INTEGER NOT NULL,
    last_value_json  TEXT,
    last_timestamp   INTEGER,
    updated_at       INTEGER NOT NULL
);

-- observation_outbox: publications not yet acknowledged by the Broker. Rows
-- are inserted in the same transaction as pipeline_state and deleted by the
-- MQTT Output Adapter after PUBACK. topic and payload are fixed at insert so a
-- retransmission sends identical bytes.
CREATE TABLE observation_outbox (
    outbox_seq  INTEGER PRIMARY KEY AUTOINCREMENT,
    pipeline_id TEXT    NOT NULL,
    topic       TEXT    NOT NULL,
    payload     BLOB    NOT NULL,
    retain      INTEGER NOT NULL,
    created_at  INTEGER NOT NULL
);
