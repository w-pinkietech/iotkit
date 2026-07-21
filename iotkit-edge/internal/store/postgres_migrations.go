package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"strings"
)

func openPostgres(dsn string, configuredEdgeID string) (*Store, error) {
	return openPostgresInternal(dsn, configuredEdgeID, false, false)
}

func openPostgresForProfileMigration(dsn string, configuredEdgeID string) (*Store, error) {
	return openPostgresInternal(dsn, configuredEdgeID, false, true)
}

func openPostgresInternal(
	dsn string,
	configuredEdgeID string,
	allowIncompleteRestore bool,
	exclusiveGuard bool,
) (*Store, error) {
	db, err := sql.Open("pgx", dsn)
	if err != nil {
		return nil, errors.New("open PostgreSQL storage")
	}
	db.SetMaxOpenConns(20)
	ctx := context.Background()
	if err := db.PingContext(ctx); err != nil {
		_ = db.Close()
		return nil, errors.New("connect PostgreSQL storage")
	}
	var postgresGuard *sql.Conn
	if !allowIncompleteRestore {
		postgresGuard, err = acquirePostgresStorageGuard(ctx, db, !exclusiveGuard)
		if err != nil {
			_ = db.Close()
			return nil, err
		}
	}
	if err := validatePostgresDurability(ctx, db); err != nil {
		closePostgresGuard(postgresGuard)
		_ = db.Close()
		return nil, err
	}
	if !allowIncompleteRestore {
		restoreState, err := postgresDatabaseRestoreState(ctx, db)
		if err != nil {
			closePostgresGuard(postgresGuard)
			_ = db.Close()
			return nil, errors.New("read PostgreSQL restore state")
		}
		if restoreState != "" && restoreState != "ready" {
			closePostgresGuard(postgresGuard)
			_ = db.Close()
			return nil, errors.New("PostgreSQL restore state is not ready")
		}
	}
	if err := applyPostgresMigrations(ctx, db, configuredEdgeID); err != nil {
		closePostgresGuard(postgresGuard)
		_ = db.Close()
		return nil, err
	}
	if err := validatePostgresSchemaContract(ctx, db); err != nil {
		closePostgresGuard(postgresGuard)
		_ = db.Close()
		return nil, err
	}
	store := &Store{
		db:            &sqlDatabase{raw: db, dialect: dialectPostgres},
		profile:       ProfilePostgres,
		postgresDSN:   dsn,
		postgresGuard: postgresGuard,
	}
	if err := store.validateConfiguredEdgeIdentity(ctx, configuredEdgeID); err != nil {
		closePostgresGuard(postgresGuard)
		_ = db.Close()
		return nil, err
	}
	if err := store.validateEdgeIdentity(ctx); err != nil {
		closePostgresGuard(postgresGuard)
		_ = db.Close()
		return nil, err
	}
	return store, nil
}

func acquirePostgresStorageGuard(ctx context.Context, db *sql.DB, shared bool) (*sql.Conn, error) {
	connection, err := db.Conn(ctx)
	if err != nil {
		return nil, errors.New("open PostgreSQL storage operation guard")
	}
	function := "pg_try_advisory_lock"
	if shared {
		function = "pg_try_advisory_lock_shared"
	}
	var acquired bool
	if err := connection.QueryRowContext(ctx,
		"SELECT "+function+"(hashtextextended('iotkit-edge-storage:' || current_database(), 0))",
	).Scan(&acquired); err != nil || !acquired {
		_ = connection.Close()
		return nil, errors.New("PostgreSQL database is in use by another IoTKit process")
	}
	return connection, nil
}

func closePostgresGuard(guard *sql.Conn) {
	if guard != nil {
		_ = guard.Close()
	}
}

func postgresDatabaseRestoreState(ctx context.Context, db *sql.DB) (string, error) {
	rows, err := db.QueryContext(ctx, `
		SELECT setting
		FROM pg_db_role_setting
		CROSS JOIN LATERAL unnest(setconfig) AS setting
		WHERE setdatabase = (SELECT oid FROM pg_database WHERE datname = current_database())
			AND setrole = 0
			AND setting LIKE 'iotkit.restore_state=%'
	`)
	if err != nil {
		return "", err
	}
	defer rows.Close()
	state := ""
	for rows.Next() {
		var setting string
		if err := rows.Scan(&setting); err != nil {
			return "", err
		}
		if state != "" {
			return "", errors.New("duplicate PostgreSQL restore state")
		}
		state = strings.TrimPrefix(setting, "iotkit.restore_state=")
	}
	return state, rows.Err()
}

