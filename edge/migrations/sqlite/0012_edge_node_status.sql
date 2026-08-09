-- Latest-only bounded Edge Node operational evidence. This is not custody
-- history: a retained MQTT replay cannot advance last_live_received_at.
CREATE TABLE edge_node_status (
    edge_node_id TEXT PRIMARY KEY REFERENCES edge_node_activations(edge_node_id) ON DELETE CASCADE,
    ledger_epoch TEXT NOT NULL,
    boot_id TEXT NOT NULL,
    status_seq INTEGER NOT NULL CHECK(status_seq > 0),
    collector_state TEXT NOT NULL CHECK(collector_state IN ('running', 'stopped')),
    adapters_json BLOB NOT NULL CHECK(json_valid(adapters_json)),
    accepted_through INTEGER NOT NULL CHECK(accepted_through >= 0),
    pending_publications INTEGER NOT NULL CHECK(pending_publications >= 0),
    storage_pressure INTEGER NOT NULL CHECK(storage_pressure IN (0, 1)),
    received_at INTEGER NOT NULL CHECK(received_at >= 0),
    last_live_received_at INTEGER CHECK(
        last_live_received_at IS NULL OR
        (last_live_received_at >= 0 AND last_live_received_at <= received_at)
    ),
    -- Edge receipt time for the current pending interval only. Retained
    -- historical status intentionally leaves this NULL.
    pending_since_at INTEGER CHECK(pending_since_at IS NULL OR pending_since_at >= 0)
);

CREATE INDEX ix_edge_node_status_live
ON edge_node_status(last_live_received_at, edge_node_id);

-- Causal diagnostics asks whether a terminal failure has a later success for
-- this exact active rule and ledger epoch.  This avoids scanning all retained
-- semantic history when classifying recovery.
CREATE INDEX ix_semantic_observation_recovery
ON semantic_observations(rule_id, ledger_epoch, source_pub_seq);

-- Bounded causal diagnostics looks up the current active epoch and each
-- active configuration directly; it must not scan retained history.
CREATE INDEX ix_raw_records_diagnostic_epoch_signal_received
ON raw_records(edge_node_id, ledger_epoch, series_key, received_at DESC, pub_seq DESC);

CREATE INDEX ix_semantic_observation_diagnostic_latest
ON semantic_observations(rule_id, created_at DESC);

CREATE INDEX ix_output_outbox_diagnostic_route_published
ON output_outbox(route_id, published_at DESC) WHERE published_at IS NOT NULL;

CREATE INDEX ix_output_outbox_diagnostic_route_pending
ON output_outbox(route_id, created_at) WHERE published_at IS NULL;
