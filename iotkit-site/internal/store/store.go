package store

import (
	"bytes"
	"context"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantic"
	_ "modernc.org/sqlite"
)

var (
	ErrConflict = errors.New("raw record content conflict")
	ErrGap      = errors.New("batch starts after the contiguous cursor")
)

type Store struct {
	db *sql.DB
}

type RawRecord struct {
	EdgeNodeID    string          `json:"edge_node_id"`
	LedgerEpoch   string          `json:"ledger_epoch"`
	PubSeq        int64           `json:"pub_seq"`
	PublicationID string          `json:"publication_id"`
	Record        json.RawMessage `json:"record"`
	ReceivedAt    int64           `json:"received_at"`
}

func Open(path string) (*Store, error) {
	db, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	store := &Store{db: db}
	if err := store.initialize(); err != nil {
		_ = db.Close()
		return nil, err
	}
	return store, nil
}

func (store *Store) Close() error {
	return store.db.Close()
}

func (store *Store) initialize() error {
	for _, table := range []string{"raw_records", "accepted_cursors"} {
		var legacyColumns int
		if err := store.db.QueryRow(
			"SELECT count(*) FROM pragma_table_info(?) WHERE name = 'gateway_identity'",
			table,
		).Scan(&legacyColumns); err != nil {
			return err
		}
		if legacyColumns > 0 {
			return errors.New("unsupported pre-release Site database; recreate it")
		}
	}

	_, err := store.db.Exec(`
		PRAGMA journal_mode = WAL;
		PRAGMA synchronous = FULL;
		PRAGMA foreign_keys = ON;
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
	`)
	return err
}

func (store *Store) PutSemanticMapping(ctx context.Context, spec semantic.MappingSpec) (semantic.Mapping, error) {
	var noMapping semantic.Mapping
	if err := spec.Validate(); err != nil {
		return noMapping, err
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noMapping, err
	}
	defer func() { _ = tx.Rollback() }()

	var mappingID string
	var revision int64
	err = tx.QueryRowContext(ctx, `
		SELECT mapping_id, revision
		FROM semantic_mappings
		WHERE edge_node_id = ? AND series_key = ? AND active = 1
	`, spec.EdgeNodeID, spec.SeriesKey).Scan(&mappingID, &revision)
	if errors.Is(err, sql.ErrNoRows) {
		mappingID, err = newSemanticMappingID()
		revision = 1
	} else if err == nil {
		revision++
	} else {
		return noMapping, err
	}

	if _, err := tx.ExecContext(ctx, `
		UPDATE semantic_mappings
		SET active = 0
		WHERE edge_node_id = ? AND series_key = ? AND active = 1
	`, spec.EdgeNodeID, spec.SeriesKey); err != nil {
		return noMapping, err
	}

	createdAt := time.Now().UnixMilli()
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_mappings (
			mapping_id, revision, edge_node_id, series_key, meaning,
			trigger_mode, active_value, active, created_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?)
	`, mappingID, revision, spec.EdgeNodeID, spec.SeriesKey, spec.Meaning,
		spec.TriggerMode, spec.ActiveValue, createdAt); err != nil {
		return noMapping, err
	}

	if _, err := tx.ExecContext(ctx, `
		INSERT INTO semantic_mapping_starts (
			mapping_id, mapping_revision, ledger_epoch, start_after_pub_seq
		)
		SELECT ?, ?, ledger_epoch, accepted_through
		FROM accepted_cursors
		WHERE edge_node_id = ?
	`, mappingID, revision, spec.EdgeNodeID); err != nil {
		return noMapping, err
	}

	if err := tx.Commit(); err != nil {
		return noMapping, err
	}
	return semantic.Mapping{
		ID:          mappingID,
		Revision:    revision,
		MappingSpec: spec,
		Active:      true,
		CreatedAt:   createdAt,
	}, nil
}

func (store *Store) ListSemanticMappings(ctx context.Context) ([]semantic.Mapping, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT mapping_id, revision, edge_node_id, series_key, meaning,
			trigger_mode, active_value, active, created_at
		FROM semantic_mappings
		ORDER BY mapping_id, revision
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	mappings := make([]semantic.Mapping, 0)
	for rows.Next() {
		var mapping semantic.Mapping
		if err := rows.Scan(
			&mapping.ID,
			&mapping.Revision,
			&mapping.EdgeNodeID,
			&mapping.SeriesKey,
			&mapping.Meaning,
			&mapping.TriggerMode,
			&mapping.ActiveValue,
			&mapping.Active,
			&mapping.CreatedAt,
		); err != nil {
			return nil, err
		}
		mappings = append(mappings, mapping)
	}
	return mappings, rows.Err()
}

func newSemanticMappingID() (string, error) {
	random := make([]byte, 16)
	if _, err := rand.Read(random); err != nil {
		return "", err
	}
	return "sm-" + hex.EncodeToString(random), nil
}

func (store *Store) AcceptBatch(ctx context.Context, batch contract.RecordBatch) (contract.AcceptedThrough, error) {
	var noAck contract.AcceptedThrough
	if err := batch.Validate(); err != nil {
		return noAck, err
	}

	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noAck, err
	}
	defer func() { _ = tx.Rollback() }()

	currentCursor, err := readCursor(ctx, tx, batch.EdgeNodeID, batch.LedgerEpoch)
	if err != nil {
		return noAck, err
	}
	if batch.CursorStart > currentCursor+1 {
		return noAck, ErrGap
	}

	now := time.Now().UnixMilli()
	for index, raw := range batch.Records {
		pubSeq := batch.CursorStart + int64(index)
		compact, fingerprint, err := canonicalRecord(raw)
		if err != nil {
			return noAck, err
		}
		if _, err := tx.ExecContext(ctx, `
			INSERT OR IGNORE INTO raw_records (
				edge_node_id, ledger_epoch, pub_seq, publication_id,
				record_json, record_sha256, received_at
			) VALUES (?, ?, ?, ?, ?, ?, ?)
		`, batch.EdgeNodeID, batch.LedgerEpoch, pubSeq, batch.PublicationID, compact, fingerprint[:], now); err != nil {
			return noAck, err
		}

		var storedFingerprint []byte
		if err := tx.QueryRowContext(ctx, `
			SELECT record_sha256 FROM raw_records
			WHERE edge_node_id = ? AND ledger_epoch = ? AND pub_seq = ?
		`, batch.EdgeNodeID, batch.LedgerEpoch, pubSeq).Scan(&storedFingerprint); err != nil {
			return noAck, err
		}
		if !bytes.Equal(storedFingerprint, fingerprint[:]) {
			return noAck, fmt.Errorf("%w at pub_seq %d", ErrConflict, pubSeq)
		}
	}

	acceptedThrough := currentCursor
	if batch.CursorEnd > acceptedThrough {
		acceptedThrough = batch.CursorEnd
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO accepted_cursors (
				edge_node_id, ledger_epoch, accepted_through, updated_at
			) VALUES (?, ?, ?, ?)
			ON CONFLICT(edge_node_id, ledger_epoch) DO UPDATE SET
				accepted_through = excluded.accepted_through,
				updated_at = excluded.updated_at
		`, batch.EdgeNodeID, batch.LedgerEpoch, acceptedThrough, now); err != nil {
			return noAck, err
		}
	}

	if err := tx.Commit(); err != nil {
		return noAck, err
	}
	return contract.AcceptedThrough{
		SchemaVersion:   contract.SchemaVersion,
		EdgeNodeID:      batch.EdgeNodeID,
		LedgerEpoch:     batch.LedgerEpoch,
		PublicationID:   batch.PublicationID,
		AcceptedThrough: acceptedThrough,
	}, nil
}

