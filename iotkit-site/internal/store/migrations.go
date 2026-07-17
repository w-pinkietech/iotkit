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
	{version: 4, sql: `
		CREATE TABLE IF NOT EXISTS site_devices (
			device_ref TEXT NOT NULL UNIQUE,
			edge_node_id TEXT NOT NULL,
			system_id TEXT NOT NULL,
			last_received_at INTEGER,
			created_at INTEGER NOT NULL,
			PRIMARY KEY (edge_node_id, system_id)
		);
		CREATE TABLE IF NOT EXISTS site_signals (
			signal_ref TEXT NOT NULL UNIQUE,
			edge_node_id TEXT NOT NULL,
			series_key TEXT NOT NULL,
			system_id TEXT,
			last_received_at INTEGER,
			created_at INTEGER NOT NULL,
			PRIMARY KEY (edge_node_id, series_key)
		);
		CREATE INDEX IF NOT EXISTS ix_site_signals_device
			ON site_signals(edge_node_id, system_id, signal_ref);
		CREATE TABLE IF NOT EXISTS inventory_projection_cursors (
			edge_node_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			last_pub_seq INTEGER NOT NULL CHECK(last_pub_seq >= 0),
			updated_at INTEGER NOT NULL,
			PRIMARY KEY (edge_node_id, ledger_epoch)
		);
		CREATE TABLE IF NOT EXISTS signal_current_values (
			edge_node_id TEXT NOT NULL,
			series_key TEXT NOT NULL,
			values_json BLOB NOT NULL CHECK(json_valid(values_json)),
			event_time INTEGER NOT NULL CHECK(event_time >= 0),
			site_received_at INTEGER NOT NULL,
			updated_at INTEGER NOT NULL,
			PRIMARY KEY (edge_node_id, series_key)
		);
		CREATE TABLE IF NOT EXISTS device_profiles (
			edge_node_id TEXT NOT NULL,
			system_id TEXT NOT NULL,
			display_name TEXT NOT NULL,
			location TEXT NOT NULL,
			revision INTEGER NOT NULL CHECK(revision > 0),
			updated_at INTEGER NOT NULL,
			PRIMARY KEY (edge_node_id, system_id)
		);
		CREATE TABLE IF NOT EXISTS signal_profiles (
			edge_node_id TEXT NOT NULL,
			series_key TEXT NOT NULL,
			display_name TEXT NOT NULL,
			revision INTEGER NOT NULL CHECK(revision > 0),
			updated_at INTEGER NOT NULL,
			PRIMARY KEY (edge_node_id, series_key)
		);
		INSERT OR IGNORE INTO site_devices(device_ref, edge_node_id, system_id, created_at)
		SELECT 'dev_' || lower(hex(randomblob(16))), edge_node_id, system_id, updated_at
		FROM descriptor_devices;
		INSERT OR IGNORE INTO site_signals(
			signal_ref, edge_node_id, series_key, system_id, last_received_at, created_at
		)
		SELECT 'sig_' || lower(hex(randomblob(16))), edge_node_id, series_key, system_id, NULL, updated_at
		FROM descriptor_signals;
	`},
	{version: 5, sql: `
		CREATE TABLE IF NOT EXISTS site_accounts (
			account_ref TEXT PRIMARY KEY,
			login_id TEXT NOT NULL,
			login_id_normalized TEXT NOT NULL UNIQUE,
			display_name TEXT NOT NULL,
			password_phc TEXT NOT NULL,
			role TEXT NOT NULL CHECK(role IN ('viewer', 'admin', 'system_admin')),
			state TEXT NOT NULL CHECK(state IN ('active', 'disabled')),
			must_change_password INTEGER NOT NULL CHECK(must_change_password IN (0, 1)),
			created_at INTEGER NOT NULL,
			updated_at INTEGER NOT NULL,
			disabled_at INTEGER
		);
		CREATE TABLE IF NOT EXISTS site_sessions (
			session_ref TEXT PRIMARY KEY,
			token_sha256 BLOB NOT NULL UNIQUE CHECK(length(token_sha256) = 32),
			csrf_sha256 BLOB NOT NULL CHECK(length(csrf_sha256) = 32),
			account_ref TEXT NOT NULL REFERENCES site_accounts(account_ref),
			issued_at INTEGER NOT NULL,
			last_seen_at INTEGER NOT NULL,
			idle_expires_at INTEGER NOT NULL,
			absolute_expires_at INTEGER NOT NULL,
			revoked_at INTEGER
		);
		CREATE INDEX IF NOT EXISTS ix_site_sessions_account_active
			ON site_sessions(account_ref, revoked_at);

		ALTER TABLE audit_events RENAME TO audit_events_v4;
		CREATE TABLE audit_events (
			audit_row_id INTEGER PRIMARY KEY AUTOINCREMENT,
			occurred_at INTEGER NOT NULL,
			actor_class TEXT NOT NULL CHECK(actor_class IN ('local_cli', 'settings_session', 'account', 'system')),
			actor_ref TEXT NOT NULL,
			actor_login_id TEXT,
			actor_display_name TEXT,
			operation TEXT NOT NULL,
			resource_ref TEXT NOT NULL,
			outcome TEXT NOT NULL CHECK(outcome IN ('success', 'failure')),
			summary_json BLOB NOT NULL CHECK(json_valid(summary_json))
		);
		INSERT INTO audit_events (
			audit_row_id, occurred_at, actor_class, actor_ref,
			operation, resource_ref, outcome, summary_json
		)
		SELECT audit_row_id, occurred_at, actor_class, actor_ref,
			operation, resource_ref, outcome, summary_json
		FROM audit_events_v4;
		DROP TABLE audit_events_v4;
	`},
	{version: 6, sql: `
		ALTER TABLE site_accounts
			ADD COLUMN revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0);
	`},
	{version: 7, sql: `
		CREATE TABLE semantic_definitions_v2 (
			definition_id TEXT NOT NULL,
			revision INTEGER NOT NULL CHECK(revision > 0),
			signal_ref TEXT NOT NULL,
			edge_node_id TEXT NOT NULL,
			series_key TEXT NOT NULL,
			series_id TEXT NOT NULL,
			spec_json BLOB NOT NULL CHECK(json_valid(spec_json)),
			active INTEGER NOT NULL CHECK(active IN (0, 1)),
			created_at INTEGER NOT NULL,
			PRIMARY KEY (definition_id, revision)
		);
		CREATE UNIQUE INDEX ux_semantic_definition_active_signal
			ON semantic_definitions_v2(signal_ref) WHERE active = 1;
		CREATE TABLE semantic_definition_starts_v2 (
			definition_id TEXT NOT NULL,
			definition_revision INTEGER NOT NULL,
			ledger_epoch TEXT NOT NULL,
			start_after_pub_seq INTEGER NOT NULL,
			PRIMARY KEY (definition_id, definition_revision, ledger_epoch)
		);
		CREATE TABLE semantic_definition_ends_v2 (
			definition_id TEXT NOT NULL,
			definition_revision INTEGER NOT NULL,
			ledger_epoch TEXT NOT NULL,
			end_at_pub_seq INTEGER NOT NULL,
			PRIMARY KEY (definition_id, definition_revision, ledger_epoch)
		);
		CREATE TABLE semantic_definition_state_v2 (
			definition_id TEXT NOT NULL,
			definition_revision INTEGER NOT NULL,
			initialized INTEGER NOT NULL CHECK(initialized IN (0, 1)),
			active INTEGER NOT NULL CHECK(active IN (0, 1)),
			counter INTEGER NOT NULL CHECK(counter >= 0),
			next_sequence INTEGER NOT NULL CHECK(next_sequence > 0),
			PRIMARY KEY (definition_id, definition_revision)
		);
		CREATE TABLE semantic_results_v2 (
			definition_id TEXT NOT NULL,
			definition_revision INTEGER NOT NULL,
			ledger_epoch TEXT NOT NULL,
			pub_seq INTEGER NOT NULL,
			observation_id TEXT,
			PRIMARY KEY (definition_id, definition_revision, ledger_epoch, pub_seq)
		);
		CREATE TABLE semantic_observations_v2 (
			observation_row_id INTEGER PRIMARY KEY AUTOINCREMENT,
			observation_id TEXT NOT NULL UNIQUE,
			series_id TEXT NOT NULL,
			sequence INTEGER NOT NULL CHECK(sequence > 0),
			definition_id TEXT NOT NULL,
			definition_revision INTEGER NOT NULL,
			kind TEXT NOT NULL CHECK(kind IN ('numeric', 'boolean', 'cumulative_counter', 'alarm')),
			value_json BLOB NOT NULL CHECK(json_valid(value_json)),
			signal_ref TEXT NOT NULL,
			edge_node_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			source_pub_seq INTEGER NOT NULL CHECK(source_pub_seq > 0),
			observed_at INTEGER NOT NULL CHECK(observed_at >= 0),
			created_at INTEGER NOT NULL,
			UNIQUE (definition_id, definition_revision, sequence)
		);
	`},
	{version: 8, sql: `
		CREATE TABLE yokakit_routes (
			route_id TEXT PRIMARY KEY,
			definition_id TEXT NOT NULL,
			source_id TEXT NOT NULL,
			signal_id TEXT NOT NULL,
			kind TEXT NOT NULL CHECK(kind IN ('production', 'onoff', 'gantt_chart', 'alarm')),
			reason TEXT NOT NULL,
			start_after_observation_row_id INTEGER NOT NULL CHECK(start_after_observation_row_id >= 0),
			active INTEGER NOT NULL CHECK(active IN (0, 1)),
			created_at INTEGER NOT NULL,
			UNIQUE(source_id, signal_id)
		);
		CREATE TABLE output_outbox_v2 (
			export_id TEXT PRIMARY KEY,
			route_id TEXT NOT NULL,
			observation_id TEXT NOT NULL,
			topic TEXT NOT NULL,
			qos INTEGER NOT NULL CHECK(qos = 1),
			payload_json BLOB NOT NULL CHECK(json_valid(payload_json)),
			attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
			created_at INTEGER NOT NULL,
			published_at INTEGER,
			UNIQUE(route_id, observation_id)
		);
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
