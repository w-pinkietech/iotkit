package store

import (
	"context"
	"database/sql"
	"errors"
	"strings"
)

type AccountRole string

const (
	AccountRoleViewer      AccountRole = "viewer"
	AccountRoleAdmin       AccountRole = "admin"
	AccountRoleSystemAdmin AccountRole = "system_admin"
)

type AccountState string

const (
	AccountStateActive   AccountState = "active"
	AccountStateDisabled AccountState = "disabled"
)

var (
	ErrAccountNotFound        = errors.New("Site account not found")
	ErrAccountLoginIDConflict = errors.New("Site account login ID already exists")
	ErrSessionNotFound        = errors.New("Site session not found")
)

type AccountRecord struct {
	AccountRef         string
	LoginID            string
	LoginIDNormalized  string
	DisplayName        string
	PasswordPHC        string
	Role               AccountRole
	State              AccountState
	MustChangePassword bool
	CreatedAt          int64
	UpdatedAt          int64
	DisabledAt         *int64
}

type SessionRecord struct {
	SessionRef        string
	TokenSHA256       []byte
	CSRFSHA256        []byte
	AccountRef        string
	IssuedAt          int64
	LastSeenAt        int64
	IdleExpiresAt     int64
	AbsoluteExpiresAt int64
	RevokedAt         *int64
}

type ActiveSession struct {
	SessionRecord
	Account AccountRecord
}

