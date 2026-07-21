package store

import (
	"bytes"
	"context"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"
	"unicode/utf8"

	"golang.org/x/crypto/argon2"
	"golang.org/x/crypto/chacha20poly1305"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
)

const (
	backupFormatVersion = 1
	backupChunkSize     = 256 * 1024
	maxBackupHeaderSize = 64 * 1024
	maxManifestSize     = 1024 * 1024
)

var backupMagic = [8]byte{'I', 'O', 'T', 'K', 'B', 'K', 'P', '1'}

type BackupManifest = edgeapp.BackupManifest

type backupContainerHeader struct {
	FormatVersion int    `json:"format_version"`
	KDF           string `json:"kdf"`
	Salt          string `json:"salt"`
	KDFTime       uint32 `json:"kdf_time"`
	KDFMemoryKiB  uint32 `json:"kdf_memory_kib"`
	KDFThreads    uint8  `json:"kdf_threads"`
	Cipher        string `json:"cipher"`
	NoncePrefix   string `json:"nonce_prefix"`
	ChunkSize     int    `json:"chunk_size"`
}

func (store *Store) ApplyEncryptedBackup(
	ctx context.Context,
	actor edgeapp.Actor,
	destination string,
	passphrase string,
) (BackupManifest, error) {
	var empty BackupManifest
	if err := actor.Validate(); err != nil {
		return empty, err
	}
	if actor.Class != edgeapp.ActorLocalCLI {
		return empty, edgeapp.ErrForbidden
	}
	if err := validateBackupPassphrase(passphrase); err != nil {
		return empty, err
	}
	if err := requireAbsentPath(destination, "backup destination"); err != nil {
		return empty, err
	}
	directory := filepath.Dir(destination)
	snapshotFile, err := os.CreateTemp(directory, ".iotkit-edge-snapshot-*")
	if err != nil {
		return empty, fmt.Errorf("create Edge snapshot staging file: %w", err)
	}
	snapshotPath := snapshotFile.Name()
	if err := snapshotFile.Close(); err != nil {
		_ = os.Remove(snapshotPath)
		return empty, err
	}
	if err := os.Remove(snapshotPath); err != nil {
		return empty, err
	}
	defer os.Remove(snapshotPath)

	snapshot, err := store.CreateConsistentSnapshot(ctx, snapshotPath)
	if err != nil {
		return empty, err
	}
	digest, err := fileSHA256(snapshotPath)
	if err != nil {
		return empty, err
	}
	backupID, err := randomOperationID("backup_")
	if err != nil {
		return empty, err
	}
	manifest := BackupManifest{
		FormatVersion:  backupFormatVersion,
		StorageProfile: string(snapshot.StorageProfile),
		PayloadFormat:  snapshot.PayloadFormat,
		BackupID:       backupID,
		CreatedAt:      time.Now().UnixMilli(),
		EdgeID:         snapshot.EdgeID,
		SchemaVersion:  snapshot.SchemaVersion,
		RawRecordCount: snapshot.RawRecordCount,
		Cursors:        snapshot.Cursors,
		DatabaseSHA256: hex.EncodeToString(digest[:]),
	}
	if err := encryptBackupContainer(destination, snapshotPath, manifest, passphrase); err != nil {
		return empty, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		_ = os.Remove(destination)
		return empty, err
	}
	defer tx.Rollback()
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO edge_backup_events(
			backup_id, created_at, destination_name,
			database_sha256, raw_record_count
		) VALUES(?, ?, ?, ?, ?)
	`, manifest.BackupID, manifest.CreatedAt, filepath.Base(destination),
		manifest.DatabaseSHA256, manifest.RawRecordCount); err != nil {
		_ = os.Remove(destination)
		return empty, fmt.Errorf("record completed Edge backup: %w", err)
	}
	for _, cursor := range manifest.Cursors {
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO edge_backup_cursors(
				backup_id, edge_node_id, ledger_epoch, accepted_through
			) VALUES(?, ?, ?, ?)
		`, manifest.BackupID, cursor.EdgeNodeID,
			cursor.LedgerEpoch, cursor.AcceptedThrough); err != nil {
			_ = os.Remove(destination)
			return empty, fmt.Errorf("record completed Edge backup cursor: %w", err)
		}
	}
	summary, err := json.Marshal(struct {
		BackupID       string `json:"backup_id"`
		DatabaseSHA256 string `json:"database_sha256"`
		RawRecordCount int64  `json:"raw_record_count"`
	}{manifest.BackupID, manifest.DatabaseSHA256, manifest.RawRecordCount})
	if err != nil {
		_ = os.Remove(destination)
		return empty, err
	}
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt: manifest.CreatedAt, ActorClass: actor.Class, ActorRef: actor.Ref,
		Operation: "edge_backup.create", ResourceRef: manifest.BackupID,
		Outcome: auditOutcomeSuccess, Summary: summary,
	}); err != nil {
		_ = os.Remove(destination)
		return empty, fmt.Errorf("audit completed Edge backup: %w", err)
	}
	if err := tx.Commit(); err != nil {
		_ = os.Remove(destination)
		return empty, fmt.Errorf("commit completed Edge backup: %w", err)
	}
	return manifest, nil
}

