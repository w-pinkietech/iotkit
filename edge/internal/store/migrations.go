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
		CREATE TABLE IF NOT EXISTS edge_devices (
			device_ref TEXT NOT NULL UNIQUE,
			edge_node_id TEXT NOT NULL,
			system_id TEXT NOT NULL,
			last_received_at INTEGER,
			created_at INTEGER NOT NULL,
			PRIMARY KEY (edge_node_id, system_id)
		);
		CREATE TABLE IF NOT EXISTS edge_signals (
			signal_ref TEXT NOT NULL UNIQUE,
			edge_node_id TEXT NOT NULL,
			series_key TEXT NOT NULL,
			system_id TEXT,
			last_received_at INTEGER,
			created_at INTEGER NOT NULL,
			PRIMARY KEY (edge_node_id, series_key)
		);
		CREATE INDEX IF NOT EXISTS ix_edge_signals_device
			ON edge_signals(edge_node_id, system_id, signal_ref);
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
			edge_received_at INTEGER NOT NULL,
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
		INSERT OR IGNORE INTO edge_devices(device_ref, edge_node_id, system_id, created_at)
		SELECT 'dev_' || lower(hex(randomblob(16))), edge_node_id, system_id, updated_at
		FROM descriptor_devices;
		INSERT OR IGNORE INTO edge_signals(
			signal_ref, edge_node_id, series_key, system_id, last_received_at, created_at
		)
		SELECT 'sig_' || lower(hex(randomblob(16))), edge_node_id, series_key, system_id, NULL, updated_at
		FROM descriptor_signals;
	`},
	{version: 5, sql: `
		CREATE TABLE IF NOT EXISTS edge_accounts (
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
		CREATE TABLE IF NOT EXISTS edge_sessions (
			session_ref TEXT PRIMARY KEY,
			token_sha256 BLOB NOT NULL UNIQUE CHECK(length(token_sha256) = 32),
			csrf_sha256 BLOB NOT NULL CHECK(length(csrf_sha256) = 32),
			account_ref TEXT NOT NULL REFERENCES edge_accounts(account_ref),
			issued_at INTEGER NOT NULL,
			last_seen_at INTEGER NOT NULL,
			idle_expires_at INTEGER NOT NULL,
			absolute_expires_at INTEGER NOT NULL,
			revoked_at INTEGER
		);
		CREATE INDEX IF NOT EXISTS ix_edge_sessions_account_active
			ON edge_sessions(account_ref, revoked_at);

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
		ALTER TABLE edge_accounts
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
	{version: 9, sql: `
		CREATE TABLE semantic_projection_failures_v2 (
			definition_id TEXT NOT NULL,
			definition_revision INTEGER NOT NULL,
			ledger_epoch TEXT NOT NULL,
			pub_seq INTEGER NOT NULL,
			error_text TEXT NOT NULL,
			attempts INTEGER NOT NULL CHECK(attempts > 0),
			last_failed_at INTEGER NOT NULL,
			PRIMARY KEY (definition_id, definition_revision, ledger_epoch, pub_seq)
		);
	`},
	{version: 10, sql: `
		ALTER TABLE signal_profiles
			ADD COLUMN display_sensor_type TEXT NOT NULL DEFAULT '';
		ALTER TABLE signal_profiles
			ADD COLUMN display_sensor_type_label TEXT NOT NULL DEFAULT '';
		ALTER TABLE signal_profiles
			ADD COLUMN display_value_kind TEXT NOT NULL DEFAULT '';
		ALTER TABLE signal_profiles
			ADD COLUMN display_unit_mode TEXT NOT NULL DEFAULT '';
		ALTER TABLE signal_profiles
			ADD COLUMN display_unit TEXT NOT NULL DEFAULT '';
		ALTER TABLE signal_profiles
			ADD COLUMN decimal_places INTEGER NOT NULL DEFAULT 0
			CHECK(decimal_places BETWEEN 0 AND 6);
	`},
	{version: 11, sql: `
		CREATE TABLE edge_meta (
			singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
			edge_id TEXT NOT NULL UNIQUE,
			created_at INTEGER NOT NULL
		);
		INSERT INTO edge_meta(singleton, edge_id, created_at)
		VALUES(1, 'edge-' || lower(hex(randomblob(16))), unixepoch('subsec') * 1000);

		CREATE TABLE edge_node_activations (
			edge_node_ref TEXT PRIMARY KEY,
			edge_node_id TEXT NOT NULL UNIQUE,
			ledger_epoch TEXT NOT NULL,
			state TEXT NOT NULL CHECK(state IN (
				'discovered', 'activating', 'active', 'recovery_hold'
			)),
			activation_id TEXT UNIQUE,
			grant_revision INTEGER NOT NULL DEFAULT 0 CHECK(grant_revision >= 0),
			display_name TEXT NOT NULL DEFAULT '',
			location TEXT NOT NULL DEFAULT '',
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

		INSERT INTO edge_node_activations(
			edge_node_ref, edge_node_id, ledger_epoch, state,
			revision, last_descriptor_at, created_at, updated_at
		)
		SELECT
			'edge_' || lower(hex(randomblob(16))),
			edge_node_id,
			ledger_epoch,
			'discovered',
			1,
			updated_at,
			updated_at,
			updated_at
		FROM edge_descriptor_state;

		WITH custody_epochs AS (
			SELECT edge_node_id, ledger_epoch FROM raw_records
			UNION
			SELECT edge_node_id, ledger_epoch FROM accepted_cursors
		),
		custody_summary AS (
			SELECT edge_node_id, MIN(ledger_epoch) AS ledger_epoch,
				COUNT(DISTINCT ledger_epoch) AS epoch_count
			FROM custody_epochs
			GROUP BY edge_node_id
		)
		INSERT INTO edge_node_activations(
			edge_node_ref, edge_node_id, ledger_epoch, state,
			revision, created_at, updated_at
		)
		SELECT
			'edge_' || lower(hex(randomblob(16))),
			edge_node_id,
			ledger_epoch,
			CASE WHEN epoch_count = 1 THEN 'active' ELSE 'recovery_hold' END,
			1,
			unixepoch('subsec') * 1000,
			unixepoch('subsec') * 1000
		FROM custody_summary
		WHERE true
		ON CONFLICT(edge_node_id) DO UPDATE SET
			ledger_epoch = excluded.ledger_epoch,
			state = excluded.state,
			revision = edge_node_activations.revision + 1,
			updated_at = excluded.updated_at;
	`},
	{version: 12, sql: `
		CREATE INDEX idx_raw_records_signal_preview
		ON raw_records(
			edge_node_id,
			json_extract(record_json, '$.series_key'),
			received_at DESC,
			ledger_epoch DESC,
			pub_seq DESC
		);
	`},
	{version: 13, sql: `
		ALTER TABLE semantic_definition_state_v2
			ADD COLUMN pending INTEGER NOT NULL DEFAULT 0
			CHECK(pending IN (0, 1));
		ALTER TABLE semantic_definition_state_v2
			ADD COLUMN pending_active INTEGER NOT NULL DEFAULT 0
			CHECK(pending_active IN (0, 1));
		ALTER TABLE semantic_definition_state_v2
			ADD COLUMN pending_since INTEGER NOT NULL DEFAULT 0
			CHECK(pending_since >= 0);
	`},
	{version: 14, sql: `
		CREATE TABLE semantic_signal_configs_v3 (
			signal_ref TEXT PRIMARY KEY,
			revision INTEGER NOT NULL CHECK(revision > 0)
		);
		CREATE TABLE signal_calibration_revisions_v3 (
			signal_ref TEXT NOT NULL,
			revision INTEGER NOT NULL CHECK(revision > 0),
			scale REAL NOT NULL,
			offset REAL NOT NULL,
			active INTEGER NOT NULL CHECK(active IN (0, 1)),
			created_at INTEGER NOT NULL,
			PRIMARY KEY(signal_ref, revision)
		);
		CREATE UNIQUE INDEX ux_signal_calibration_active_v3
			ON signal_calibration_revisions_v3(signal_ref) WHERE active = 1;
		CREATE TABLE signal_calibration_starts_v3 (
			signal_ref TEXT NOT NULL,
			calibration_revision INTEGER NOT NULL,
			ledger_epoch TEXT NOT NULL,
			start_after_pub_seq INTEGER NOT NULL CHECK(start_after_pub_seq >= 0),
			PRIMARY KEY(signal_ref, calibration_revision, ledger_epoch)
		);
		CREATE TABLE semantic_rules_v3 (
			rule_id TEXT PRIMARY KEY,
			signal_ref TEXT NOT NULL,
			display_name TEXT NOT NULL,
			kind TEXT NOT NULL CHECK(kind IN (
				'numeric', 'boolean', 'cumulative_counter', 'alarm'
			)),
			series_id TEXT NOT NULL UNIQUE,
			display_order INTEGER NOT NULL CHECK(display_order > 0),
			created_at INTEGER NOT NULL,
			retired_at INTEGER
		);
		CREATE UNIQUE INDEX ux_semantic_rule_name_v3
			ON semantic_rules_v3(signal_ref, display_name)
			WHERE retired_at IS NULL;
		CREATE INDEX ix_semantic_rules_signal_v3
			ON semantic_rules_v3(signal_ref, rule_id);
		CREATE UNIQUE INDEX ux_semantic_rule_display_order_v3
			ON semantic_rules_v3(signal_ref, display_order);
		CREATE TABLE semantic_rule_revisions_v3 (
			rule_id TEXT NOT NULL,
			revision INTEGER NOT NULL CHECK(revision > 0),
			spec_json BLOB NOT NULL CHECK(json_valid(spec_json)),
			active INTEGER NOT NULL CHECK(active IN (0, 1)),
			created_at INTEGER NOT NULL,
			PRIMARY KEY(rule_id, revision)
		);
		CREATE UNIQUE INDEX ux_semantic_rule_revision_active_v3
			ON semantic_rule_revisions_v3(rule_id) WHERE active = 1;
		CREATE TABLE semantic_rule_starts_v3 (
			rule_id TEXT NOT NULL,
			rule_revision INTEGER NOT NULL,
			ledger_epoch TEXT NOT NULL,
			start_after_pub_seq INTEGER NOT NULL CHECK(start_after_pub_seq >= 0),
			PRIMARY KEY(rule_id, rule_revision, ledger_epoch)
		);
		CREATE TABLE semantic_rule_ends_v3 (
			rule_id TEXT NOT NULL,
			rule_revision INTEGER NOT NULL,
			ledger_epoch TEXT NOT NULL,
			end_at_pub_seq INTEGER NOT NULL CHECK(end_at_pub_seq >= 0),
			PRIMARY KEY(rule_id, rule_revision, ledger_epoch)
		);
		CREATE TABLE semantic_rule_runtime_v3 (
			rule_id TEXT PRIMARY KEY,
			initialized INTEGER NOT NULL CHECK(initialized IN (0, 1)),
			detector_active INTEGER NOT NULL CHECK(detector_active IN (0, 1)),
			counter INTEGER NOT NULL CHECK(counter >= 0),
			pending INTEGER NOT NULL CHECK(pending IN (0, 1)),
			pending_active INTEGER NOT NULL CHECK(pending_active IN (0, 1)),
			pending_since INTEGER NOT NULL CHECK(pending_since >= 0),
			applied_rule_revision INTEGER NOT NULL CHECK(applied_rule_revision >= 0),
			applied_calibration_revision INTEGER NOT NULL CHECK(applied_calibration_revision >= 0),
			applied_ledger_epoch TEXT NOT NULL,
			next_sequence INTEGER NOT NULL CHECK(next_sequence > 0)
		);
		CREATE TABLE semantic_projection_receipts_v3 (
			rule_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			pub_seq INTEGER NOT NULL CHECK(pub_seq > 0),
			rule_revision INTEGER NOT NULL,
			calibration_revision INTEGER NOT NULL,
			observation_id TEXT,
			PRIMARY KEY(rule_id, ledger_epoch, pub_seq)
		);
		CREATE TABLE semantic_observations_v3 (
			observation_row_id INTEGER PRIMARY KEY AUTOINCREMENT,
			observation_id TEXT NOT NULL UNIQUE,
			rule_id TEXT NOT NULL,
			rule_revision INTEGER NOT NULL,
			calibration_revision INTEGER NOT NULL,
			series_id TEXT NOT NULL,
			sequence INTEGER NOT NULL CHECK(sequence > 0),
			kind TEXT NOT NULL CHECK(kind IN (
				'numeric', 'boolean', 'cumulative_counter', 'alarm'
			)),
			value_json BLOB NOT NULL CHECK(json_valid(value_json)),
			signal_ref TEXT NOT NULL,
			edge_node_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			source_pub_seq INTEGER NOT NULL CHECK(source_pub_seq >= 0),
			observed_at INTEGER NOT NULL CHECK(observed_at >= 0),
			created_at INTEGER NOT NULL,
			UNIQUE(rule_id, sequence)
		);
		CREATE TABLE semantic_projection_failures_v3 (
			rule_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			pub_seq INTEGER NOT NULL CHECK(pub_seq > 0),
			error_text TEXT NOT NULL,
			attempts INTEGER NOT NULL CHECK(attempts > 0),
			last_failed_at INTEGER NOT NULL,
			PRIMARY KEY(rule_id, ledger_epoch, pub_seq)
		);
		CREATE TABLE semantic_counter_resets_v3 (
			reset_id TEXT PRIMARY KEY,
			rule_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			apply_after_pub_seq INTEGER NOT NULL CHECK(apply_after_pub_seq >= 0),
			requested_at INTEGER NOT NULL,
			applied_at INTEGER,
			actor_ref TEXT NOT NULL,
			zero_observation_id TEXT
		);
		CREATE INDEX ix_semantic_counter_resets_pending_v3
			ON semantic_counter_resets_v3(rule_id, applied_at, requested_at, reset_id);
		CREATE TABLE semantic_counter_reset_boundaries_v3 (
			reset_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			apply_after_pub_seq INTEGER NOT NULL CHECK(apply_after_pub_seq >= 0),
			PRIMARY KEY(reset_id, ledger_epoch)
		);
		CREATE TABLE yokakit_routes_v3 (
			route_id TEXT PRIMARY KEY,
			rule_id TEXT NOT NULL,
			source_id TEXT NOT NULL,
			signal_id TEXT NOT NULL,
			kind TEXT NOT NULL CHECK(kind IN (
				'production', 'onoff', 'gantt_chart', 'alarm'
			)),
			reason TEXT NOT NULL,
			start_after_observation_row_id INTEGER NOT NULL CHECK(
				start_after_observation_row_id >= 0
			),
			active INTEGER NOT NULL CHECK(active IN (0, 1)),
			created_at INTEGER NOT NULL,
			UNIQUE(source_id, signal_id)
		);
		CREATE TABLE output_outbox_v3 (
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
		INSERT INTO semantic_signal_configs_v3(signal_ref, revision)
		SELECT signal_ref, 1 FROM edge_signals;
		INSERT INTO signal_calibration_revisions_v3(
			signal_ref, revision, scale, offset, active, created_at
		)
		SELECT signal_ref, 1, 1, 0, 1, created_at FROM edge_signals;
		INSERT INTO signal_calibration_starts_v3(
			signal_ref, calibration_revision, ledger_epoch, start_after_pub_seq
		)
		SELECT signal.signal_ref, 1, cursor.ledger_epoch, 0
		FROM edge_signals AS signal
		JOIN accepted_cursors AS cursor
			ON cursor.edge_node_id = signal.edge_node_id;
	`},
	{version: 15, sql: `
		CREATE TABLE output_routes (
			route_id TEXT PRIMARY KEY,
			rule_id TEXT NOT NULL,
			adapter_id TEXT NOT NULL,
			config_schema_version INTEGER NOT NULL CHECK(
				config_schema_version >= 1
			),
			config_json BLOB NOT NULL CHECK(
				json_valid(config_json) AND json_type(config_json) = 'object'
			),
			start_after_observation_row_id INTEGER NOT NULL CHECK(
				start_after_observation_row_id >= 0
			),
			active INTEGER NOT NULL CHECK(active IN (0, 1)),
			created_at INTEGER NOT NULL
		);
		INSERT INTO output_routes(
			route_id, rule_id, adapter_id, config_schema_version,
			config_json, start_after_observation_row_id, active, created_at
		)
		SELECT route_id, rule_id, 'yokakit.mqtt.v1', 1,
			json_object(
				'schema_version', 1,
				'source_id', source_id,
				'signal_id', signal_id,
				'kind', kind,
				'reason', reason
			),
			start_after_observation_row_id, active, created_at
		FROM yokakit_routes_v3;
		CREATE INDEX ix_output_routes_rule
			ON output_routes(rule_id, active);
		CREATE UNIQUE INDEX ux_output_routes_yokakit_identity
			ON output_routes(
				json_extract(config_json, '$.source_id'),
				json_extract(config_json, '$.signal_id')
			)
			WHERE adapter_id = 'yokakit.mqtt.v1';
		DROP TABLE yokakit_routes_v3;
	`},
	{version: 16, sql: `
		ALTER TABLE descriptor_devices ADD COLUMN model_id TEXT;
	`},
	{version: 17, sql: `
		ALTER TABLE output_routes ADD COLUMN last_transform_error_code TEXT
			CHECK(last_transform_error_code IS NULL OR last_transform_error_code IN (
				'adapter_unavailable',
				'config_version_mismatch',
				'invalid_observation',
				'transform_failed'
			));
		ALTER TABLE output_routes ADD COLUMN last_transform_error_at INTEGER
			CHECK(last_transform_error_at IS NULL OR last_transform_error_at >= 0);
		ALTER TABLE output_routes ADD COLUMN last_transform_success_at INTEGER
			CHECK(last_transform_success_at IS NULL OR last_transform_success_at >= 0);
	`},
	{version: 18, sql: `
		CREATE TABLE export_profiles (
			profile_id TEXT PRIMARY KEY,
			display_name TEXT NOT NULL,
			adapter_id TEXT NOT NULL,
			adapter_schema_version INTEGER NOT NULL CHECK(adapter_schema_version > 0),
			profile_config_json BLOB NOT NULL CHECK(
				json_valid(profile_config_json) AND
				json_type(profile_config_json) = 'object'
			),
			state TEXT NOT NULL CHECK(state IN (
				'preparing', 'active', 'draining', 'stopped'
			)),
			auto_bind_future_rules INTEGER NOT NULL CHECK(
				auto_bind_future_rules IN (0, 1)
			),
			revision INTEGER NOT NULL CHECK(revision > 0),
			created_at INTEGER NOT NULL CHECK(created_at >= 0),
			drain_requested_at INTEGER CHECK(
				drain_requested_at IS NULL OR drain_requested_at >= 0
			),
			stopped_at INTEGER CHECK(stopped_at IS NULL OR stopped_at >= 0)
		);
		CREATE UNIQUE INDEX ux_export_profiles_active_adapter
			ON export_profiles(adapter_id)
			WHERE state IN ('preparing', 'active', 'draining');

		CREATE TABLE output_profile_rule_bindings (
			binding_id TEXT PRIMARY KEY,
			profile_id TEXT NOT NULL,
			rule_id TEXT NOT NULL,
			source_id TEXT NOT NULL,
			signal_id TEXT,
			mode TEXT,
			reason TEXT NOT NULL DEFAULT '',
			state TEXT NOT NULL CHECK(state IN (
				'needs_configuration', 'prepared', 'active', 'ineligible',
				'draining', 'stopped'
			)),
			ineligible_reason TEXT NOT NULL DEFAULT '',
			revision INTEGER NOT NULL CHECK(revision > 0),
			created_at INTEGER NOT NULL CHECK(created_at >= 0),
			activated_at INTEGER CHECK(
				activated_at IS NULL OR activated_at >= 0
			),
			stopped_at INTEGER CHECK(stopped_at IS NULL OR stopped_at >= 0),
			UNIQUE(profile_id, rule_id, mode)
		);
		CREATE UNIQUE INDEX ux_output_binding_identity
			ON output_profile_rule_bindings(source_id, signal_id)
			WHERE signal_id IS NOT NULL;
		CREATE INDEX ix_output_bindings_profile_state
			ON output_profile_rule_bindings(profile_id, state, rule_id);

		CREATE TABLE output_binding_starts (
			binding_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			start_after_pub_seq INTEGER NOT NULL CHECK(start_after_pub_seq >= 0),
			PRIMARY KEY(binding_id, ledger_epoch)
		);
		CREATE TABLE output_binding_ends (
			binding_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			end_at_pub_seq INTEGER NOT NULL CHECK(end_at_pub_seq >= 0),
			PRIMARY KEY(binding_id, ledger_epoch)
		);

		ALTER TABLE output_routes ADD COLUMN binding_id TEXT;
		ALTER TABLE output_routes ADD COLUMN lifecycle_state TEXT NOT NULL
			DEFAULT 'active' CHECK(lifecycle_state IN (
				'active', 'draining', 'stopped'
			));
		CREATE UNIQUE INDEX ux_output_routes_binding
			ON output_routes(binding_id) WHERE binding_id IS NOT NULL;
	`},
	{version: 19, sql: `
		ALTER TABLE semantic_rule_revisions_v3
			ADD COLUMN series_id TEXT NOT NULL DEFAULT '';
		UPDATE semantic_rule_revisions_v3
		SET series_id = (
			SELECT rule.series_id FROM semantic_rules_v3 AS rule
			WHERE rule.rule_id = semantic_rule_revisions_v3.rule_id
		);
		ALTER TABLE semantic_rule_runtime_v3
			ADD COLUMN applied_series_id TEXT NOT NULL DEFAULT '';
	`},
	{version: 20, sql: `
		ALTER TABLE semantic_observations_v3
			RENAME TO semantic_observations_v3_rule_sequence;
		CREATE TABLE semantic_observations_v3 (
			observation_row_id INTEGER PRIMARY KEY AUTOINCREMENT,
			observation_id TEXT NOT NULL UNIQUE,
			rule_id TEXT NOT NULL,
			rule_revision INTEGER NOT NULL,
			calibration_revision INTEGER NOT NULL,
			series_id TEXT NOT NULL,
			sequence INTEGER NOT NULL CHECK(sequence > 0),
			kind TEXT NOT NULL CHECK(kind IN (
				'numeric', 'boolean', 'cumulative_counter', 'alarm'
			)),
			value_json BLOB NOT NULL CHECK(json_valid(value_json)),
			signal_ref TEXT NOT NULL,
			edge_node_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			source_pub_seq INTEGER NOT NULL CHECK(source_pub_seq >= 0),
			observed_at INTEGER NOT NULL CHECK(observed_at >= 0),
			created_at INTEGER NOT NULL,
			UNIQUE(series_id, sequence)
		);
		INSERT INTO semantic_observations_v3(
			observation_row_id, observation_id, rule_id, rule_revision,
			calibration_revision, series_id, sequence, kind, value_json,
			signal_ref, edge_node_id, ledger_epoch, source_pub_seq,
			observed_at, created_at
		)
		SELECT observation_row_id, observation_id, rule_id, rule_revision,
			calibration_revision, series_id, sequence, kind, value_json,
			signal_ref, edge_node_id, ledger_epoch, source_pub_seq,
			observed_at, created_at
		FROM semantic_observations_v3_rule_sequence;
		DROP TABLE semantic_observations_v3_rule_sequence;
		UPDATE semantic_rule_runtime_v3 AS runtime
		SET applied_series_id = COALESCE((
			SELECT revision.series_id
			FROM semantic_rule_revisions_v3 AS revision
			WHERE revision.rule_id = runtime.rule_id
				AND revision.revision = runtime.applied_rule_revision
		), (
			SELECT revision.series_id
			FROM semantic_rule_revisions_v3 AS revision
			WHERE revision.rule_id = runtime.rule_id
				AND revision.active = 1
		), '');
	`},
	{version: 21, sql: `
		CREATE INDEX IF NOT EXISTS ix_semantic_observations_rule_row
			ON semantic_observations_v3(rule_id, observation_row_id);
		CREATE INDEX IF NOT EXISTS ix_semantic_observations_rule_source_cursor
			ON semantic_observations_v3(
				rule_id, ledger_epoch, source_pub_seq
			);
	`},
	{version: 22, sql: `
		ALTER TABLE output_profile_rule_bindings
			RENAME TO output_profile_rule_bindings_without_prepared;
		CREATE TABLE output_profile_rule_bindings (
			binding_id TEXT PRIMARY KEY,
			profile_id TEXT NOT NULL,
			rule_id TEXT NOT NULL,
			source_id TEXT NOT NULL,
			signal_id TEXT,
			mode TEXT,
			reason TEXT NOT NULL DEFAULT '',
			state TEXT NOT NULL CHECK(state IN (
				'needs_configuration', 'prepared', 'active', 'ineligible',
				'draining', 'stopped'
			)),
			ineligible_reason TEXT NOT NULL DEFAULT '',
			revision INTEGER NOT NULL CHECK(revision > 0),
			created_at INTEGER NOT NULL CHECK(created_at >= 0),
			activated_at INTEGER CHECK(
				activated_at IS NULL OR activated_at >= 0
			),
			stopped_at INTEGER CHECK(
				stopped_at IS NULL OR stopped_at >= 0
			),
			UNIQUE(profile_id, rule_id, mode)
		);
		INSERT INTO output_profile_rule_bindings(
			binding_id, profile_id, rule_id, source_id, signal_id,
			mode, reason, state, ineligible_reason, revision,
			created_at, activated_at, stopped_at
		)
		SELECT binding_id, profile_id, rule_id, source_id, signal_id,
			mode, reason, state, ineligible_reason, revision,
			created_at, activated_at, stopped_at
		FROM output_profile_rule_bindings_without_prepared;
		DROP TABLE output_profile_rule_bindings_without_prepared;
		CREATE UNIQUE INDEX ux_output_binding_identity
			ON output_profile_rule_bindings(source_id, signal_id)
			WHERE signal_id IS NOT NULL;
		CREATE INDEX ix_output_bindings_profile_state
			ON output_profile_rule_bindings(profile_id, state, rule_id);
	`},
	{version: 23, sql: `
		ALTER TABLE export_profiles
			RENAME TO export_profiles_without_preparing;
		CREATE TABLE export_profiles (
			profile_id TEXT PRIMARY KEY,
			display_name TEXT NOT NULL,
			adapter_id TEXT NOT NULL,
			adapter_schema_version INTEGER NOT NULL CHECK(
				adapter_schema_version > 0
			),
			profile_config_json BLOB NOT NULL CHECK(
				json_valid(profile_config_json) AND
				json_type(profile_config_json) = 'object'
			),
			state TEXT NOT NULL CHECK(state IN (
				'preparing', 'active', 'draining', 'stopped'
			)),
			auto_bind_future_rules INTEGER NOT NULL CHECK(
				auto_bind_future_rules IN (0, 1)
			),
			revision INTEGER NOT NULL CHECK(revision > 0),
			created_at INTEGER NOT NULL CHECK(created_at >= 0),
			drain_requested_at INTEGER CHECK(
				drain_requested_at IS NULL OR drain_requested_at >= 0
			),
			stopped_at INTEGER CHECK(
				stopped_at IS NULL OR stopped_at >= 0
			)
		);
		INSERT INTO export_profiles(
			profile_id, display_name, adapter_id,
			adapter_schema_version, profile_config_json, state,
			auto_bind_future_rules, revision, created_at,
			drain_requested_at, stopped_at
		)
		SELECT profile_id, display_name, adapter_id,
			adapter_schema_version, profile_config_json, state,
			auto_bind_future_rules, revision, created_at,
			drain_requested_at, stopped_at
		FROM export_profiles_without_preparing;
		DROP TABLE export_profiles_without_preparing;
		CREATE UNIQUE INDEX ux_export_profiles_active_adapter
			ON export_profiles(adapter_id)
			WHERE state IN ('preparing', 'active', 'draining');
	`},
	{version: 24, sql: `
		UPDATE output_routes
		SET active = 0, lifecycle_state = 'stopped'
		WHERE binding_id IS NULL;
	`},
	{version: 25, sql: `
		CREATE TABLE output_signal_identities (
			output_identity_id TEXT PRIMARY KEY,
			adapter_id TEXT NOT NULL,
			rule_id TEXT NOT NULL,
			mode TEXT NOT NULL,
			source_id TEXT NOT NULL,
			signal_id TEXT NOT NULL,
			created_at INTEGER NOT NULL CHECK(created_at >= 0),
			UNIQUE(adapter_id, rule_id, mode),
			UNIQUE(source_id, signal_id)
		);
		WITH ranked_identity AS (
			SELECT profile.adapter_id, binding.rule_id, binding.mode,
				binding.source_id, binding.signal_id, binding.created_at,
				ROW_NUMBER() OVER (
					PARTITION BY profile.adapter_id, binding.rule_id, binding.mode
					ORDER BY
						CASE WHEN profile.state IN (
							'preparing', 'active', 'draining'
						) AND binding.state IN (
							'prepared', 'active', 'draining'
						) THEN 0 ELSE 1 END,
						binding.created_at DESC,
						binding.binding_id DESC
				) AS identity_rank
			FROM output_profile_rule_bindings AS binding
			JOIN export_profiles AS profile
				ON profile.profile_id = binding.profile_id
			WHERE binding.signal_id IS NOT NULL
				AND binding.mode IS NOT NULL
		)
		INSERT INTO output_signal_identities(
			output_identity_id, adapter_id, rule_id, mode,
			source_id, signal_id, created_at
		)
		SELECT 'osi_' || lower(hex(randomblob(16))),
			adapter_id, rule_id, mode, source_id, signal_id, created_at
		FROM ranked_identity
		WHERE identity_rank = 1;

		ALTER TABLE output_profile_rule_bindings
			RENAME TO output_profile_rule_bindings_with_embedded_identity;
		CREATE TABLE output_profile_rule_bindings (
			binding_id TEXT PRIMARY KEY,
			profile_id TEXT NOT NULL,
			rule_id TEXT NOT NULL,
			output_identity_id TEXT,
			reason TEXT NOT NULL DEFAULT '',
			state TEXT NOT NULL CHECK(state IN (
				'needs_configuration', 'prepared', 'active', 'ineligible',
				'draining', 'stopped'
			)),
			ineligible_reason TEXT NOT NULL DEFAULT '',
			revision INTEGER NOT NULL CHECK(revision > 0),
			created_at INTEGER NOT NULL CHECK(created_at >= 0),
			activated_at INTEGER CHECK(
				activated_at IS NULL OR activated_at >= 0
			),
			stopped_at INTEGER CHECK(
				stopped_at IS NULL OR stopped_at >= 0
			),
			UNIQUE(profile_id, rule_id)
		);
		INSERT INTO output_profile_rule_bindings(
			binding_id, profile_id, rule_id, output_identity_id,
			reason, state, ineligible_reason, revision,
			created_at, activated_at, stopped_at
		)
		SELECT binding.binding_id, binding.profile_id, binding.rule_id,
			identity.output_identity_id, binding.reason, binding.state,
			binding.ineligible_reason, binding.revision,
			binding.created_at, binding.activated_at, binding.stopped_at
		FROM output_profile_rule_bindings_with_embedded_identity AS binding
		JOIN export_profiles AS profile
			ON profile.profile_id = binding.profile_id
		LEFT JOIN output_signal_identities AS identity
			ON identity.adapter_id = profile.adapter_id
			AND identity.rule_id = binding.rule_id
			AND identity.mode = binding.mode;
		DROP TABLE output_profile_rule_bindings_with_embedded_identity;
		CREATE INDEX ix_output_bindings_profile_state
			ON output_profile_rule_bindings(profile_id, state, rule_id);
		DROP INDEX IF EXISTS ux_output_routes_yokakit_identity;
	`},
	{version: 26, sql: `
		CREATE INDEX idx_raw_records_history_received
		ON raw_records(
			received_at DESC,
			edge_node_id DESC,
			ledger_epoch DESC,
			pub_seq DESC
		);
	`},
	{version: 27, sql: `
		CREATE TABLE edge_backup_events (
			backup_id TEXT PRIMARY KEY,
			created_at INTEGER NOT NULL CHECK(created_at >= 0),
			destination_name TEXT NOT NULL,
			database_sha256 TEXT NOT NULL CHECK(length(database_sha256) = 64),
			raw_record_count INTEGER NOT NULL CHECK(raw_record_count >= 0)
		);
		CREATE TABLE edge_backup_cursors (
			backup_id TEXT NOT NULL REFERENCES edge_backup_events(backup_id),
			edge_node_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			accepted_through INTEGER NOT NULL CHECK(accepted_through >= 0),
			PRIMARY KEY(backup_id, edge_node_id, ledger_epoch)
		);
		CREATE TABLE edge_restore_events (
			restore_id TEXT PRIMARY KEY,
			backup_id TEXT,
			restored_at INTEGER NOT NULL CHECK(restored_at >= 0),
			backup_created_at INTEGER NOT NULL CHECK(backup_created_at >= 0),
			backup_edge_id TEXT NOT NULL,
			backup_schema_version INTEGER NOT NULL CHECK(backup_schema_version > 0),
			backup_sha256 TEXT NOT NULL CHECK(length(backup_sha256) = 64)
		);
		CREATE TABLE edge_restore_cursor_checks (
			restore_id TEXT NOT NULL REFERENCES edge_restore_events(restore_id),
			edge_node_id TEXT NOT NULL,
			ledger_epoch TEXT NOT NULL,
			backup_accepted_through INTEGER NOT NULL CHECK(backup_accepted_through >= 0),
			state TEXT NOT NULL CHECK(state IN (
				'pending', 'verified', 'recovery_required', 'archive_lost'
			)),
			observed_cursor_start INTEGER CHECK(observed_cursor_start > 0),
			updated_at INTEGER NOT NULL CHECK(updated_at >= 0),
			PRIMARY KEY (restore_id, edge_node_id, ledger_epoch)
		);
		CREATE INDEX ix_edge_restore_cursor_checks_pending
			ON edge_restore_cursor_checks(edge_node_id, ledger_epoch, state);
	`},
	{version: 28, sql: `
		CREATE INDEX idx_semantic_observations_history
		ON semantic_observations_v3(
			observed_at DESC,
			observation_row_id DESC
		);
	`},
	{version: 29, sql: `
		CREATE TABLE edge_storage_samples (
			sampled_at INTEGER PRIMARY KEY,
			database_bytes INTEGER NOT NULL CHECK(database_bytes >= 0),
			raw_record_count INTEGER NOT NULL CHECK(raw_record_count >= 0)
		);
	`},
	{version: 30, sql: `
		CREATE TABLE output_sensor_identities (
			output_sensor_identity_id TEXT PRIMARY KEY,
			signal_ref TEXT NOT NULL,
			source_id TEXT NOT NULL,
			sensor_id TEXT NOT NULL,
			created_at INTEGER NOT NULL CHECK(created_at >= 0),
			registered_at INTEGER CHECK(
				registered_at IS NULL OR registered_at >= 0
			),
			UNIQUE(signal_ref),
			UNIQUE(source_id, sensor_id)
		);
		ALTER TABLE output_profile_rule_bindings
			ADD COLUMN output_sensor_identity_id TEXT;
		ALTER TABLE output_profile_rule_bindings
			ADD COLUMN mode TEXT;
		UPDATE output_profile_rule_bindings AS binding
		SET mode = (
			SELECT identity.mode FROM output_signal_identities AS identity
			WHERE identity.output_identity_id = binding.output_identity_id
		);

		INSERT INTO output_sensor_identities(
			output_sensor_identity_id, signal_ref, source_id,
			sensor_id, created_at, registered_at
		)
		SELECT 'osy_' || lower(hex(randomblob(16))), rule.signal_ref,
			identity.source_id, 'sen-' || lower(hex(randomblob(16))),
			MIN(binding.created_at), NULL
		FROM output_profile_rule_bindings AS binding
		JOIN export_profiles AS profile ON profile.profile_id = binding.profile_id
		JOIN semantic_rules_v3 AS rule ON rule.rule_id = binding.rule_id
		JOIN output_signal_identities AS identity
			ON identity.output_identity_id = binding.output_identity_id
		WHERE profile.adapter_id = 'yokakit.mqtt.v1'
		GROUP BY rule.signal_ref, identity.source_id;

		UPDATE output_profile_rule_bindings AS binding
		SET output_sensor_identity_id = (
			SELECT sensor.output_sensor_identity_id
			FROM semantic_rules_v3 AS rule
			JOIN output_sensor_identities AS sensor
				ON sensor.signal_ref = rule.signal_ref
			WHERE rule.rule_id = binding.rule_id
		)
		WHERE EXISTS (
			SELECT 1 FROM export_profiles AS profile
			WHERE profile.profile_id = binding.profile_id
				AND profile.adapter_id = 'yokakit.mqtt.v1'
		);

		DELETE FROM output_binding_starts
		WHERE binding_id IN (
			SELECT binding.binding_id
			FROM output_profile_rule_bindings AS binding
			JOIN export_profiles AS profile
				ON profile.profile_id = binding.profile_id
			WHERE profile.adapter_id = 'yokakit.mqtt.v1'
				AND profile.state IN ('preparing', 'active')
				AND binding.state IN ('prepared', 'active')
		);
		UPDATE output_routes
		SET active = 0
		WHERE binding_id IN (
			SELECT binding.binding_id
			FROM output_profile_rule_bindings AS binding
			JOIN export_profiles AS profile
				ON profile.profile_id = binding.profile_id
			WHERE profile.adapter_id = 'yokakit.mqtt.v1'
				AND profile.state IN ('preparing', 'active')
				AND binding.state IN ('prepared', 'active')
		);
		UPDATE output_profile_rule_bindings AS binding
		SET state = 'prepared', activated_at = NULL, revision = revision + 1
		WHERE binding.state = 'active'
			AND EXISTS (
				SELECT 1 FROM export_profiles AS profile
				WHERE profile.profile_id = binding.profile_id
					AND profile.adapter_id = 'yokakit.mqtt.v1'
					AND profile.state IN ('preparing', 'active')
			);
		UPDATE export_profiles
		SET state = 'preparing', revision = revision + 1
		WHERE adapter_id = 'yokakit.mqtt.v1' AND state = 'active';

		UPDATE output_routes AS route
		SET config_json = CAST(json_object(
			'schema_version', 1,
			'source_id', (
				SELECT sensor.source_id
				FROM output_profile_rule_bindings AS binding
				JOIN output_sensor_identities AS sensor
					ON sensor.output_sensor_identity_id = binding.output_sensor_identity_id
				WHERE binding.binding_id = route.binding_id
			),
			'sensor_id', (
				SELECT sensor.sensor_id
				FROM output_profile_rule_bindings AS binding
				JOIN output_sensor_identities AS sensor
					ON sensor.output_sensor_identity_id = binding.output_sensor_identity_id
				WHERE binding.binding_id = route.binding_id
			),
			'kind', json_extract(route.config_json, '$.kind'),
			'reason', COALESCE(json_extract(route.config_json, '$.reason'), '')
		) AS BLOB)
		WHERE route.adapter_id = 'yokakit.mqtt.v1'
			AND route.binding_id IS NOT NULL;

		UPDATE output_profile_rule_bindings AS binding
		SET output_identity_id = NULL
		WHERE EXISTS (
			SELECT 1 FROM export_profiles AS profile
			WHERE profile.profile_id = binding.profile_id
				AND profile.adapter_id = 'yokakit.mqtt.v1'
		);
		DELETE FROM output_signal_identities
		WHERE adapter_id = 'yokakit.mqtt.v1';
	`},
	{version: 31, sql: `
		UPDATE export_profiles
		SET adapter_id = 'pinikiet.mqtt.v1',
			display_name = CASE
				WHEN display_name = 'YokaKit' THEN 'Pinikiet'
				ELSE display_name
			END
		WHERE adapter_id = 'yokakit.mqtt.v1';
		UPDATE output_routes
		SET adapter_id = 'pinikiet.mqtt.v1'
		WHERE adapter_id = 'yokakit.mqtt.v1';
		UPDATE output_signal_identities
		SET adapter_id = 'pinikiet.mqtt.v1'
		WHERE adapter_id = 'yokakit.mqtt.v1';
	`},
}

