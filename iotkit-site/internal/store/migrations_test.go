package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"path/filepath"
	"testing"
)

func TestOpenMigratesRealVersionThreeDatabaseWithoutDroppingData(t *testing.T) {
	path := filepath.Join(t.TempDir(), "site.db")
	db, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	for _, migration := range schemaMigrations[:3] {
		if _, err := db.Exec(migration.sql); err != nil {
			t.Fatal(err)
		}
		if _, err := db.Exec(fmt.Sprintf("PRAGMA user_version = %d", migration.version)); err != nil {
			t.Fatal(err)
		}
	}
	const systemID = "018f0000-0000-7000-8000-000000000001"
	const seriesKey = systemID + ":temperature:na:primary"
	if _, err := db.Exec(`
		INSERT INTO descriptor_devices (
			edge_node_id, system_id, identifier, state, presence,
			descriptor_revision, updated_at
		) VALUES ('edge-node-01', ?, '01234567', 'active', 'current', 1, 1000)
	`, systemID); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`
		INSERT INTO descriptor_signals (
			edge_node_id, series_key, system_id, measurement_key, channel_index,
			variant, unit, value_type, presence, descriptor_revision, updated_at
		) VALUES ('edge-node-01', ?, ?, 'temperature', NULL,
			'primary', 'C', 'float', 'current', 1, 1000)
	`, seriesKey, systemID); err != nil {
		t.Fatal(err)
	}
	recordJSON := fmt.Sprintf(
		`{"family":"measurement","schema_version":1,"epoch":"epoch-01","pub_seq":1,"series_key":%q,"values":[21.5],"event_time":1500}`,
		seriesKey,
	)
	if _, err := db.Exec(`
		INSERT INTO raw_records (
			edge_node_id, ledger_epoch, pub_seq, publication_id,
			record_json, record_sha256, received_at
		) VALUES ('edge-node-01', 'epoch-01', 1, 'publication-01', ?, zeroblob(32), 2000)
	`, recordJSON); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`
		INSERT INTO accepted_cursors (
			edge_node_id, ledger_epoch, accepted_through, updated_at
		) VALUES ('edge-node-01', 'epoch-01', 1, 2000)
	`); err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}

	store, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = store.Close() })
	var version int
	if err := store.db.QueryRow("PRAGMA user_version").Scan(&version); err != nil {
		t.Fatal(err)
	}
	if version != 15 {
		t.Fatalf("schema version = %d, want 15", version)
	}
	if got := testTableCount(t, store.db, "site_devices"); got != 1 {
		t.Fatalf("backfilled devices = %d, want 1", got)
	}
	if got := testTableCount(t, store.db, "site_signals"); got != 1 {
		t.Fatalf("backfilled signals = %d, want 1", got)
	}
	records, err := store.ListRawRecords(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != 1 {
		t.Fatalf("raw records = %d, want 1", len(records))
	}
	if processed, err := store.ReconcileInventorySources(context.Background(), 10); err != nil || processed != 1 {
		t.Fatalf("post-migration projection processed = %d, err = %v", processed, err)
	}
	signals, err := store.ListInventorySignals(context.Background(), 10, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(signals) != 1 || signals[0].Latest == nil || signals[0].Latest.EventTime != 1500 {
		t.Fatalf("migrated signals = %#v", signals)
	}
}

func TestMigrationFifteenCreatesGenericOutputSchemaWithoutDroppingV2(t *testing.T) {
	store := openTestStore(t)
	for _, table := range []string{
		"signal_calibration_revisions_v3",
		"signal_calibration_starts_v3",
		"semantic_rules_v3",
		"semantic_rule_revisions_v3",
		"semantic_rule_starts_v3",
		"semantic_rule_ends_v3",
		"semantic_rule_runtime_v3",
		"semantic_projection_receipts_v3",
		"semantic_observations_v3",
		"semantic_projection_failures_v3",
		"semantic_counter_resets_v3",
		"semantic_counter_reset_boundaries_v3",
		"output_routes",
		"output_outbox_v3",
		"semantic_definitions_v2",
	} {
		var exists int
		if err := store.db.QueryRow(`
			SELECT EXISTS(
				SELECT 1 FROM sqlite_master
				WHERE type = 'table' AND name = ?
			)
		`, table).Scan(&exists); err != nil {
			t.Fatal(err)
		}
		if exists != 1 {
			t.Fatalf("table %s was not created", table)
		}
	}

	var uniqueSignalRuleIndex int
	if err := store.db.QueryRow(`
		SELECT count(*) FROM sqlite_master
		WHERE type = 'index'
			AND tbl_name = 'semantic_rules_v3'
			AND sql LIKE '%ON semantic_rules_v3(signal_ref) WHERE retired_at IS NULL%'
	`).Scan(&uniqueSignalRuleIndex); err != nil {
		t.Fatal(err)
	}
	if uniqueSignalRuleIndex != 0 {
		t.Fatal("v3 schema still limits a signal to one active rule")
	}
}

func TestMigrationFifteenConvertsYokaKitRoutesToGenericOutputRoutes(t *testing.T) {
	path := filepath.Join(t.TempDir(), "site.db")
	db, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	applyTestMigrationsThrough(t, db, 14)
	if _, err := db.Exec(`
		INSERT INTO yokakit_routes_v3(
			route_id, rule_id, source_id, signal_id, kind, reason,
			start_after_observation_row_id, active, created_at
		) VALUES(
			'out_0123456789abcdef0123456789abcdef',
			'rule_0123456789abcdef0123456789abcdef',
			'line-a', 'production', 'production', '', 42, 1, 1000
		)
	`); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`
		INSERT INTO output_outbox_v3(
			export_id, route_id, observation_id, topic, qos,
			payload_json, attempts, created_at
		) VALUES(
			'export-01',
			'out_0123456789abcdef0123456789abcdef',
			'observation-01',
			'yokakit/v1/sources/line-a/signals/production/observations',
			1, '{"schema_version":1}', 2, 1100
		)
	`); err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}

	archive, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = archive.Close() })

	var (
		adapterID    string
		configSchema int
		configJSON   string
		startAfter   int64
	)
	if err := archive.db.QueryRow(`
		SELECT adapter_id, config_schema_version, CAST(config_json AS TEXT),
			start_after_observation_row_id
		FROM output_routes
		WHERE route_id = 'out_0123456789abcdef0123456789abcdef'
	`).Scan(&adapterID, &configSchema, &configJSON, &startAfter); err != nil {
		t.Fatal(err)
	}
	if adapterID != "yokakit.mqtt.v1" || configSchema != 1 ||
		startAfter != 42 {
		t.Fatalf(
			"adapter=%q schema=%d start=%d config=%s",
			adapterID,
			configSchema,
			startAfter,
			configJSON,
		)
	}
	var config struct {
		SchemaVersion int    `json:"schema_version"`
		SourceID      string `json:"source_id"`
		SignalID      string `json:"signal_id"`
		Kind          string `json:"kind"`
	}
	if err := json.Unmarshal([]byte(configJSON), &config); err != nil {
		t.Fatal(err)
	}
	if config.SchemaVersion != 1 || config.SourceID != "line-a" ||
		config.SignalID != "production" || config.Kind != "production" {
		t.Fatalf("config = %#v", config)
	}
	var outboxRoute string
	var outboxAttempts int
	if err := archive.db.QueryRow(`
		SELECT route_id, attempts FROM output_outbox_v3
		WHERE export_id = 'export-01'
	`).Scan(&outboxRoute, &outboxAttempts); err != nil {
		t.Fatal(err)
	}
	if outboxRoute != "out_0123456789abcdef0123456789abcdef" ||
		outboxAttempts != 2 {
		t.Fatalf("outbox route=%q attempts=%d", outboxRoute, outboxAttempts)
	}
}

