package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"strings"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
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
	ErrAccountNotFound        = errors.New("Edge account not found")
	ErrAccountLoginIDConflict = errors.New("Edge account login ID already exists")
	ErrSessionNotFound        = errors.New("Edge session not found")
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
	Revision           int64
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
		INSERT INTO edge_accounts (
			account_ref, login_id, login_id_normalized, display_name,
			password_phc, role, state, must_change_password,
			created_at, updated_at, disabled_at, revision
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, account.AccountRef, account.LoginID, account.LoginIDNormalized, account.DisplayName,
		account.PasswordPHC, account.Role, account.State, account.MustChangePassword,
		account.CreatedAt, account.UpdatedAt, account.DisabledAt, normalizedRevision(account.Revision))
	if err != nil && strings.Contains(err.Error(), "edge_accounts.login_id_normalized") {
		return ErrAccountLoginIDConflict
	}
	return err
}

func (store *Store) GetAccountByLoginID(ctx context.Context, normalizedLoginID string) (AccountRecord, error) {
	return scanAccount(store.db.QueryRowContext(ctx, `
		SELECT account_ref, login_id, login_id_normalized, display_name,
			password_phc, role, state, must_change_password,
			created_at, updated_at, disabled_at, revision
		FROM edge_accounts
		WHERE login_id_normalized = ?
	`, normalizedLoginID))
}

func (store *Store) DisableAccount(ctx context.Context, accountRef string, disabledAt int64) error {
	result, err := store.db.ExecContext(ctx, `
		UPDATE edge_accounts
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
		INSERT INTO edge_sessions (
			session_ref, token_sha256, csrf_sha256, account_ref,
			issued_at, last_seen_at, idle_expires_at, absolute_expires_at, revoked_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, session.SessionRef, session.TokenSHA256, session.CSRFSHA256, session.AccountRef,
		session.IssuedAt, session.LastSeenAt, session.IdleExpiresAt,
		session.AbsoluteExpiresAt, session.RevokedAt)
	return err
}

func (store *Store) CreateSessionWithAudit(
	ctx context.Context,
	session SessionRecord,
	event edgeapp.AuditEvent,
) error {
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer func() { _ = tx.Rollback() }()
	if _, err := tx.ExecContext(ctx, `
		INSERT INTO edge_sessions (
			session_ref, token_sha256, csrf_sha256, account_ref,
			issued_at, last_seen_at, idle_expires_at, absolute_expires_at, revoked_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, session.SessionRef, session.TokenSHA256, session.CSRFSHA256, session.AccountRef,
		session.IssuedAt, session.LastSeenAt, session.IdleExpiresAt,
		session.AbsoluteExpiresAt, session.RevokedAt); err != nil {
		return err
	}
	if err := insertAuditEventTx(ctx, tx, event); err != nil {
		return err
	}
	return tx.Commit()
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
		UPDATE edge_accounts
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
		UPDATE edge_sessions
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
		FROM edge_sessions s
		JOIN edge_accounts a ON a.account_ref = s.account_ref
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
		UPDATE edge_sessions
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
		UPDATE edge_sessions
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

func (store *Store) RevokeSessionWithAudit(
	ctx context.Context,
	sessionRef string,
	revokedAt int64,
	event edgeapp.AuditEvent,
) error {
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer func() { _ = tx.Rollback() }()
	result, err := tx.ExecContext(ctx, `
		UPDATE edge_sessions
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
	if err := insertAuditEventTx(ctx, tx, event); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) RevokeAccountSessions(ctx context.Context, accountRef string, revokedAt int64) error {
	_, err := store.db.ExecContext(ctx, `
		UPDATE edge_sessions
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
		&account.Revision,
	); errors.Is(err, sql.ErrNoRows) {
		return AccountRecord{}, ErrAccountNotFound
	} else if err != nil {
		return AccountRecord{}, err
	}
	account.DisabledAt = nullableInt64(disabledAt)
	return account, nil
}

func normalizedRevision(revision int64) int64 {
	if revision < 1 {
		return 1
	}
	return revision
}

func (store *Store) CreateEdgeAccount(
	ctx context.Context,
	actor edgeapp.Actor,
	provision edgeapp.AccountProvision,
) (edgeapp.Account, error) {
	var noAccount edgeapp.Account
	if err := actor.Validate(); err != nil {
		return noAccount, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noAccount, err
	}
	defer func() { _ = tx.Rollback() }()

	if provision.RequireUnowned {
		var count int
		if err := tx.QueryRowContext(ctx, "SELECT count(*) FROM edge_accounts").Scan(&count); err != nil {
			return noAccount, err
		}
		if count != 0 {
			return noAccount, edgeapp.ErrAlreadyOwned
		}
	}

	accountRef, err := newResourceRef("acct_")
	if err != nil {
		return noAccount, err
	}
	now := time.Now().UnixMilli()
	account := edgeapp.Account{
		AccountRef:         accountRef,
		LoginID:            provision.LoginID,
		DisplayName:        provision.DisplayName,
		Role:               provision.Role,
		State:              edgeapp.AccountStateActive,
		MustChangePassword: provision.MustChangePassword,
		Revision:           1,
		CreatedAt:          now,
		UpdatedAt:          now,
	}
	_, err = tx.ExecContext(ctx, `
		INSERT INTO edge_accounts (
			account_ref, login_id, login_id_normalized, display_name,
			password_phc, role, state, must_change_password,
			created_at, updated_at, disabled_at, revision
		) VALUES (?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, NULL, 1)
	`, account.AccountRef, account.LoginID, account.LoginID, account.DisplayName,
		provision.PasswordPHC, account.Role, account.MustChangePassword,
		account.CreatedAt, account.UpdatedAt)
	if err != nil {
		if strings.Contains(err.Error(), "edge_accounts.login_id_normalized") {
			return noAccount, ErrAccountLoginIDConflict
		}
		return noAccount, err
	}
	summary, err := json.Marshal(struct {
		LoginID            string              `json:"login_id"`
		DisplayName        string              `json:"display_name"`
		Role               edgeapp.AccountRole `json:"role"`
		MustChangePassword bool                `json:"must_change_password"`
	}{
		LoginID:            account.LoginID,
		DisplayName:        account.DisplayName,
		Role:               account.Role,
		MustChangePassword: account.MustChangePassword,
	})
	if err != nil {
		return noAccount, err
	}
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt:  now,
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "account.create",
		ResourceRef: account.AccountRef,
		Outcome:     auditOutcomeSuccess,
		Summary:     summary,
	}); err != nil {
		return noAccount, err
	}
	if err := tx.Commit(); err != nil {
		return noAccount, err
	}
	return account, nil
}