func RestoreEncryptedBackup(
	ctx context.Context,
	source string,
	destination string,
	passphrase string,
) (BackupManifest, error) {
	var empty BackupManifest
	if err := validateBackupPassphrase(passphrase); err != nil {
		return empty, err
	}
	if err := requireAbsentPath(destination, "restore destination"); err != nil {
		return empty, err
	}
	directory := filepath.Dir(destination)
	payload, err := os.CreateTemp(directory, ".iotkit-edge-restore-payload-*")
	if err != nil {
		return empty, fmt.Errorf("create restore staging file: %w", err)
	}
	payloadPath := payload.Name()
	defer os.Remove(payloadPath)
	if err := payload.Chmod(0o600); err != nil {
		_ = payload.Close()
		return empty, err
	}
	if err := decryptBackupContainer(source, payload, passphrase); err != nil {
		_ = payload.Close()
		return empty, err
	}
	if err := payload.Sync(); err != nil {
		_ = payload.Close()
		return empty, err
	}
	if _, err := payload.Seek(0, io.SeekStart); err != nil {
		_ = payload.Close()
		return empty, err
	}

	manifest, err := readBackupManifest(payload)
	if err != nil {
		_ = payload.Close()
		return empty, err
	}
	if manifest.StorageProfile != "" && manifest.StorageProfile != string(ProfileEmbedded) {
		_ = payload.Close()
		return empty, errors.New("backup storage profile does not match embedded restore destination")
	}
	database, err := os.CreateTemp(directory, ".iotkit-edge-restored-db-*")
	if err != nil {
		_ = payload.Close()
		return empty, err
	}
	databasePath := database.Name()
	defer func() {
		_ = os.Remove(databasePath)
		_ = os.Remove(databasePath + "-wal")
		_ = os.Remove(databasePath + "-shm")
	}()
	if err := database.Chmod(0o600); err != nil {
		_ = payload.Close()
		_ = database.Close()
		return empty, err
	}
	hash := sha256.New()
	if _, err := io.Copy(io.MultiWriter(database, hash), payload); err != nil {
		_ = payload.Close()
		_ = database.Close()
		return empty, fmt.Errorf("extract Edge database: %w", err)
	}
	if err := payload.Close(); err != nil {
		_ = database.Close()
		return empty, err
	}
	if err := database.Sync(); err != nil {
		_ = database.Close()
		return empty, err
	}
	if err := database.Close(); err != nil {
		return empty, err
	}
	if got := hex.EncodeToString(hash.Sum(nil)); !strings.EqualFold(got, manifest.DatabaseSHA256) {
		return empty, errors.New("Edge backup database checksum does not match its manifest")
	}
	if err := validateRestoredSnapshot(ctx, databasePath, manifest); err != nil {
		return empty, err
	}
	if err := prepareRestoredDatabase(ctx, databasePath, manifest); err != nil {
		return empty, err
	}
	if err := installNewFile(databasePath, destination, "restore destination"); err != nil {
		return empty, err
	}
	return manifest, nil
}

