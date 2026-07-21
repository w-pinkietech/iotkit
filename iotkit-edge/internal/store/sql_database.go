package store

import (
	"context"
	"database/sql"
	"strings"
)

type sqlDialect string

const (
	dialectSQLite   sqlDialect = "sqlite"
	dialectPostgres sqlDialect = "postgres"
)

type sqlDatabase struct {
	raw     *sql.DB
	dialect sqlDialect
}

func (database *sqlDatabase) query(query string) string {
	query = normalizePortableSQL(query)
	if database.dialect == dialectPostgres {
		return rebindPostgresPlaceholders(query)
	}
	return query
}

func (database *sqlDatabase) Close() error { return database.raw.Close() }

func (database *sqlDatabase) Exec(query string, args ...any) (sql.Result, error) {
	return database.raw.Exec(database.query(query), portableArgs(database.dialect, args)...)
}

func (database *sqlDatabase) ExecContext(ctx context.Context, query string, args ...any) (sql.Result, error) {
	return database.raw.ExecContext(ctx, database.query(query), portableArgs(database.dialect, args)...)
}

func (database *sqlDatabase) Query(query string, args ...any) (*sql.Rows, error) {
	return database.raw.Query(database.query(query), portableArgs(database.dialect, args)...)
}

func (database *sqlDatabase) QueryContext(ctx context.Context, query string, args ...any) (*sql.Rows, error) {
	return database.raw.QueryContext(ctx, database.query(query), portableArgs(database.dialect, args)...)
}

func (database *sqlDatabase) QueryRow(query string, args ...any) *sql.Row {
	return database.raw.QueryRow(database.query(query), portableArgs(database.dialect, args)...)
}

func (database *sqlDatabase) QueryRowContext(ctx context.Context, query string, args ...any) *sql.Row {
	return database.raw.QueryRowContext(ctx, database.query(query), portableArgs(database.dialect, args)...)
}

func (database *sqlDatabase) BeginTx(ctx context.Context, options *sql.TxOptions) (*sqlTx, error) {
	tx, err := database.raw.BeginTx(ctx, options)
	if err != nil {
		return nil, err
	}
	return &sqlTx{raw: tx, dialect: database.dialect}, nil
}

type sqlTx struct {
	raw     *sql.Tx
	dialect sqlDialect
}

func (tx *sqlTx) query(query string) string {
	query = normalizePortableSQL(query)
	if tx.dialect == dialectPostgres {
		return rebindPostgresPlaceholders(query)
	}
	return query
}

func normalizePortableSQL(query string) string {
	const sqliteInsert = "INSERT OR IGNORE INTO"
	if !strings.Contains(query, sqliteInsert) {
		return query
	}
	query = strings.Replace(query, sqliteInsert, "INSERT INTO", 1)
	trimmed := strings.TrimSpace(query)
	if strings.HasSuffix(trimmed, ";") {
		trimmed = strings.TrimSuffix(trimmed, ";")
	}
	return trimmed + " ON CONFLICT DO NOTHING"
}

func portableArgs(dialect sqlDialect, args []any) []any {
	if dialect != dialectPostgres {
		return args
	}
	converted := make([]any, len(args))
	for index, value := range args {
		if boolean, ok := value.(bool); ok {
			if boolean {
				converted[index] = int64(1)
			} else {
				converted[index] = int64(0)
			}
			continue
		}
		converted[index] = value
	}
	return converted
}

func (tx *sqlTx) ExecContext(ctx context.Context, query string, args ...any) (sql.Result, error) {
	return tx.raw.ExecContext(ctx, tx.query(query), portableArgs(tx.dialect, args)...)
}

func (tx *sqlTx) QueryContext(ctx context.Context, query string, args ...any) (*sql.Rows, error) {
	return tx.raw.QueryContext(ctx, tx.query(query), portableArgs(tx.dialect, args)...)
}

func (tx *sqlTx) QueryRowContext(ctx context.Context, query string, args ...any) *sql.Row {
	return tx.raw.QueryRowContext(ctx, tx.query(query), portableArgs(tx.dialect, args)...)
}

func (tx *sqlTx) Commit() error   { return tx.raw.Commit() }
func (tx *sqlTx) Rollback() error { return tx.raw.Rollback() }

func rebindPostgresPlaceholders(query string) string {
	var output strings.Builder
	output.Grow(len(query) + 8)
	placeholder := 1
	var quote byte
	for index := 0; index < len(query); index++ {
		character := query[index]
		if quote != 0 {
			output.WriteByte(character)
			if character == quote {
				if index+1 < len(query) && query[index+1] == quote {
					index++
					output.WriteByte(query[index])
					continue
				}
				quote = 0
			}
			continue
		}
		if character == '\'' || character == '"' {
			quote = character
			output.WriteByte(character)
			continue
		}
		if character == '?' {
			output.WriteByte('$')
			output.WriteString(intToDecimal(placeholder))
			placeholder++
			continue
		}
		output.WriteByte(character)
	}
	return output.String()
}

func intToDecimal(value int) string {
	if value == 0 {
		return "0"
	}
	var buffer [20]byte
	position := len(buffer)
	for value > 0 {
		position--
		buffer[position] = byte('0' + value%10)
		value /= 10
	}
	return string(buffer[position:])
}
