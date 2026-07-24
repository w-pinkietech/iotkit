CREATE TABLE raw_records (
    edge_node_id TEXT NOT NULL,
    ledger_epoch TEXT NOT NULL,
    pub_seq BIGINT NOT NULL CHECK(pub_seq > 0),
    publication_id TEXT NOT NULL,
    record_json BYTEA NOT NULL,
    record_sha256 BYTEA NOT NULL CHECK(octet_length(record_sha256) = 32),
    received_at BIGINT NOT NULL CHECK(received_at >= 0),
    PRIMARY KEY (edge_node_id, ledger_epoch, pub_seq)
);

CREATE TABLE accepted_cursors (
    edge_node_id TEXT NOT NULL,
    ledger_epoch TEXT NOT NULL,
    accepted_through BIGINT NOT NULL CHECK(accepted_through >= 0),
    updated_at BIGINT NOT NULL CHECK(updated_at >= 0),
    PRIMARY KEY (edge_node_id, ledger_epoch)
);

CREATE INDEX idx_raw_records_history_received
ON raw_records(
    received_at DESC,
    edge_node_id DESC,
    ledger_epoch DESC,
    pub_seq DESC
);