func encryptBackupContainer(
	destination string,
	databasePath string,
	manifest BackupManifest,
	passphrase string,
) error {
	header, key, noncePrefix, err := newBackupHeader(passphrase)
	if err != nil {
		return err
	}
	defer clear(key)
	headerJSON, err := json.Marshal(header)
	if err != nil {
		return err
	}
	manifestJSON, err := json.Marshal(manifest)
	if err != nil {
		return err
	}
	if len(manifestJSON) > maxManifestSize {
		return errors.New("Edge backup manifest is too large")
	}
	database, err := os.Open(databasePath)
	if err != nil {
		return err
	}
	defer database.Close()
	manifestLength := make([]byte, 4)
	binary.BigEndian.PutUint32(manifestLength, uint32(len(manifestJSON)))
	plaintext := io.MultiReader(bytes.NewReader(manifestLength), bytes.NewReader(manifestJSON), database)

	directory := filepath.Dir(destination)
	output, err := os.CreateTemp(directory, ".iotkit-edge-backup-*")
	if err != nil {
		return fmt.Errorf("create backup staging file: %w", err)
	}
	temporaryPath := output.Name()
	installed := false
	defer func() {
		_ = output.Close()
		if !installed {
			_ = os.Remove(temporaryPath)
		}
	}()
	if err := output.Chmod(0o600); err != nil {
		return err
	}
	if _, err := output.Write(backupMagic[:]); err != nil {
		return err
	}
	if err := binary.Write(output, binary.BigEndian, uint32(len(headerJSON))); err != nil {
		return err
	}
	if _, err := output.Write(headerJSON); err != nil {
		return err
	}
	aead, err := chacha20poly1305.NewX(key)
	if err != nil {
		return err
	}
	headerDigest := sha256.Sum256(append(append([]byte{}, backupMagic[:]...), headerJSON...))
	buffer := make([]byte, backupChunkSize)
	var sequence uint64
	for {
		count, readErr := io.ReadFull(plaintext, buffer)
		if readErr != nil && !errors.Is(readErr, io.EOF) && !errors.Is(readErr, io.ErrUnexpectedEOF) {
			return readErr
		}
		if count > 0 {
			plainChunk := make([]byte, count+1)
			copy(plainChunk[1:], buffer[:count])
			if err := writeEncryptedChunk(output, aead, noncePrefix, headerDigest, sequence, plainChunk); err != nil {
				return err
			}
			sequence++
		}
		if errors.Is(readErr, io.EOF) || errors.Is(readErr, io.ErrUnexpectedEOF) {
			if err := writeEncryptedChunk(output, aead, noncePrefix, headerDigest, sequence, []byte{1}); err != nil {
				return err
			}
			break
		}
	}
	if err := output.Sync(); err != nil {
		return err
	}
	if err := output.Close(); err != nil {
		return err
	}
	if err := installNewFile(temporaryPath, destination, "backup destination"); err != nil {
		return err
	}
	installed = true
	return nil
}

func decryptBackupContainer(source string, destination io.Writer, passphrase string) error {
	input, err := os.Open(source)
	if err != nil {
		return fmt.Errorf("open Edge backup: %w", err)
	}
	defer input.Close()
	var magic [8]byte
	if _, err := io.ReadFull(input, magic[:]); err != nil || magic != backupMagic {
		return errors.New("unsupported or damaged Edge backup format")
	}
	var headerLength uint32
	if err := binary.Read(input, binary.BigEndian, &headerLength); err != nil || headerLength == 0 || headerLength > maxBackupHeaderSize {
		return errors.New("invalid Edge backup header")
	}
	headerJSON := make([]byte, headerLength)
	if _, err := io.ReadFull(input, headerJSON); err != nil {
		return errors.New("truncated Edge backup header")
	}
	var header backupContainerHeader
	decoder := json.NewDecoder(bytes.NewReader(headerJSON))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&header); err != nil {
		return errors.New("invalid Edge backup header")
	}
	salt, noncePrefix, err := validateBackupHeader(header)
	if err != nil {
		return err
	}
	key := argon2.IDKey([]byte(passphrase), salt, header.KDFTime, header.KDFMemoryKiB, header.KDFThreads, chacha20poly1305.KeySize)
	defer clear(key)
	aead, err := chacha20poly1305.NewX(key)
	if err != nil {
		return err
	}
	headerDigest := sha256.Sum256(append(append([]byte{}, backupMagic[:]...), headerJSON...))
	for sequence := uint64(0); ; sequence++ {
		var ciphertextLength uint32
		if err := binary.Read(input, binary.BigEndian, &ciphertextLength); err != nil {
			return errors.New("Edge backup is truncated before its final authenticated chunk")
		}
		maximum := uint32(header.ChunkSize + 1 + aead.Overhead())
		if ciphertextLength < uint32(1+aead.Overhead()) || ciphertextLength > maximum {
			return errors.New("invalid encrypted Edge backup chunk")
		}
		ciphertext := make([]byte, ciphertextLength)
		if _, err := io.ReadFull(input, ciphertext); err != nil {
			return errors.New("truncated encrypted Edge backup chunk")
		}
		plain, err := aead.Open(nil, backupNonce(noncePrefix, sequence), ciphertext, backupAAD(headerDigest, sequence))
		if err != nil {
			return errors.New("Edge backup authentication failed; the passphrase is wrong or the backup was changed")
		}
		if len(plain) == 0 || (plain[0] != 0 && plain[0] != 1) {
			return errors.New("invalid authenticated Edge backup chunk")
		}
		if plain[0] == 1 {
			if len(plain) != 1 {
				return errors.New("invalid Edge backup final chunk")
			}
			var trailing [1]byte
			if count, err := input.Read(trailing[:]); err != io.EOF || count != 0 {
				return errors.New("Edge backup has unauthenticated trailing data")
			}
			return nil
		}
		if _, err := destination.Write(plain[1:]); err != nil {
			return err
		}
	}
}

