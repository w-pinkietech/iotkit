package store

import (
	"context"
	"database/sql"
	"fmt"
)

type migration struct {
	version int
	sql     string
}

var schemaMigrations = []migration{
	{version: 1, sql: `
		CREATE TABLE IF NOT EXISTS raw_records (
			edge_node_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			pub_seq INTEGER NOT NULL,
			publication_id TEXT NOT NULL,
			record_json BLOB NOT NULL,
			record_sha256 BLOB NOT NULL,
			received_at INTEGER NOT NULL,
			PRIMARY KEY (edge_node_id, ledger_epoch, pub_seq)
		);
		CREATE TABLE IF NOT EXISTS accepted_cursors (
			edge_node_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			accepted_through INTEGER NOT NULL,
			updated_at INTEGER NOT NULL,
			PRIMARY KEY (edge_node_id, ledger_epoch)
		);
		CREATE TABLE IF NOT EXISTS semantic_mappings (
			mapping_id TEXT NOT NULL,
			revision INTEGER NOT NULL,
			edge_node_id TEXT NOT NULL,
			series_key TEXT NOT NULL,
			meaning TEXT NOT NULL CHECK(meaning = 'production_pulse'),
			trigger_mode TEXT NOT NULL CHECK(trigger_mode IN ('active_sample', 'active_edge')),
			active_value INTEGER NOT NULL CHECK(active_value IN (0, 1)),
			active INTEGER NOT NULL CHECK(active IN (0, 1)),
			created_at INTEGER NOT NULL,
			PRIMARY KEY (mapping_id, revision)
		);
		CREATE UNIQUE INDEX IF NOT EXISTS ux_semantic_one_active_per_source
			ON semantic_mappings(edge_node_id, series_key) WHERE active = 1;
		CREATE TABLE IF NOT EXISTS semantic_mapping_starts (
			mapping_id TEXT NOT NULL,
			mapping_revision INTEGER NOT NULL,
			ledger_epoch TEXT NOT NULL,
			start_after_pub_seq INTEGER NOT NULL,
			PRIMARY KEY (mapping_id, mapping_revision, ledger_epoch)
		);
		CREATE TABLE IF NOT EXISTS semantic_mapping_ends (
			mapping_id TEXT NOT NULL,
			mapping_revision INTEGER NOT NULL,
			ledger_epoch TEXT NOT NULL,
			end_at_pub_seq INTEGER NOT NULL,
			PRIMARY KEY (mapping_id, mapping_revision, ledger_epoch)
		);
		CREATE TABLE IF NOT EXISTS semantic_mapping_state (
			mapping_id TEXT NOT NULL,
			mapping_revision INTEGER NOT NULL,
			last_value INTEGER,
			next_event_sequence INTEGER NOT NULL,
			PRIMARY KEY (mapping_id, mapping_revision)
		);
		CREATE TABLE IF NOT EXISTS semantic_results (
			mapping_id TEXT NOT NULL,
			mapping_revision INTEGER NOT NULL,
			ledger_epoch TEXT NOT NULL,
			pub_seq INTEGER NOT NULL,
			emitted_event_id TEXT,
			PRIMARY KEY (mapping_id, mapping_revision, ledger_epoch, pub_seq)
		);
		CREATE TABLE IF NOT EXISTS semantic_events (
			event_row_id INTEGER PRIMARY KEY AUTOINCREMENT,
			event_id TEXT NOT NULL UNIQUE,
			mapping_id TEXT NOT NULL,
			mapping_revision INTEGER NOT NULL,
			event_sequence INTEGER NOT NULL,
			meaning TEXT NOT NULL,
			edge_node_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			source_pub_seq INTEGER NOT NULL,
			source_series_key TEXT NOT NULL,
			occurred_at INTEGER NOT NULL,
			created_at INTEGER NOT NULL,
			UNIQUE (mapping_id, mapping_revision, event_sequence)
		);
		CREATE TABLE IF NOT EXISTS mqtt_routes (
			route_id TEXT PRIMARY KEY,
			mapping_id TEXT NOT NULL,
			topic TEXT NOT NULL,
			qos INTEGER NOT NULL CHECK(qos = 1),
			start_after_event_row_id INTEGER NOT NULL,
			active INTEGER NOT NULL CHECK(active IN (0, 1)),
			created_at INTEGER NOT NULL,
			UNIQUE (mapping_id, topic)
		);
		CREATE TABLE IF NOT EXISTS mqtt_export_outbox (
			export_id TEXT PRIMARY KEY,
			route_id TEXT NOT NULL,
			event_id TEXT NOT NULL,
			topic TEXT NOT NULL,
			qos INTEGER NOT NULL,
			payload_json BLOB NOT NULL,
			attempts INTEGER NOT NULL DEFAULT 0,
			published_at INTEGER,
			created_at INTEGER NOT NULL,
			UNIQUE (route_id, event_id)
		);
	`},
	{version: 2, sql: `
		CREATE TABLE IF NOT EXISTS audit_events (
			audit_row_id INTEGER PRIMARY KEY AUTOINCREMENT,
			occurred_at INTEGER NOT NULL,
			actor_class TEXT NOT NULL CHECK(actor_class IN ('local_cli', 'settings_session', 'system')),
			actor_ref TEXT NOT NULL,
			operation TEXT NOT NULL,
			resource_ref TEXT NOT NULL,
			outcome TEXT NOT NULL CHECK(outcome IN ('success', 'failure')),
			summary_json BLOB NOT NULL CHECK(json_valid(summary_json))
		);
	`},
	{version: 3, sql: `
		CREATE TABLE IF NOT EXISTS edge_descriptor_state (
			edge_node_id TEXT PRIMARY KEY,
			ledger_epoch TEXT NOT NULL,
			descriptor_revision INTEGER NOT NULL CHECK(descriptor_revision > 0),
			content_sha256 BLOB NOT NULL CHECK(length(content_sha256) = 32),
			updated_at INTEGER NOT NULL
		);
		CREATE TABLE IF NOT EXISTS descriptor_devices (
			edge_node_id TEXT NOT NULL,
			system_id TEXT NOT NULL,
			identifier TEXT,
			state TEXT NOT NULL CHECK(state IN ('quarantined', 'active', 'retired')),
			presence TEXT NOT NULL CHECK(presence IN ('current', 'stale')),
			descriptor_revision INTEGER NOT NULL,
			updated_at INTEGER NOT NULL,
			PRIMARY KEY (edge_node_id, system_id)
		);
		CREATE TABLE IF NOT EXISTS descriptor_signals (
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
			PRIMARY KEY (edge_node_id, series_key)
		);
		CREATE INDEX IF NOT EXISTS ix_descriptor_devices_presence
			ON descriptor_devices(edge_node_id, presence, system_id);
		CREATE INDEX IF NOT EXISTS ix_descriptor_signals_presence
			ON descriptor_signals(edge_node_id, presence, series_key);
	`},
}

