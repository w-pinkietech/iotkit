package store

import (
	"context"
	"database/sql"
	"errors"
	"os"
	"path/filepath"
	"syscall"
	"time"
)

type StorageState string

const (
	StorageHealthy     StorageState = "healthy"
	StorageWarning     StorageState = "warning"
	StorageCritical    StorageState = "critical"
	StorageUnavailable StorageState = "unavailable"
)

type StorageStatus struct {
	Profile                  Profile      `json:"profile"`
	State                    StorageState `json:"state"`
	FilesystemAvailable      bool         `json:"filesystem_available"`
	DatabaseBytes            int64        `json:"database_bytes"`
	ReclaimableBytes         int64        `json:"reclaimable_bytes"`
	DiskTotalBytes           uint64       `json:"disk_total_bytes"`
	DiskAvailableBytes       uint64       `json:"disk_available_bytes"`
	DiskUsedPercent          int          `json:"disk_used_percent"`
	WarningPercent           int          `json:"warning_percent"`
	RawRecordCount           int64        `json:"raw_record_count"`
	SemanticObservationCount int64        `json:"semantic_observation_count"`
	PendingOutputCount       int64        `json:"pending_output_count"`
	ProjectionFailureCount   int64        `json:"projection_failure_count"`
	LastBackupID             string       `json:"last_backup_id,omitempty"`
	LastBackupAt             *int64       `json:"last_backup_at,omitempty"`
	LastBackupRawRecordCount int64        `json:"last_backup_raw_record_count,omitempty"`
	BackupProtectedRawCount  int64        `json:"backup_protected_raw_count"`
	UnprotectedRawCount      int64        `json:"unprotected_raw_count"`
	AutomaticRawPurgeEnabled bool         `json:"automatic_raw_purge_enabled"`
	GrowthBytesPerDay        int64        `json:"growth_bytes_per_day"`
	EstimatedDaysRemaining   *int64       `json:"estimated_days_remaining,omitempty"`
	AbsoluteReserveState     string       `json:"absolute_reserve_state"`
}

var storageStatusNow = time.Now

func (store *Store) GetStorageStatus(
	ctx context.Context,
	warningPercent int,
) (StorageStatus, error) {
	if warningPercent < 50 || warningPercent > 99 {
		return StorageStatus{}, errors.New("storage warning percent must be between 50 and 99")
	}
	status := StorageStatus{
		Profile: store.profile, State: StorageUnavailable,
		WarningPercent: warningPercent,
	}
	if store.profile == ProfilePostgres {
		return store.getPostgresStorageStatus(ctx, status)
	}
	return store.getEmbeddedStorageStatus(ctx, status)
}

func (store *Store) getEmbeddedStorageStatus(
	ctx context.Context,
	status StorageStatus,
) (StorageStatus, error) {
	var pageCount, pageSize, freePages int64
	if err := store.db.QueryRowContext(ctx, "PRAGMA page_count").Scan(&pageCount); err != nil {
		return StorageStatus{}, err
	}
	if err := store.db.QueryRowContext(ctx, "PRAGMA page_size").Scan(&pageSize); err != nil {
		return StorageStatus{}, err
	}
	if err := store.db.QueryRowContext(ctx, "PRAGMA freelist_count").Scan(&freePages); err != nil {
		return StorageStatus{}, err
	}
	status.DatabaseBytes = pageCount * pageSize
	status.ReclaimableBytes = freePages * pageSize
	if err := store.populateStorageCounts(ctx, &status); err != nil {
		return StorageStatus{}, err
	}

	var sequence int
	var name, databasePath string
	rows, err := store.db.QueryContext(ctx, "PRAGMA database_list")
	if err != nil {
		return StorageStatus{}, err
	}
	for rows.Next() {
		if err := rows.Scan(&sequence, &name, &databasePath); err != nil {
			_ = rows.Close()
			return StorageStatus{}, err
		}
		if name == "main" {
			break
		}
	}
	if err := rows.Close(); err != nil {
		return StorageStatus{}, err
	}
	if databasePath == "" {
		return status, nil
	}
	for _, suffix := range []string{"-wal", "-shm"} {
		if info, err := os.Stat(databasePath + suffix); err == nil {
			status.DatabaseBytes += info.Size()
		}
	}

	var filesystem syscall.Statfs_t
	if err := syscall.Statfs(filepath.Dir(databasePath), &filesystem); err != nil {
		return status, nil
	}
	status.FilesystemAvailable = true
	status.DiskTotalBytes = filesystem.Blocks * uint64(filesystem.Bsize)
	status.DiskAvailableBytes = filesystem.Bavail * uint64(filesystem.Bsize)
	if status.DiskTotalBytes > 0 {
		used := status.DiskTotalBytes - status.DiskAvailableBytes
		status.DiskUsedPercent = int(used * 100 / status.DiskTotalBytes)
	}
	status.State = StorageHealthy
	if status.DiskUsedPercent >= status.WarningPercent {
		status.State = StorageWarning
	}
	if status.DiskUsedPercent >= 97 {
		status.State = StorageCritical
	}
	status.AbsoluteReserveState = "adequate"
	if status.DiskAvailableBytes < 2*1024*1024*1024 {
		status.AbsoluteReserveState = "warning"
	}
	if status.DiskAvailableBytes < 512*1024*1024 {
		status.AbsoluteReserveState = "critical"
	}
	store.recordStorageGrowth(ctx, &status)
	return status, nil
}