func (store *Store) GetEdgeAccount(ctx context.Context, accountRef string) (edgeapp.Account, error) {
	return scanEdgeAccount(store.db.QueryRowContext(ctx, `
		SELECT account_ref, login_id, display_name, role, state,
			must_change_password, revision, created_at, updated_at
		FROM edge_accounts
		WHERE account_ref = ?
	`, accountRef))
}

func (store *Store) GetEdgeAccountByLoginID(
	ctx context.Context,
	normalizedLoginID string,
) (edgeapp.Account, error) {
	return scanEdgeAccount(store.db.QueryRowContext(ctx, `
		SELECT account_ref, login_id, display_name, role, state,
			must_change_password, revision, created_at, updated_at
		FROM edge_accounts
		WHERE login_id_normalized = ?
	`, normalizedLoginID))
}

func (store *Store) GetEdgeAccountCredential(
	ctx context.Context,
	accountRef string,
) (edgeapp.AccountCredential, error) {
	var credential edgeapp.AccountCredential
	var err error
	credential.Account, err = scanEdgeAccount(store.db.QueryRowContext(ctx, `
		SELECT account_ref, login_id, display_name, role, state,
			must_change_password, revision, created_at, updated_at
		FROM edge_accounts
		WHERE account_ref = ?
	`, accountRef))
	if err != nil {
		return edgeapp.AccountCredential{}, err
	}
	if err := store.db.QueryRowContext(ctx, `
		SELECT password_phc FROM edge_accounts WHERE account_ref = ?
	`, accountRef).Scan(&credential.PasswordPHC); err != nil {
		return edgeapp.AccountCredential{}, err
	}
	return credential, nil
}