func validatePostgresSchemaContract(ctx context.Context, db *sql.DB) error {
	rows, err := db.QueryContext(ctx, `
		SELECT column_name, data_type
		FROM information_schema.columns
		WHERE table_schema = 'public'
			AND table_name = 'signal_calibration_revisions_v3'
			AND column_name IN ('scale', 'offset')
	`)
	if err != nil {
		return errors.New("inspect PostgreSQL storage schema")
	}
	defer rows.Close()
	seen := 0
	for rows.Next() {
		var column, dataType string
		if err := rows.Scan(&column, &dataType); err != nil {
			return errors.New("inspect PostgreSQL storage schema")
		}
		if dataType != "double precision" {
			return fmt.Errorf("PostgreSQL column %s does not preserve SQLite numeric precision", column)
		}
		seen++
	}
	if err := rows.Err(); err != nil || seen != 2 {
		return errors.New("PostgreSQL storage schema is incomplete")
	}
	return nil
}

func validatePostgresDurability(ctx context.Context, db *sql.DB) error {
	for setting, expected := range map[string]string{
		"fsync":              "on",
		"synchronous_commit": "on",
		"full_page_writes":   "on",
	} {
		var value string
		if err := db.QueryRowContext(ctx, "SHOW "+setting).Scan(&value); err != nil {
			return errors.New("read PostgreSQL durability settings")
		}
		if value != expected {
			return errors.New("PostgreSQL durability settings do not satisfy the custody contract")
		}
	}
	return nil
}

func (store *Store) validateConfiguredEdgeIdentity(ctx context.Context, configuredEdgeID string) error {
	if configuredEdgeID == "" {
		return nil
	}
	var storedEdgeID string
	if err := store.db.QueryRowContext(ctx,
		"SELECT edge_id FROM edge_meta WHERE singleton = 1",
	).Scan(&storedEdgeID); err != nil {
		return err
	}
	if storedEdgeID != configuredEdgeID {
		return errors.New("configured IoTKit Edge ID does not match the existing database")
	}
	return nil
}

func applyPostgresMigrations(ctx context.Context, db *sql.DB, configuredEdgeID string) error {
	if _, err := db.ExecContext(ctx, postgresCompatibilitySQL); err != nil {
		return fmt.Errorf("initialize PostgreSQL compatibility functions: %w", err)
	}
	if _, err := db.ExecContext(ctx, `
		CREATE TABLE IF NOT EXISTS edge_schema_meta (
			singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
			version INTEGER NOT NULL
		);
		INSERT INTO edge_schema_meta(singleton, version) VALUES(1, 0)
		ON CONFLICT(singleton) DO NOTHING;
	`); err != nil {
		return fmt.Errorf("initialize PostgreSQL schema metadata: %w", err)
	}
	var current int
	if err := db.QueryRowContext(ctx,
		"SELECT version FROM edge_schema_meta WHERE singleton = 1",
	).Scan(&current); err != nil {
		return fmt.Errorf("read PostgreSQL Edge schema version: %w", err)
	}
	latest := schemaMigrations[len(schemaMigrations)-1].version
	if current > latest {
		return fmt.Errorf("Edge schema version %d is newer than supported version %d", current, latest)
	}
	if current == 28 && latest == 29 {
		if err := migratePostgres28To29(ctx, db); err != nil {
			return err
		}
		current = 29
	}
	if current > 0 && current < latest {
		return errors.New("automatic PostgreSQL schema upgrades are not supported; use a verified offline migration")
	}
	for _, migration := range schemaMigrations {
		if migration.version <= current {
			continue
		}
		tx, err := db.BeginTx(ctx, nil)
		if err != nil {
			return fmt.Errorf("begin PostgreSQL Edge schema migration %d: %w", migration.version, err)
		}
		migrationSQL := postgresMigrationSQL(migration.sql)
		if migrationSQL != "" {
			if _, err := tx.ExecContext(ctx, migrationSQL); err != nil {
				_ = tx.Rollback()
				return fmt.Errorf("apply PostgreSQL Edge schema migration %d: %w", migration.version, err)
			}
		}
		if migration.version == 11 && configuredEdgeID != "" {
			if _, err := tx.ExecContext(ctx,
				"UPDATE edge_meta SET edge_id = $1 WHERE singleton = 1",
				configuredEdgeID,
			); err != nil {
				_ = tx.Rollback()
				return fmt.Errorf("assign configured Edge identity in PostgreSQL migration 11: %w", err)
			}
		}
		if _, err := tx.ExecContext(ctx,
			"UPDATE edge_schema_meta SET version = $1 WHERE singleton = 1",
			migration.version,
		); err != nil {
			_ = tx.Rollback()
			return fmt.Errorf("record PostgreSQL Edge schema migration %d: %w", migration.version, err)
		}
		if err := tx.Commit(); err != nil {
			return fmt.Errorf("commit PostgreSQL Edge schema migration %d: %w", migration.version, err)
		}
		current = migration.version
	}
	return nil
}

