package store

import (
	"errors"
	"os"
	"syscall"
)

// SQLiteDeploymentLock uses the database inode itself so symlink and hard-link
// aliases participate in the same operational lock. Normal IoTKit processes
// take a shared lock; the offline profile migration takes an exclusive lock.
type SQLiteDeploymentLock struct {
	file *os.File
}

func acquireSQLiteSharedDeploymentLock(databasePath string) (*SQLiteDeploymentLock, error) {
	return acquireSQLiteDeploymentLock(databasePath, syscall.LOCK_SH, true)
}

func AcquireSQLiteDeploymentLock(databasePath string) (*SQLiteDeploymentLock, error) {
	return acquireSQLiteDeploymentLock(databasePath, syscall.LOCK_EX, false)
}

func acquireSQLiteDeploymentLock(databasePath string, mode int, create bool) (*SQLiteDeploymentLock, error) {
	if databasePath == "" || databasePath == ":memory:" {
		return nil, errors.New("SQLite deployment lock requires a filesystem database path")
	}
	flags := os.O_RDWR
	if create {
		flags |= os.O_CREATE
	}
	file, err := os.OpenFile(databasePath, flags, 0o600)
	if err != nil {
		return nil, errors.New("open SQLite database for deployment lock")
	}
	if err := syscall.Flock(int(file.Fd()), mode|syscall.LOCK_NB); err != nil {
		_ = file.Close()
		return nil, errors.New("SQLite database is in use by another IoTKit operation")
	}
	return &SQLiteDeploymentLock{file: file}, nil
}

func (lock *SQLiteDeploymentLock) Close() error {
	if lock == nil || lock.file == nil {
		return nil
	}
	err := syscall.Flock(int(lock.file.Fd()), syscall.LOCK_UN)
	closeErr := lock.file.Close()
	lock.file = nil
	if err != nil {
		return err
	}
	return closeErr
}