func (store *Store) populateStorageCounts(
	ctx context.Context,
	status *StorageStatus,
) error {

	counts := []struct {
		query  string
		target *int64
	}{
		{"SELECT count(*) FROM raw_records", &status.RawRecordCount},
		{"SELECT count(*) FROM semantic_observations_v3", &status.SemanticObservationCount},
		{"SELECT count(*) FROM output_outbox_v3 WHERE published_at IS NULL", &status.PendingOutputCount},
		{"SELECT count(*) FROM semantic_projection_failures_v3", &status.ProjectionFailureCount},
	}
	for _, count := range counts {
		if err := store.db.QueryRowContext(ctx, count.query).Scan(count.target); err != nil {
			return err
		}
	}
	var lastBackupAt int64
	err := store.db.QueryRowContext(ctx, `
		SELECT backup_id, created_at, raw_record_count
		FROM edge_backup_events
		ORDER BY created_at DESC, backup_id DESC
		LIMIT 1
	`).Scan(&status.LastBackupID, &lastBackupAt, &status.LastBackupRawRecordCount)
	if err == nil {
		status.LastBackupAt = &lastBackupAt
		if err := store.db.QueryRowContext(ctx, `
			SELECT count(*)
			FROM raw_records AS raw
			JOIN edge_backup_cursors AS cursor
				ON cursor.backup_id = ?
				AND cursor.edge_node_id = raw.edge_node_id
				AND cursor.ledger_epoch = raw.ledger_epoch
				AND raw.pub_seq <= cursor.accepted_through
		`, status.LastBackupID).Scan(&status.BackupProtectedRawCount); err != nil {
			return err
		}
	} else if !errors.Is(err, sql.ErrNoRows) {
		return err
	}
	status.UnprotectedRawCount = status.RawRecordCount - status.BackupProtectedRawCount
	return nil
}

func (store *Store) getPostgresStorageStatus(
	ctx context.Context,
	status StorageStatus,
) (StorageStatus, error) {
	if err := store.db.QueryRowContext(ctx,
		"SELECT pg_database_size(current_database())",
	).Scan(&status.DatabaseBytes); err != nil {
		return StorageStatus{}, err
	}
	if err := store.populateStorageCounts(ctx, &status); err != nil {
		return StorageStatus{}, err
	}
	// PostgreSQL cannot report filesystem free space through the SQL contract.
	// Keep capacity state unknown instead of presenting a false healthy signal.
	status.State = StorageUnavailable
	status.AbsoluteReserveState = "unknown"
	store.recordStorageGrowth(ctx, &status)
	return status, nil
}

func (store *Store) recordStorageGrowth(ctx context.Context, status *StorageStatus) {
	now := storageStatusNow().Truncate(time.Hour).UnixMilli()
	var sampledAt, databaseBytes int64
	err := store.db.QueryRowContext(ctx, `
		SELECT sampled_at, database_bytes FROM edge_storage_samples
		WHERE sampled_at <= ? AND sampled_at >= ?
		ORDER BY sampled_at ASC LIMIT 1
	`, now-6*time.Hour.Milliseconds(), now-7*24*time.Hour.Milliseconds()).Scan(&sampledAt, &databaseBytes)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return
	}
	if err == nil && now > sampledAt && status.DatabaseBytes > databaseBytes {
		status.GrowthBytesPerDay = (status.DatabaseBytes - databaseBytes) *
			(24 * time.Hour.Milliseconds()) / (now - sampledAt)
		if status.FilesystemAvailable && status.GrowthBytesPerDay > 0 {
			days := int64(status.DiskAvailableBytes) / status.GrowthBytesPerDay
			status.EstimatedDaysRemaining = &days
		}
	}
	if _, err := store.db.ExecContext(ctx, `
		INSERT INTO edge_storage_samples(sampled_at, database_bytes, raw_record_count)
		VALUES(?, ?, ?)
		ON CONFLICT(sampled_at) DO UPDATE SET
			database_bytes = excluded.database_bytes,
			raw_record_count = excluded.raw_record_count
	`, now, status.DatabaseBytes, status.RawRecordCount); err != nil {
		return
	}
	_, _ = store.db.ExecContext(ctx,
		"DELETE FROM edge_storage_samples WHERE sampled_at < ?",
		now-30*24*time.Hour.Milliseconds(),
	)
}