func newBackupHeader(passphrase string) (backupContainerHeader, []byte, []byte, error) {
	salt := make([]byte, 16)
	noncePrefix := make([]byte, 16)
	if _, err := rand.Read(salt); err != nil {
		return backupContainerHeader{}, nil, nil, err
	}
	if _, err := rand.Read(noncePrefix); err != nil {
		return backupContainerHeader{}, nil, nil, err
	}
	header := backupContainerHeader{
		FormatVersion: backupFormatVersion,
		KDF:           "argon2id",
		Salt:          base64.RawStdEncoding.EncodeToString(salt),
		KDFTime:       3,
		KDFMemoryKiB:  64 * 1024,
		KDFThreads:    4,
		Cipher:        "xchacha20-poly1305",
		NoncePrefix:   base64.RawStdEncoding.EncodeToString(noncePrefix),
		ChunkSize:     backupChunkSize,
	}
	key := argon2.IDKey([]byte(passphrase), salt, header.KDFTime, header.KDFMemoryKiB, header.KDFThreads, chacha20poly1305.KeySize)
	return header, key, noncePrefix, nil
}

func validateBackupHeader(header backupContainerHeader) ([]byte, []byte, error) {
	if header.FormatVersion != backupFormatVersion || header.KDF != "argon2id" || header.Cipher != "xchacha20-poly1305" {
		return nil, nil, errors.New("unsupported Edge backup cryptographic format")
	}
	if header.KDFTime < 1 || header.KDFTime > 10 || header.KDFMemoryKiB < 16*1024 || header.KDFMemoryKiB > 256*1024 || header.KDFThreads < 1 || header.KDFThreads > 16 {
		return nil, nil, errors.New("unsafe Edge backup key-derivation parameters")
	}
	if header.ChunkSize < 4096 || header.ChunkSize > 4*1024*1024 {
		return nil, nil, errors.New("invalid Edge backup chunk size")
	}
	salt, err := base64.RawStdEncoding.DecodeString(header.Salt)
	if err != nil || len(salt) != 16 {
		return nil, nil, errors.New("invalid Edge backup salt")
	}
	noncePrefix, err := base64.RawStdEncoding.DecodeString(header.NoncePrefix)
	if err != nil || len(noncePrefix) != 16 {
		return nil, nil, errors.New("invalid Edge backup nonce")
	}
	return salt, noncePrefix, nil
}

func writeEncryptedChunk(
	output io.Writer,
	aead interface {
		Seal([]byte, []byte, []byte, []byte) []byte
		Overhead() int
	},
	noncePrefix []byte,
	headerDigest [sha256.Size]byte,
	sequence uint64,
	plain []byte,
) error {
	ciphertext := aead.Seal(nil, backupNonce(noncePrefix, sequence), plain, backupAAD(headerDigest, sequence))
	if err := binary.Write(output, binary.BigEndian, uint32(len(ciphertext))); err != nil {
		return err
	}
	_, err := output.Write(ciphertext)
	return err
}