func canUpgradePostgresSchema(current int, latest int) bool {
	return current == latest || (current == 28 && latest == 29)
}

func migratePostgres28To29(ctx context.Context, db *sql.DB) error {
	tx, err := db.BeginTx(ctx, nil)
	if err != nil {
		return errors.New("begin PostgreSQL schema migration 29")
	}
	defer tx.Rollback()
	if _, err := tx.ExecContext(ctx, `
		ALTER TABLE signal_calibration_revisions_v3
			ALTER COLUMN scale TYPE DOUBLE PRECISION USING scale::DOUBLE PRECISION,
			ALTER COLUMN "offset" TYPE DOUBLE PRECISION USING "offset"::DOUBLE PRECISION;
		CREATE TABLE edge_storage_samples (
			sampled_at BIGINT PRIMARY KEY,
			database_bytes BIGINT NOT NULL CHECK(database_bytes >= 0),
			raw_record_count BIGINT NOT NULL CHECK(raw_record_count >= 0)
		);
		UPDATE edge_schema_meta SET version = 29 WHERE singleton = 1 AND version = 28;
	`); err != nil {
		return fmt.Errorf("apply PostgreSQL schema migration 29: %w", err)
	}
	var version int
	if err := tx.QueryRowContext(ctx,
		"SELECT version FROM edge_schema_meta WHERE singleton = 1",
	).Scan(&version); err != nil || version != 29 {
		return errors.New("verify PostgreSQL schema migration 29")
	}
	if err := tx.Commit(); err != nil {
		return errors.New("commit PostgreSQL schema migration 29")
	}
	return nil
}

func postgresMigrationSQL(source string) string {
	statements := strings.Split(source, ";")
	converted := make([]string, 0, len(statements))
	for _, statement := range statements {
		trimmed := strings.TrimSpace(statement)
		upper := strings.ToUpper(trimmed)
		if trimmed == "" || strings.HasPrefix(upper, "INSERT OR IGNORE") {
			continue
		}
		// A PostgreSQL profile is initialized directly at the current schema.
		// Historical data-copy statements operate on empty predecessor tables and
		// are deliberately omitted; edge_meta is the only seed row.
		if strings.Contains(upper, "INSERT INTO") &&
			!strings.HasPrefix(upper, "INSERT INTO EDGE_META") {
			continue
		}
		trimmed = strings.ReplaceAll(trimmed,
			"INTEGER PRIMARY KEY AUTOINCREMENT",
			"BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY",
		)
		trimmed = strings.ReplaceAll(trimmed, "INTEGER", "BIGINT")
		trimmed = strings.ReplaceAll(trimmed, " REAL", " DOUBLE PRECISION")
		trimmed = strings.ReplaceAll(trimmed, " BLOB", " BYTEA")
		trimmed = strings.ReplaceAll(trimmed, "offset", `"offset"`)
		converted = append(converted, trimmed)
	}
	return strings.Join(converted, ";\n")
}