func TestMigrationFifteenRollsBackBeforeDroppingYokaKitRoutes(t *testing.T) {
	path := filepath.Join(t.TempDir(), "site.db")
	db, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	applyTestMigrationsThrough(t, db, 14)
	if _, err := db.Exec(`
		INSERT INTO yokakit_routes_v3(
			route_id, rule_id, source_id, signal_id, kind, reason,
			start_after_observation_row_id, active, created_at
		) VALUES(
			'out_0123456789abcdef0123456789abcdef',
			'rule_0123456789abcdef0123456789abcdef',
			'line-a', 'production', 'production', '', 0, 1, 1000
		);
		CREATE TABLE output_routes(conflict TEXT);
	`); err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}

	if archive, err := Open(path); err == nil {
		_ = archive.Close()
		t.Fatal("migration succeeded despite conflicting output_routes table")
	}
	db, err = sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = db.Close() })
	var version, routeCount int
	if err := db.QueryRow("PRAGMA user_version").Scan(&version); err != nil {
		t.Fatal(err)
	}
	if err := db.QueryRow(`
		SELECT count(*) FROM yokakit_routes_v3
	`).Scan(&routeCount); err != nil {
		t.Fatal(err)
	}
	if version != 14 || routeCount != 1 {
		t.Fatalf("version=%d route count=%d", version, routeCount)
	}
}