func (store *Store) ListEdgeAccounts(ctx context.Context) ([]edgeapp.Account, error) {
	rows, err := store.db.QueryContext(ctx, `
		SELECT account_ref, login_id, display_name, role, state,
			must_change_password, revision, created_at, updated_at
		FROM edge_accounts
		ORDER BY login_id_normalized, account_ref
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	accounts := make([]edgeapp.Account, 0)
	for rows.Next() {
		account, err := scanEdgeAccount(rows)
		if err != nil {
			return nil, err
		}
		accounts = append(accounts, account)
	}
	return accounts, rows.Err()
}

func (store *Store) UpdateEdgeAccount(
	ctx context.Context,
	actor edgeapp.Actor,
	accountRef string,
	displayName string,
	role edgeapp.AccountRole,
	expectedRevision int64,
) (edgeapp.Account, error) {
	var noAccount edgeapp.Account
	if err := actor.Validate(); err != nil {
		return noAccount, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noAccount, err
	}
	defer func() { _ = tx.Rollback() }()

	account, err := scanEdgeAccount(tx.QueryRowContext(ctx, `
		SELECT account_ref, login_id, display_name, role, state,
			must_change_password, revision, created_at, updated_at
		FROM edge_accounts
		WHERE account_ref = ?
	`, accountRef))
	if err != nil {
		return noAccount, err
	}
	if account.Revision != expectedRevision {
		return noAccount, edgeapp.ErrRevisionMismatch
	}
	if account.State != edgeapp.AccountStateActive {
		return noAccount, edgeapp.ErrNotFound
	}
	roleChanged := account.Role != role
	if account.Role == edgeapp.AccountRoleSystemAdmin &&
		role != edgeapp.AccountRoleSystemAdmin {
		var activeSystemAdmins int
		if err := tx.QueryRowContext(ctx, `
			SELECT count(*) FROM edge_accounts
			WHERE role = 'system_admin' AND state = 'active'
		`).Scan(&activeSystemAdmins); err != nil {
			return noAccount, err
		}
		if activeSystemAdmins <= 1 {
			return noAccount, edgeapp.ErrLastSystemAdmin
		}
	}
	now := time.Now().UnixMilli()
	account.DisplayName = displayName
	account.Role = role
	account.Revision++
	account.UpdatedAt = now
	if _, err := tx.ExecContext(ctx, `
		UPDATE edge_accounts
		SET display_name = ?, role = ?, revision = ?, updated_at = ?
		WHERE account_ref = ? AND revision = ? AND state = 'active'
	`, account.DisplayName, account.Role, account.Revision, now,
		accountRef, expectedRevision); err != nil {
		return noAccount, err
	}
	if roleChanged {
		if _, err := tx.ExecContext(ctx, `
			UPDATE edge_sessions SET revoked_at = ?
			WHERE account_ref = ? AND revoked_at IS NULL
		`, now, accountRef); err != nil {
			return noAccount, err
		}
	}
	summary, _ := json.Marshal(struct {
		DisplayName string              `json:"display_name"`
		Role        edgeapp.AccountRole `json:"role"`
		Revision    int64               `json:"revision"`
	}{
		DisplayName: account.DisplayName,
		Role:        account.Role,
		Revision:    account.Revision,
	})
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt:  now,
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "account.update",
		ResourceRef: accountRef,
		Outcome:     auditOutcomeSuccess,
		Summary:     summary,
	}); err != nil {
		return noAccount, err
	}
	if err := tx.Commit(); err != nil {
		return noAccount, err
	}
	return account, nil
}

func (store *Store) DisableEdgeAccount(
	ctx context.Context,
	actor edgeapp.Actor,
	accountRef string,
	expectedRevision int64,
) (edgeapp.Account, error) {
	var noAccount edgeapp.Account
	if err := actor.Validate(); err != nil {
		return noAccount, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noAccount, err
	}
	defer func() { _ = tx.Rollback() }()

	account, err := scanEdgeAccount(tx.QueryRowContext(ctx, `
		SELECT account_ref, login_id, display_name, role, state,
			must_change_password, revision, created_at, updated_at
		FROM edge_accounts
		WHERE account_ref = ?
	`, accountRef))
	if err != nil {
		return noAccount, err
	}
	if account.Revision != expectedRevision {
		return noAccount, edgeapp.ErrRevisionMismatch
	}
	if account.State != edgeapp.AccountStateActive {
		return noAccount, edgeapp.ErrNotFound
	}
	if account.Role == edgeapp.AccountRoleSystemAdmin {
		var activeSystemAdmins int
		if err := tx.QueryRowContext(ctx, `
			SELECT count(*) FROM edge_accounts
			WHERE role = 'system_admin' AND state = 'active'
		`).Scan(&activeSystemAdmins); err != nil {
			return noAccount, err
		}
		if activeSystemAdmins <= 1 {
			return noAccount, edgeapp.ErrLastSystemAdmin
		}
	}

	now := time.Now().UnixMilli()
	account.State = edgeapp.AccountStateDisabled
	account.Revision++
	account.UpdatedAt = now
	if _, err := tx.ExecContext(ctx, `
		UPDATE edge_accounts
		SET state = 'disabled', disabled_at = ?, updated_at = ?, revision = ?
		WHERE account_ref = ? AND revision = ? AND state = 'active'
	`, now, now, account.Revision, accountRef, expectedRevision); err != nil {
		return noAccount, err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE edge_sessions SET revoked_at = ?
		WHERE account_ref = ? AND revoked_at IS NULL
	`, now, accountRef); err != nil {
		return noAccount, err
	}
	summary, _ := json.Marshal(struct {
		Revision int64 `json:"revision"`
	}{Revision: account.Revision})
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt:  now,
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "account.disable",
		ResourceRef: accountRef,
		Outcome:     auditOutcomeSuccess,
		Summary:     summary,
	}); err != nil {
		return noAccount, err
	}
	if err := tx.Commit(); err != nil {
		return noAccount, err
	}
	return account, nil
}

