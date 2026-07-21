package store

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"hash"
	"regexp"
	"sort"
	"strings"
)

type ProfileMigrationReport struct {
	SourceProfile Profile          `json:"source_profile"`
	TargetProfile Profile          `json:"target_profile"`
	EdgeID        string           `json:"edge_id"`
	SchemaVersion int              `json:"schema_version"`
	TableCounts   map[string]int64 `json:"table_counts"`
	Cursors       []BackupCursor   `json:"cursors"`
	ContentDigest string           `json:"content_digest"`
	Completed     bool             `json:"completed"`
}

type migrationQuerier interface {
	QueryContext(context.Context, string, ...any) (*sql.Rows, error)
	QueryRowContext(context.Context, string, ...any) *sql.Row
}

var sqlIdentifierPattern = regexp.MustCompile(`^[a-z][a-z0-9_]*$`)

func MigrateSQLiteToPostgres(
	ctx context.Context,
	sqlitePath string,
	postgresDSN string,
) (ProfileMigrationReport, error) {
	report := ProfileMigrationReport{
		SourceProfile: ProfileEmbedded,
		TargetProfile: ProfilePostgres,
		TableCounts:   make(map[string]int64),
	}
	source, err := sql.Open("sqlite", "file:"+sqlitePath+"?mode=ro")
	if err != nil {
		return report, errors.New("open SQLite migration source")
	}
	defer source.Close()
	if err := source.QueryRowContext(ctx, "PRAGMA user_version").Scan(&report.SchemaVersion); err != nil {
		return report, errors.New("read SQLite migration source schema")
	}
	latest := schemaMigrations[len(schemaMigrations)-1].version
	if report.SchemaVersion != latest {
		return report, fmt.Errorf("SQLite migration source schema is %d, want %d", report.SchemaVersion, latest)
	}
	if err := source.QueryRowContext(ctx,
		"SELECT edge_id FROM edge_meta WHERE singleton = 1",
	).Scan(&report.EdgeID); err != nil {
		return report, errors.New("read SQLite migration source Edge identity")
	}
	tables, columns, err := sqliteMigrationLayout(ctx, source)
	if err != nil {
		return report, err
	}
	target, err := OpenWithOptions(OpenOptions{
		Profile: ProfilePostgres, PostgresDSN: postgresDSN,
		EdgeID: report.EdgeID,
	})
	if err != nil {
		// A fresh PostgreSQL schema has a generated Edge ID. Open without the
		// configured identity, then replace that seed inside the import.
		target, err = OpenWithOptions(OpenOptions{Profile: ProfilePostgres, PostgresDSN: postgresDSN})
	}
	if err != nil {
		return report, err
	}
	defer target.Close()
	if err := requireEmptyPostgresMigrationTarget(ctx, target, tables); err != nil {
		return report, err
	}
	tx, err := target.db.BeginTx(ctx, nil)
	if err != nil {
		return report, err
	}
	defer tx.Rollback()
	if _, err := tx.ExecContext(ctx, "DELETE FROM edge_meta"); err != nil {
		return report, err
	}
	sourceHash := sha256.New()
	for _, table := range tables {
		count, err := copyMigrationTable(ctx, source, tx, table, columns[table], sourceHash)
		if err != nil {
			return report, fmt.Errorf("copy SQLite table %s: %w", table, err)
		}
		report.TableCounts[table] = count
	}
	if err := resetPostgresIdentitySequences(ctx, tx); err != nil {
		return report, err
	}
	targetHash := sha256.New()
	for _, table := range tables {
		count, err := digestMigrationTable(ctx, tx, table, columns[table], targetHash)
		if err != nil {
			return report, fmt.Errorf("verify PostgreSQL table %s: %w", table, err)
		}
		if count != report.TableCounts[table] {
			return report, fmt.Errorf("migration row count mismatch for %s", table)
		}
	}
	if !strings.EqualFold(hex.EncodeToString(sourceHash.Sum(nil)), hex.EncodeToString(targetHash.Sum(nil))) {
		return report, errors.New("migration content digest mismatch")
	}
	report.ContentDigest = hex.EncodeToString(sourceHash.Sum(nil))
	rows, err := tx.QueryContext(ctx, `
		SELECT edge_node_id, ledger_epoch, accepted_through
		FROM accepted_cursors ORDER BY edge_node_id, ledger_epoch
	`)
	if err != nil {
		return report, err
	}
	for rows.Next() {
		var cursor BackupCursor
		if err := rows.Scan(&cursor.EdgeNodeID, &cursor.LedgerEpoch, &cursor.AcceptedThrough); err != nil {
			_ = rows.Close()
			return report, err
		}
		report.Cursors = append(report.Cursors, cursor)
	}
	if err := rows.Close(); err != nil {
		return report, err
	}
	if err := tx.Commit(); err != nil {
		return report, err
	}
	report.Completed = true
	return report, nil
}