func (store *Store) ListRawRecords(ctx context.Context, limit int) ([]RawRecord, error) {
	if limit < 1 || limit > 10_000 {
		return nil, errors.New("raw record query limit must be between 1 and 10000")
	}
	rows, err := store.db.QueryContext(ctx, `
		SELECT edge_node_id, ledger_epoch, pub_seq, publication_id, record_json, received_at
		FROM raw_records
		ORDER BY received_at DESC, edge_node_id, ledger_epoch, pub_seq DESC
		LIMIT ?
	`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	records := make([]RawRecord, 0)
	for rows.Next() {
		var record RawRecord
		var payload []byte
		if err := rows.Scan(
			&record.EdgeNodeID,
			&record.LedgerEpoch,
			&record.PubSeq,
			&record.PublicationID,
			&payload,
			&record.ReceivedAt,
		); err != nil {
			return nil, err
		}
		record.Record = json.RawMessage(payload)
		records = append(records, record)
	}
	return records, rows.Err()
}

func readCursor(ctx context.Context, tx *sql.Tx, edgeNodeID, ledgerEpoch string) (int64, error) {
	var cursor int64
	err := tx.QueryRowContext(ctx, `
		SELECT accepted_through FROM accepted_cursors
		WHERE edge_node_id = ? AND ledger_epoch = ?
	`, edgeNodeID, ledgerEpoch).Scan(&cursor)
	if errors.Is(err, sql.ErrNoRows) {
		return 0, nil
	}
	return cursor, err
}

func canonicalRecord(raw json.RawMessage) ([]byte, [sha256.Size]byte, error) {
	var compact bytes.Buffer
	if err := json.Compact(&compact, raw); err != nil {
		return nil, [sha256.Size]byte{}, err
	}
	payload := compact.Bytes()
	return payload, sha256.Sum256(payload), nil
}