func (store *Store) ReplaceEdgeAccountPassword(
	ctx context.Context,
	actor edgeapp.Actor,
	accountRef string,
	passwordPHC string,
	mustChangePassword bool,
	expectedRevision int64,
) (edgeapp.Account, error) {
	var noAccount edgeapp.Account
	if err := actor.Validate(); err != nil {
		return noAccount, err
	}
	tx, err := store.db.BeginTx(ctx, nil)
	if err != nil {
		return noAccount, err
	}
	defer func() { _ = tx.Rollback() }()

	account, err := scanEdgeAccount(tx.QueryRowContext(ctx, `
		SELECT account_ref, login_id, display_name, role, state,
			must_change_password, revision, created_at, updated_at
		FROM edge_accounts
		WHERE account_ref = ?
	`, accountRef))
	if err != nil {
		return noAccount, err
	}
	if account.Revision != expectedRevision {
		return noAccount, edgeapp.ErrRevisionMismatch
	}
	now := time.Now().UnixMilli()
	account.MustChangePassword = mustChangePassword
	account.Revision++
	account.UpdatedAt = now
	if _, err := tx.ExecContext(ctx, `
		UPDATE edge_accounts
		SET password_phc = ?, must_change_password = ?, updated_at = ?, revision = ?
		WHERE account_ref = ? AND revision = ?
	`, passwordPHC, mustChangePassword, now, account.Revision,
		accountRef, expectedRevision); err != nil {
		return noAccount, err
	}
	if _, err := tx.ExecContext(ctx, `
		UPDATE edge_sessions SET revoked_at = ?
		WHERE account_ref = ? AND revoked_at IS NULL
	`, now, accountRef); err != nil {
		return noAccount, err
	}
	summary, _ := json.Marshal(struct {
		MustChangePassword bool  `json:"must_change_password"`
		Revision           int64 `json:"revision"`
	}{
		MustChangePassword: account.MustChangePassword,
		Revision:           account.Revision,
	})
	if err := insertAuditEventTx(ctx, tx, edgeapp.AuditEvent{
		OccurredAt:  now,
		ActorClass:  actor.Class,
		ActorRef:    actor.Ref,
		Operation:   "account.password_replace",
		ResourceRef: accountRef,
		Outcome:     auditOutcomeSuccess,
		Summary:     summary,
	}); err != nil {
		return noAccount, err
	}
	if err := tx.Commit(); err != nil {
		return noAccount, err
	}
	return account, nil
}

func scanEdgeAccount(row rowScanner) (edgeapp.Account, error) {
	var account edgeapp.Account
	if err := row.Scan(
		&account.AccountRef,
		&account.LoginID,
		&account.DisplayName,
		&account.Role,
		&account.State,
		&account.MustChangePassword,
		&account.Revision,
		&account.CreatedAt,
		&account.UpdatedAt,
	); errors.Is(err, sql.ErrNoRows) {
		return edgeapp.Account{}, edgeapp.ErrNotFound
	} else if err != nil {
		return edgeapp.Account{}, err
	}
	return account, nil
}

func nullableInt64(value sql.NullInt64) *int64 {
	if !value.Valid {
		return nil
	}
	return &value.Int64
}