func sqliteMigrationLayout(ctx context.Context, db *sql.DB) ([]string, map[string][]string, error) {
	rows, err := db.QueryContext(ctx, `
		SELECT name FROM sqlite_master
		WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
		ORDER BY name
	`)
	if err != nil {
		return nil, nil, err
	}
	defer rows.Close()
	var tables []string
	for rows.Next() {
		var table string
		if err := rows.Scan(&table); err != nil {
			return nil, nil, err
		}
		if !sqlIdentifierPattern.MatchString(table) {
			return nil, nil, errors.New("SQLite migration source contains an invalid table name")
		}
		tables = append(tables, table)
	}
	if err := rows.Err(); err != nil {
		return nil, nil, err
	}
	sort.SliceStable(tables, func(left, right int) bool {
		return migrationTablePriority(tables[left]) < migrationTablePriority(tables[right])
	})
	columns := make(map[string][]string, len(tables))
	for _, table := range tables {
		columnRows, err := db.QueryContext(ctx, "PRAGMA table_info("+quoteSQLIdentifier(table)+")")
		if err != nil {
			return nil, nil, err
		}
		for columnRows.Next() {
			var sequence, notNull, primaryKey int
			var name, dataType string
			var defaultValue any
			if err := columnRows.Scan(&sequence, &name, &dataType, &notNull, &defaultValue, &primaryKey); err != nil {
				_ = columnRows.Close()
				return nil, nil, err
			}
			if !sqlIdentifierPattern.MatchString(name) {
				_ = columnRows.Close()
				return nil, nil, errors.New("SQLite migration source contains an invalid column name")
			}
			columns[table] = append(columns[table], name)
		}
		if err := columnRows.Close(); err != nil {
			return nil, nil, err
		}
	}
	return tables, columns, nil
}

func migrationTablePriority(table string) int {
	switch table {
	case "edge_accounts":
		return 0
	case "edge_backup_events", "edge_restore_events":
		return 1
	case "edge_backup_cursors", "edge_restore_cursor_checks", "edge_sessions":
		return 2
	default:
		return 1
	}
}

func requireEmptyPostgresMigrationTarget(ctx context.Context, target *Store, tables []string) error {
	for _, table := range tables {
		if table == "edge_meta" {
			continue
		}
		var count int64
		if err := target.db.QueryRowContext(ctx,
			"SELECT count(*) FROM "+quoteSQLIdentifier(table),
		).Scan(&count); err != nil {
			return err
		}
		if count != 0 {
			return errors.New("PostgreSQL migration target is not empty")
		}
	}
	return nil
}

