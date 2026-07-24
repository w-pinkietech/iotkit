CREATE TABLE raw_records (
    edge_node_id TEXT NOT NULL,
    ledger_epoch TEXT NOT NULL,
    pub_seq INTEGER NOT NULL CHECK(pub_seq > 0),
    publication_id TEXT NOT NULL,
    record_json BLOB NOT NULL CHECK(json_valid(record_json)),
    record_sha256 BLOB NOT NULL CHECK(length(record_sha256) = 32),
    received_at INTEGER NOT NULL CHECK(received_at >= 0),
    PRIMARY KEY (edge_node_id, ledger_epoch, pub_seq)
);

CREATE TABLE accepted_cursors (
    edge_node_id TEXT NOT NULL,
    ledger_epoch TEXT NOT NULL,
    accepted_through INTEGER NOT NULL CHECK(accepted_through >= 0),
    updated_at INTEGER NOT NULL CHECK(updated_at >= 0),
    PRIMARY KEY (edge_node_id, ledger_epoch)
);

CREATE INDEX idx_raw_records_history_received
ON raw_records(
    received_at DESC,
    edge_node_id DESC,
    ledger_epoch DESC,
    pub_seq DESC
);