func TestMigrationFourteenPreservesV2SemanticAndOutputData(t *testing.T) {
	path := filepath.Join(t.TempDir(), "site.db")
	db, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	applyTestMigrationsThrough(t, db, 13)
	if _, err := db.Exec(`
		INSERT INTO semantic_definitions_v2(
			definition_id, revision, signal_ref, edge_node_id, series_key,
			series_id, spec_json, active, created_at
		) VALUES(
			'legacy-definition', 2, 'legacy-signal', 'legacy-edge',
			'legacy-series', 'legacy-semantic-series',
			'{"kind":"numeric"}', 1, 1000
		);
		INSERT INTO semantic_observations_v2(
			observation_id, series_id, sequence, definition_id,
			definition_revision, kind, value_json, signal_ref,
			edge_node_id, ledger_epoch, source_pub_seq, observed_at, created_at
		) VALUES(
			'legacy-observation', 'legacy-semantic-series', 7,
			'legacy-definition', 2, 'numeric', '12.5', 'legacy-signal',
			'legacy-edge', 'legacy-epoch', 9, 2000, 2100
		);
		INSERT INTO yokakit_routes(
			route_id, definition_id, source_id, signal_id, kind, reason,
			start_after_observation_row_id, active, created_at
		) VALUES(
			'legacy-route', 'legacy-definition', 'legacy-source',
			'legacy-channel', 'production', '', 0, 1, 3000
		);
		INSERT INTO output_outbox_v2(
			export_id, route_id, observation_id, topic, qos,
			payload_json, attempts, created_at
		) VALUES(
			'legacy-export', 'legacy-route', 'legacy-observation',
			'yokakit/v1/sources/legacy-source/channels/legacy-channel',
			1, '{"value":12.5}', 3, 4000
		);
	`); err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}

	archive, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = archive.Close() })

	var (
		definitionRevision int64
		observationValue   string
		routeSignalID      string
		outboxAttempts     int
	)
	if err := archive.db.QueryRow(`
		SELECT revision FROM semantic_definitions_v2
		WHERE definition_id = 'legacy-definition' AND active = 1
	`).Scan(&definitionRevision); err != nil {
		t.Fatal(err)
	}
	if err := archive.db.QueryRow(`
		SELECT CAST(value_json AS TEXT) FROM semantic_observations_v2
		WHERE observation_id = 'legacy-observation'
	`).Scan(&observationValue); err != nil {
		t.Fatal(err)
	}
	if err := archive.db.QueryRow(`
		SELECT signal_id FROM yokakit_routes WHERE route_id = 'legacy-route'
	`).Scan(&routeSignalID); err != nil {
		t.Fatal(err)
	}
	if err := archive.db.QueryRow(`
		SELECT attempts FROM output_outbox_v2 WHERE export_id = 'legacy-export'
	`).Scan(&outboxAttempts); err != nil {
		t.Fatal(err)
	}
	if definitionRevision != 2 || observationValue != "12.5" ||
		routeSignalID != "legacy-channel" || outboxAttempts != 3 {
		t.Fatalf(
			"v2 data changed: revision=%d value=%q signal=%q attempts=%d",
			definitionRevision,
			observationValue,
			routeSignalID,
			outboxAttempts,
		)
	}
}

func TestMigrationFourteenRollsBackCompletelyOnFailure(t *testing.T) {
	path := filepath.Join(t.TempDir(), "site.db")
	db, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	applyTestMigrationsThrough(t, db, 13)
	if _, err := db.Exec(`
		INSERT INTO semantic_definitions_v2(
			definition_id, revision, signal_ref, edge_node_id, series_key,
			series_id, spec_json, active, created_at
		) VALUES(
			'legacy-definition', 1, 'legacy-signal', 'legacy-edge',
			'legacy-series', 'legacy-semantic-series',
			'{"kind":"numeric"}', 1, 1000
		);
		CREATE TABLE semantic_rules_v3(conflicting_column TEXT);
	`); err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}

	if archive, err := Open(path); err == nil {
		_ = archive.Close()
		t.Fatal("migration succeeded despite a conflicting v3 table")
	}

	db, err = sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = db.Close() })
	var version int
	if err := db.QueryRow("PRAGMA user_version").Scan(&version); err != nil {
		t.Fatal(err)
	}
	if version != 13 {
		t.Fatalf("schema version = %d, want 13 after rollback", version)
	}
	var preservedV2Definitions int
	if err := db.QueryRow(
		"SELECT count(*) FROM semantic_definitions_v2",
	).Scan(&preservedV2Definitions); err != nil {
		t.Fatal(err)
	}
	if preservedV2Definitions != 1 {
		t.Fatalf(
			"preserved v2 definitions = %d, want 1",
			preservedV2Definitions,
		)
	}
	var partialV3Table int
	if err := db.QueryRow(`
		SELECT EXISTS(
			SELECT 1 FROM sqlite_master
			WHERE type = 'table' AND name = 'semantic_signal_configs_v3'
		)
	`).Scan(&partialV3Table); err != nil {
		t.Fatal(err)
	}
	if partialV3Table != 0 {
		t.Fatal("failed migration left an earlier v3 table behind")
	}
}