func copyMigrationTable(
	ctx context.Context,
	source migrationQuerier,
	target *sqlTx,
	table string,
	columns []string,
	digest hash.Hash,
) (int64, error) {
	selectSQL := migrationSelectSQL(table, columns)
	rows, err := source.QueryContext(ctx, selectSQL)
	if err != nil {
		return 0, err
	}
	defer rows.Close()
	quoted := make([]string, len(columns))
	markers := make([]string, len(columns))
	for index, column := range columns {
		quoted[index] = quoteSQLIdentifier(column)
		markers[index] = "?"
	}
	insertSQL := "INSERT INTO " + quoteSQLIdentifier(table) + "(" +
		strings.Join(quoted, ",") + ") VALUES(" + strings.Join(markers, ",") + ")"
	var count int64
	for rows.Next() {
		values, pointers := migrationRowBuffers(len(columns))
		if err := rows.Scan(pointers...); err != nil {
			return 0, err
		}
		writeMigrationDigest(digest, table, values)
		if _, err := target.ExecContext(ctx, insertSQL, values...); err != nil {
			return 0, err
		}
		count++
	}
	return count, rows.Err()
}

func digestMigrationTable(
	ctx context.Context,
	database migrationQuerier,
	table string,
	columns []string,
	digest hash.Hash,
) (int64, error) {
	rows, err := database.QueryContext(ctx, migrationSelectSQL(table, columns))
	if err != nil {
		return 0, err
	}
	defer rows.Close()
	var count int64
	for rows.Next() {
		values, pointers := migrationRowBuffers(len(columns))
		if err := rows.Scan(pointers...); err != nil {
			return 0, err
		}
		writeMigrationDigest(digest, table, values)
		count++
	}
	return count, rows.Err()
}

func migrationSelectSQL(table string, columns []string) string {
	quoted := make([]string, len(columns))
	for index, column := range columns {
		quoted[index] = quoteSQLIdentifier(column)
	}
	list := strings.Join(quoted, ",")
	return "SELECT " + list + " FROM " + quoteSQLIdentifier(table) + " ORDER BY " + list
}

func migrationRowBuffers(count int) ([]any, []any) {
	values := make([]any, count)
	pointers := make([]any, count)
	for index := range values {
		pointers[index] = &values[index]
	}
	return values, pointers
}

func writeMigrationDigest(digest hash.Hash, table string, values []any) {
	_, _ = fmt.Fprintf(digest, "T%d:%sR", len(table), table)
	for _, value := range values {
		switch typed := value.(type) {
		case nil:
			_, _ = digest.Write([]byte("N;"))
		case int64:
			_, _ = fmt.Fprintf(digest, "I%d;", typed)
		case float64:
			_, _ = fmt.Fprintf(digest, "F%016x;", typed)
		case bool:
			if typed {
				_, _ = digest.Write([]byte("I1;"))
			} else {
				_, _ = digest.Write([]byte("I0;"))
			}
		case string:
			_, _ = fmt.Fprintf(digest, "S%d:%s;", len(typed), typed)
		case []byte:
			_, _ = fmt.Fprintf(digest, "B%d:", len(typed))
			_, _ = digest.Write(typed)
			_, _ = digest.Write([]byte(";"))
		default:
			_, _ = fmt.Fprintf(digest, "X%v;", typed)
		}
	}
}

func resetPostgresIdentitySequences(ctx context.Context, tx *sqlTx) error {
	for _, identity := range [][2]string{
		{"audit_events", "audit_row_id"},
		{"semantic_events", "event_row_id"},
		{"semantic_observations_v2", "observation_row_id"},
		{"semantic_observations_v3", "observation_row_id"},
	} {
		query := fmt.Sprintf(`
			SELECT setval(
				pg_get_serial_sequence('%s', '%s'),
				COALESCE(MAX(%s), 1),
				COUNT(*) > 0
			) FROM %s
		`, identity[0], identity[1], quoteSQLIdentifier(identity[1]), quoteSQLIdentifier(identity[0]))
		if _, err := tx.ExecContext(ctx, query); err != nil {
			return err
		}
	}
	return nil
}

func quoteSQLIdentifier(identifier string) string {
	return `"` + identifier + `"`
}