func backupNonce(prefix []byte, sequence uint64) []byte {
	nonce := make([]byte, chacha20poly1305.NonceSizeX)
	copy(nonce, prefix)
	binary.BigEndian.PutUint64(nonce[16:], sequence)
	return nonce
}

func backupAAD(headerDigest [sha256.Size]byte, sequence uint64) []byte {
	aad := make([]byte, sha256.Size+8)
	copy(aad, headerDigest[:])
	binary.BigEndian.PutUint64(aad[sha256.Size:], sequence)
	return aad
}

func readBackupManifest(payload io.Reader) (BackupManifest, error) {
	var length uint32
	if err := binary.Read(payload, binary.BigEndian, &length); err != nil || length == 0 || length > maxManifestSize {
		return BackupManifest{}, errors.New("invalid Edge backup manifest length")
	}
	encoded := make([]byte, length)
	if _, err := io.ReadFull(payload, encoded); err != nil {
		return BackupManifest{}, errors.New("truncated Edge backup manifest")
	}
	var manifest BackupManifest
	decoder := json.NewDecoder(bytes.NewReader(encoded))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&manifest); err != nil {
		return BackupManifest{}, errors.New("invalid Edge backup manifest")
	}
	if manifest.FormatVersion != backupFormatVersion || manifest.BackupID == "" || manifest.CreatedAt < 0 || manifest.EdgeID == "" || manifest.SchemaVersion < 1 || manifest.RawRecordCount < 0 {
		return BackupManifest{}, errors.New("invalid Edge backup manifest values")
	}
	if manifest.StorageProfile == "" {
		manifest.StorageProfile = string(ProfileEmbedded)
		manifest.PayloadFormat = "sqlite-database"
	}
	if manifest.StorageProfile != string(ProfileEmbedded) &&
		manifest.StorageProfile != string(ProfilePostgres) {
		return BackupManifest{}, errors.New("invalid Edge backup storage profile")
	}
	decodedHash, err := hex.DecodeString(manifest.DatabaseSHA256)
	if err != nil || len(decodedHash) != sha256.Size {
		return BackupManifest{}, errors.New("invalid Edge backup database checksum")
	}
	return manifest, nil
}

func validateRestoredSnapshot(ctx context.Context, path string, manifest BackupManifest) error {
	info, err := inspectSnapshot(ctx, path)
	if err != nil {
		return err
	}
	return validateSnapshotInfo(info, manifest)
}

func prepareRestoredDatabase(ctx context.Context, path string, manifest BackupManifest) error {
	restored, err := Open(path)
	if err != nil {
		return fmt.Errorf("open restored Edge database: %w", err)
	}
	closed := false
	defer func() {
		if !closed {
			_ = restored.Close()
		}
	}()
	if err := prepareRestoredStore(ctx, restored, manifest); err != nil {
		return err
	}
	if _, err := restored.db.ExecContext(ctx, "PRAGMA wal_checkpoint(TRUNCATE)"); err != nil {
		return err
	}
	if err := restored.Close(); err != nil {
		return err
	}
	closed = true
	file, err := os.OpenFile(path, os.O_RDWR, 0)
	if err != nil {
		return err
	}
	if err := file.Sync(); err != nil {
		_ = file.Close()
		return err
	}
	return file.Close()
}