func applyTestMigrationsThrough(t *testing.T, db *sql.DB, version int) {
	t.Helper()
	for _, migration := range schemaMigrations {
		if migration.version > version {
			break
		}
		if _, err := db.Exec(migration.sql); err != nil {
			t.Fatal(err)
		}
		if _, err := db.Exec(
			fmt.Sprintf("PRAGMA user_version = %d", migration.version),
		); err != nil {
			t.Fatal(err)
		}
	}
}

func TestActivationMigrationFailsClosedForAmbiguousLegacyEpochs(t *testing.T) {
	for _, test := range []struct {
		name      string
		epochs    []string
		wantState EdgeActivationState
		wantEpoch string
	}{
		{
			name:      "one legacy custody epoch remains active",
			epochs:    []string{"epoch-a"},
			wantState: EdgeActive,
			wantEpoch: "epoch-a",
		},
		{
			name:      "multiple legacy custody epochs require recovery",
			epochs:    []string{"epoch-a", "epoch-b"},
			wantState: EdgeRecoveryHold,
			wantEpoch: "epoch-a",
		},
	} {
		t.Run(test.name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "legacy-activation.db")
			db, err := sql.Open("sqlite", path)
			if err != nil {
				t.Fatal(err)
			}
			for _, migration := range schemaMigrations {
				if migration.version > 10 {
					break
				}
				if _, err := db.Exec(migration.sql); err != nil {
					t.Fatal(err)
				}
				if _, err := db.Exec(fmt.Sprintf("PRAGMA user_version = %d", migration.version)); err != nil {
					t.Fatal(err)
				}
			}
			for index, epoch := range test.epochs {
				if _, err := db.Exec(`
					INSERT INTO accepted_cursors(
						edge_node_id, ledger_epoch, accepted_through, updated_at
					) VALUES('edge-legacy', ?, ?, 1)
				`, epoch, index+1); err != nil {
					t.Fatal(err)
				}
			}
			if err := db.Close(); err != nil {
				t.Fatal(err)
			}

			store, err := Open(path)
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() { _ = store.Close() })
			edges, err := store.ListEdges(context.Background())
			if err != nil {
				t.Fatal(err)
			}
			if len(edges) != 1 || edges[0].State != test.wantState ||
				edges[0].LedgerEpoch != test.wantEpoch {
				t.Fatalf("edges = %#v", edges)
			}
		})
	}
}

func TestSignalProfileV2MigrationPreservesLegacyProfile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "site.db")
	db, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	for _, migration := range schemaMigrations {
		if migration.version > 9 {
			break
		}
		if _, err := db.Exec(migration.sql); err != nil {
			t.Fatal(err)
		}
		if _, err := db.Exec(fmt.Sprintf("PRAGMA user_version = %d", migration.version)); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := db.Exec(`
		INSERT INTO signal_profiles (
			edge_node_id, series_key, display_name, revision, updated_at
		) VALUES ('edge-node-01', 'legacy-series', '旧温度表示', 3, 1000)
	`); err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}

	store, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = store.Close() })
	var (
		displayName       string
		displaySensorType string
		displayValueKind  string
		revision          int64
	)
	if err := store.db.QueryRow(`
		SELECT display_name, display_sensor_type, display_value_kind, revision
		FROM signal_profiles
		WHERE edge_node_id = 'edge-node-01' AND series_key = 'legacy-series'
	`).Scan(&displayName, &displaySensorType, &displayValueKind, &revision); err != nil {
		t.Fatal(err)
	}
	if displayName != "旧温度表示" || revision != 3 {
		t.Fatalf("legacy profile changed: name=%q revision=%d", displayName, revision)
	}
	if displaySensorType != "" || displayValueKind != "" {
		t.Fatalf("legacy profile was silently completed: sensor=%q kind=%q",
			displaySensorType, displayValueKind)
	}
}