func (store *Store) CreateAccount(ctx context.Context, account AccountRecord) error {
	_, err := store.db.ExecContext(ctx, `
		INSERT INTO site_accounts (
			account_ref, login_id, login_id_normalized, display_name,
			password_phc, role, state, must_change_password,
			created_at, updated_at, disabled_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, account.AccountRef, account.LoginID, account.LoginIDNormalized, account.DisplayName,
		account.PasswordPHC, account.Role, account.State, account.MustChangePassword,
		account.CreatedAt, account.UpdatedAt, account.DisabledAt)
	if err != nil && strings.Contains(err.Error(), "site_accounts.login_id_normalized") {
		return ErrAccountLoginIDConflict
	}
	return err
}

func (store *Store) GetAccountByLoginID(ctx context.Context, normalizedLoginID string) (AccountRecord, error) {
	return scanAccount(store.db.QueryRowContext(ctx, `
		SELECT account_ref, login_id, login_id_normalized, display_name,
			password_phc, role, state, must_change_password,
			created_at, updated_at, disabled_at
		FROM site_accounts
		WHERE login_id_normalized = ?
	`, normalizedLoginID))
}

func (store *Store) DisableAccount(ctx context.Context, accountRef string, disabledAt int64) error {
	result, err := store.db.ExecContext(ctx, `
		UPDATE site_accounts
		SET state = 'disabled', disabled_at = ?, updated_at = ?
		WHERE account_ref = ? AND state = 'active'
	`, disabledAt, disabledAt, accountRef)
	if err != nil {
		return err
	}
	updated, err := result.RowsAffected()
	if err != nil {
		return err
	}
	if updated == 0 {
		return ErrAccountNotFound
	}
	return nil
}

func (store *Store) CreateSession(ctx context.Context, session SessionRecord) error {
	_, err := store.db.ExecContext(ctx, `
		INSERT INTO site_sessions (
			session_ref, token_sha256, csrf_sha256, account_ref,
			issued_at, last_seen_at, idle_expires_at, absolute_expires_at, revoked_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, session.SessionRef, session.TokenSHA256, session.CSRFSHA256, session.AccountRef,
		session.IssuedAt, session.LastSeenAt, session.IdleExpiresAt,
		session.AbsoluteExpiresAt, session.RevokedAt)
	return err
}

func (store *Store) ReplaceAccountPassword(
	ctx context.Context,
	accountRef string,
	passwordPHC string,
	mustChangePassword bool,
	updatedAt int64,
) error {
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer func() { _ = tx.Rollback() }()

	result, err := tx.ExecContext(ctx, `
		UPDATE site_accounts
		SET password_phc = ?, must_change_password = ?, updated_at = ?
		WHERE account_ref = ?
	`, passwordPHC, mustChangePassword, updatedAt, accountRef)
	if err != nil {
		return err
	}
	updated, err := result.RowsAffected()
	if err != nil {
		return err
	}
	if updated == 0 {
		return ErrAccountNotFound
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE site_sessions
		SET revoked_at = ?
		WHERE account_ref = ? AND revoked_at IS NULL
	`, updatedAt, accountRef); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) GetActiveSessionByTokenHash(
	ctx context.Context,
	tokenSHA256 []byte,
	now int64,
) (ActiveSession, error) {
	row := store.db.QueryRowContext(ctx, `
		SELECT
			s.session_ref, s.token_sha256, s.csrf_sha256, s.account_ref,
			s.issued_at, s.last_seen_at, s.idle_expires_at, s.absolute_expires_at, s.revoked_at,
			a.account_ref, a.login_id, a.login_id_normalized, a.display_name,
			a.password_phc, a.role, a.state, a.must_change_password,
			a.created_at, a.updated_at, a.disabled_at
		FROM site_sessions s
		JOIN site_accounts a ON a.account_ref = s.account_ref
		WHERE s.token_sha256 = ?
			AND s.revoked_at IS NULL
			AND s.idle_expires_at > ?
			AND s.absolute_expires_at > ?
			AND a.state = 'active'
	`, tokenSHA256, now, now)

	var active ActiveSession
	var revokedAt sql.NullInt64
	var disabledAt sql.NullInt64
	if err := row.Scan(
		&active.SessionRef,
		&active.TokenSHA256,
		&active.CSRFSHA256,
		&active.SessionRecord.AccountRef,
		&active.IssuedAt,
		&active.LastSeenAt,
		&active.IdleExpiresAt,
		&active.AbsoluteExpiresAt,
		&revokedAt,
		&active.Account.AccountRef,
		&active.Account.LoginID,
		&active.Account.LoginIDNormalized,
		&active.Account.DisplayName,
		&active.Account.PasswordPHC,
		&active.Account.Role,
		&active.Account.State,
		&active.Account.MustChangePassword,
		&active.Account.CreatedAt,
		&active.Account.UpdatedAt,
		&disabledAt,
	); errors.Is(err, sql.ErrNoRows) {
		return ActiveSession{}, ErrSessionNotFound
	} else if err != nil {
		return ActiveSession{}, err
	}
	active.RevokedAt = nullableInt64(revokedAt)
	active.Account.DisabledAt = nullableInt64(disabledAt)
	return active, nil
}

func (store *Store) TouchSession(
	ctx context.Context,
	sessionRef string,
	lastSeenAt int64,
	idleExpiresAt int64,
) error {
	result, err := store.db.ExecContext(ctx, `
		UPDATE site_sessions
		SET last_seen_at = ?, idle_expires_at = ?
		WHERE session_ref = ?
			AND revoked_at IS NULL
			AND absolute_expires_at > ?
	`, lastSeenAt, idleExpiresAt, sessionRef, lastSeenAt)
	if err != nil {
		return err
	}
	updated, err := result.RowsAffected()
	if err != nil {
		return err
	}
	if updated == 0 {
		return ErrSessionNotFound
	}
	return nil
}

func (store *Store) RevokeSession(ctx context.Context, sessionRef string, revokedAt int64) error {
	result, err := store.db.ExecContext(ctx, `
		UPDATE site_sessions
		SET revoked_at = ?
		WHERE session_ref = ? AND revoked_at IS NULL
	`, revokedAt, sessionRef)
	if err != nil {
		return err
	}
	updated, err := result.RowsAffected()
	if err != nil {
		return err
	}
	if updated == 0 {
		return ErrSessionNotFound
	}
	return nil
}

func (store *Store) RevokeAccountSessions(ctx context.Context, accountRef string, revokedAt int64) error {
	_, err := store.db.ExecContext(ctx, `
		UPDATE site_sessions
		SET revoked_at = ?
		WHERE account_ref = ? AND revoked_at IS NULL
	`, revokedAt, accountRef)
	return err
}

type rowScanner interface {
	Scan(...any) error
}

func scanAccount(row rowScanner) (AccountRecord, error) {
	var account AccountRecord
	var disabledAt sql.NullInt64
	if err := row.Scan(
		&account.AccountRef,
		&account.LoginID,
		&account.LoginIDNormalized,
		&account.DisplayName,
		&account.PasswordPHC,
		&account.Role,
		&account.State,
		&account.MustChangePassword,
		&account.CreatedAt,
		&account.UpdatedAt,
		&disabledAt,
	); errors.Is(err, sql.ErrNoRows) {
		return AccountRecord{}, ErrAccountNotFound
	} else if err != nil {
		return AccountRecord{}, err
	}
	account.DisabledAt = nullableInt64(disabledAt)
	return account, nil
}

func nullableInt64(value sql.NullInt64) *int64 {
	if !value.Valid {
		return nil
	}
	return &value.Int64
}