func prepareRestoredStore(ctx context.Context, restored *Store, manifest BackupManifest) error {
	restoreID, err := randomOperationID("restore_")
	if err != nil {
		return err
	}
	now := time.Now().UnixMilli()
	tx, err := restored.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err := tx.ExecContext(ctx, `
		UPDATE edge_sessions SET revoked_at = ? WHERE revoked_at IS NULL
	`, now); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT OR IGNORE INTO edge_backup_events(
			backup_id, created_at, destination_name,
			database_sha256, raw_record_count
		) VALUES(?, ?, 'restored-backup', ?, ?)
	`, manifest.BackupID, manifest.CreatedAt,
		manifest.DatabaseSHA256, manifest.RawRecordCount); err != nil {
		return err
	}
	for _, cursor := range manifest.Cursors {
		if _, err := tx.ExecContext(ctx, `
			INSERT OR IGNORE INTO edge_backup_cursors(
				backup_id, edge_node_id, ledger_epoch, accepted_through
			) VALUES(?, ?, ?, ?)
		`, manifest.BackupID, cursor.EdgeNodeID,
			cursor.LedgerEpoch, cursor.AcceptedThrough); err != nil {
			return err
		}
	}
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO edge_restore_events(
			restore_id, backup_id, restored_at, backup_created_at, backup_edge_id,
			backup_schema_version, backup_sha256
		) VALUES(?, ?, ?, ?, ?, ?, ?)
	`, restoreID, manifest.BackupID, now, manifest.CreatedAt, manifest.EdgeID, manifest.SchemaVersion, manifest.DatabaseSHA256); err != nil {
		return err
	}
	for _, cursor := range manifest.Cursors {
		if _, err := tx.ExecContext(ctx, `
			INSERT INTO edge_restore_cursor_checks(
				restore_id, edge_node_id, ledger_epoch,
				backup_accepted_through, state, updated_at
			) VALUES(?, ?, ?, ?, 'pending', ?)
		`, restoreID, cursor.EdgeNodeID, cursor.LedgerEpoch, cursor.AcceptedThrough, now); err != nil {
			return err
		}
	}
	restoreSummary, err := json.Marshal(struct {
		BackupID       string `json:"backup_id"`
		DatabaseSHA256 string `json:"database_sha256"`
		RawRecordCount int64  `json:"raw_record_count"`
	}{manifest.BackupID, manifest.DatabaseSHA256, manifest.RawRecordCount})
	if err != nil {
		return err
	}
	actor := edgeapp.LocalCLIActor()
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt: now, ActorClass: actor.Class, ActorRef: actor.Ref,
		Operation: "edge_backup.restore", ResourceRef: restoreID,
		Outcome: auditOutcomeSuccess, Summary: restoreSummary,
	}); err != nil {
		return err
	}
	if err := tx.Commit(); err != nil {
		return err
	}
	return nil
}

func pendingRestoredCursorCheckTx(
	ctx context.Context,
	tx *sqlTx,
	edgeNodeID string,
	ledgerEpoch string,
) (string, bool, error) {
	var restoreID string
	err := tx.QueryRowContext(ctx, `
		SELECT check_state.restore_id
		FROM edge_restore_cursor_checks AS check_state
		JOIN edge_restore_events AS event
			ON event.restore_id = check_state.restore_id
		WHERE check_state.edge_node_id = ? AND check_state.ledger_epoch = ?
			AND check_state.state = 'pending'
		ORDER BY event.restored_at DESC, check_state.restore_id DESC
		LIMIT 1
	`, edgeNodeID, ledgerEpoch).Scan(&restoreID)
	if errors.Is(err, sql.ErrNoRows) {
		return "", false, nil
	}
	return restoreID, err == nil, err
}