func applyMigrations(
	ctx context.Context,
	db *sql.DB,
	configuredEdgeID string,
) error {
	var current int
	if err := db.QueryRowContext(ctx, "PRAGMA user_version").Scan(&current); err != nil {
		return fmt.Errorf("read Edge schema version: %w", err)
	}
	latest := schemaMigrations[len(schemaMigrations)-1].version
	if current > latest {
		return fmt.Errorf("Edge schema version %d is newer than supported version %d", current, latest)
	}

	for _, migration := range schemaMigrations {
		if migration.version <= current {
			continue
		}
		tx, err := db.BeginTx(ctx, nil)
		if err != nil {
			return fmt.Errorf("begin Edge schema migration %d: %w", migration.version, err)
		}
		if _, err := tx.ExecContext(ctx, migration.sql); err != nil {
			_ = tx.Rollback()
			return fmt.Errorf("apply Edge schema migration %d: %w", migration.version, err)
		}
		if migration.version == 11 && configuredEdgeID != "" {
			if _, err := tx.ExecContext(
				ctx,
				"UPDATE edge_meta SET edge_id = ? WHERE singleton = 1",
				configuredEdgeID,
			); err != nil {
				_ = tx.Rollback()
				return fmt.Errorf(
					"assign configured Edge identity in migration 11: %w",
					err,
				)
			}
		}
		if _, err := tx.ExecContext(ctx, fmt.Sprintf("PRAGMA user_version = %d", migration.version)); err != nil {
			_ = tx.Rollback()
			return fmt.Errorf("record Edge schema migration %d: %w", migration.version, err)
		}
		if err := tx.Commit(); err != nil {
			return fmt.Errorf("commit Edge schema migration %d: %w", migration.version, err)
		}
		current = migration.version
	}
	return nil
}