func applyMigrations(ctx context.Context, db *sql.DB) error {
	var current int
	if err := db.QueryRowContext(ctx, "PRAGMA user_version").Scan(&current); err != nil {
		return fmt.Errorf("read Site schema version: %w", err)
	}
	latest := schemaMigrations[len(schemaMigrations)-1].version
	if current > latest {
		return fmt.Errorf("Site schema version %d is newer than supported version %d", current, latest)
	}

	for _, migration := range schemaMigrations {
		if migration.version <= current {
			continue
		}
		tx, err := db.BeginTx(ctx, nil)
		if err != nil {
			return fmt.Errorf("begin Site schema migration %d: %w", migration.version, err)
		}
		if _, err := tx.ExecContext(ctx, migration.sql); err != nil {
			_ = tx.Rollback()
			return fmt.Errorf("apply Site schema migration %d: %w", migration.version, err)
		}
		if _, err := tx.ExecContext(ctx, fmt.Sprintf("PRAGMA user_version = %d", migration.version)); err != nil {
			_ = tx.Rollback()
			return fmt.Errorf("record Site schema migration %d: %w", migration.version, err)
		}
		if err := tx.Commit(); err != nil {
			return fmt.Errorf("commit Site schema migration %d: %w", migration.version, err)
		}
		current = migration.version
	}
	return nil
}