func (store *Store) ApplyRestoredArchiveLoss(
	ctx context.Context,
	actor edgeapp.Actor,
	edgeNodeID string,
	ledgerEpoch string,
	confirmedEdgeID string,
	reason string,
) error {
	if err := actor.Validate(); err != nil {
		return err
	}
	if actor.Class != edgeapp.ActorLocalCLI {
		return edgeapp.ErrForbidden
	}
	if strings.TrimSpace(edgeNodeID) == "" || strings.TrimSpace(ledgerEpoch) == "" {
		return errors.New("Edge Node ID and ledger epoch are required")
	}
	if strings.TrimSpace(reason) == "" {
		return errors.New("archive-loss reason is required")
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var edgeID string
	if err := tx.QueryRowContext(ctx, `
		SELECT edge_id FROM edge_meta WHERE singleton = 1
	`).Scan(&edgeID); err != nil {
		return err
	}
	if confirmedEdgeID != edgeID {
		return errors.New("confirmed IoTKit Edge ID does not match this IoTKit Edge")
	}
	var restoreID string
	var observedCursorStart int64
	err = tx.QueryRowContext(ctx, `
		SELECT check_state.restore_id, check_state.observed_cursor_start
		FROM edge_restore_cursor_checks AS check_state
		JOIN edge_restore_events AS event
			ON event.restore_id = check_state.restore_id
		WHERE check_state.edge_node_id = ? AND check_state.ledger_epoch = ?
			AND check_state.state = 'recovery_required'
		ORDER BY event.restored_at DESC, check_state.restore_id DESC
		LIMIT 1
	`, edgeNodeID, ledgerEpoch).Scan(&restoreID, &observedCursorStart)
	if errors.Is(err, sql.ErrNoRows) {
		return errors.New("no restored archive-loss decision is pending for this Edge Node stream")
	}
	if err != nil {
		return err
	}
	acceptedThrough := observedCursorStart - 1
	now := time.Now().UnixMilli()
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO accepted_cursors(edge_node_id, ledger_epoch, accepted_through, updated_at)
		VALUES(?, ?, ?, ?)
		ON CONFLICT(edge_node_id, ledger_epoch) DO UPDATE SET
			accepted_through = excluded.accepted_through,
			updated_at = excluded.updated_at
	`, edgeNodeID, ledgerEpoch, acceptedThrough, now); err != nil {
		return err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE edge_restore_cursor_checks
		SET state = 'archive_lost', updated_at = ?
		WHERE restore_id = ? AND edge_node_id = ? AND ledger_epoch = ?
			AND state = 'recovery_required'
	`, now, restoreID, edgeNodeID, ledgerEpoch); err != nil {
		return err
	}
	result, err := tx.ExecContext(ctx, `
		UPDATE edge_node_activations
		SET state = 'active', revision = revision + 1, updated_at = ?
		WHERE edge_node_id = ? AND ledger_epoch = ? AND state = 'recovery_hold'
	`, now, edgeNodeID, ledgerEpoch)
	if err != nil {
		return err
	}
	if changed, err := result.RowsAffected(); err != nil {
		return err
	} else if changed != 1 {
		return errors.New("Edge Node is not in recovery hold for this restored cursor")
	}
	summary, err := json.Marshal(struct {
		RestoreID       string `json:"restore_id"`
		LedgerEpoch     string `json:"ledger_epoch"`
		AcceptedThrough int64  `json:"accepted_through"`
		Reason          string `json:"reason"`
	}{restoreID, ledgerEpoch, acceptedThrough, reason})
	if err != nil {
		return err
	}
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt: now, ActorClass: actor.Class, ActorRef: actor.Ref,
		Operation: "edge_restore.accept_archive_loss", ResourceRef: edgeNodeID,
		Outcome: auditOutcomeSuccess, Summary: summary,
	}); err != nil {
		return err
	}
	return tx.Commit()
}

func randomOperationID(prefix string) (string, error) {
	value := make([]byte, 16)
	if _, err := rand.Read(value); err != nil {
		return "", err
	}
	return prefix + hex.EncodeToString(value), nil
}

func fileSHA256(path string) ([sha256.Size]byte, error) {
	var empty [sha256.Size]byte
	file, err := os.Open(path)
	if err != nil {
		return empty, err
	}
	defer file.Close()
	hash := sha256.New()
	if _, err := io.Copy(hash, file); err != nil {
		return empty, err
	}
	copy(empty[:], hash.Sum(nil))
	return empty, nil
}

func requireAbsentPath(path string, label string) error {
	if strings.TrimSpace(path) == "" {
		return fmt.Errorf("%s is required", label)
	}
	if _, err := os.Lstat(path); err == nil {
		return fmt.Errorf("%s already exists", label)
	} else if !errors.Is(err, os.ErrNotExist) {
		return fmt.Errorf("inspect %s: %w", label, err)
	}
	return nil
}

func installNewFile(temporaryPath string, destination string, label string) error {
	if err := os.Link(temporaryPath, destination); err != nil {
		if errors.Is(err, os.ErrExist) {
			return fmt.Errorf("%s already exists", label)
		}
		return fmt.Errorf("install %s: %w", label, err)
	}
	if err := os.Remove(temporaryPath); err != nil {
		return fmt.Errorf("remove installed staging link: %w", err)
	}
	if err := syncDirectory(filepath.Dir(destination)); err != nil {
		_ = os.Remove(destination)
		_ = syncDirectory(filepath.Dir(destination))
		return fmt.Errorf("persist installed %s: %w", label, err)
	}
	return nil
}

func validateBackupPassphrase(passphrase string) error {
	if !utf8.ValidString(passphrase) || utf8.RuneCountInString(passphrase) < 12 {
		return errors.New("backup passphrase must contain at least 12 characters")
	}
	if utf8.RuneCountInString(passphrase) > 1024 {
		return errors.New("backup passphrase is too long")
	}
	return nil
}

func syncDirectory(path string) error {
	directory, err := os.Open(path)
	if err != nil {
		return err
	}
	defer directory.Close()
	return directory.Sync()
}
