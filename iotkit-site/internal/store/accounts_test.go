package store

import (
	"bytes"
	"context"
	"errors"
	"testing"
)

func TestOpenCreatesLocalAccountSchema(t *testing.T) {
	store := openTestStore(t)

	var version int
	if err := store.db.QueryRow("PRAGMA user_version").Scan(&version); err != nil {
		t.Fatal(err)
	}
	if version != 5 {
		t.Fatalf("schema version = %d, want 5", version)
	}
	for _, table := range []string{"site_accounts", "site_sessions"} {
		var got int
		if err := store.db.QueryRow("SELECT count(*) FROM " + table).Scan(&got); err != nil {
			t.Fatal(err)
		}
		if got != 0 {
			t.Fatalf("%s rows = %d, want 0", table, got)
		}
	}
}

func TestDisabledAccountLoginIDCannotBeReused(t *testing.T) {
	store := openTestStore(t)
	ctx := context.Background()
	account := AccountRecord{
		AccountRef:         "acct_00000000000000000000000000000001",
		LoginID:            "Operator.One",
		LoginIDNormalized:  "operator.one",
		DisplayName:        "第一工場 担当者",
		PasswordPHC:        "$argon2id$test-only",
		Role:               AccountRoleViewer,
		State:              AccountStateActive,
		MustChangePassword: true,
		CreatedAt:          1000,
		UpdatedAt:          1000,
	}
	if err := store.CreateAccount(ctx, account); err != nil {
		t.Fatal(err)
	}
	if err := store.DisableAccount(ctx, account.AccountRef, 2000); err != nil {
		t.Fatal(err)
	}

	reused := account
	reused.AccountRef = "acct_00000000000000000000000000000002"
	reused.LoginID = "operator.one"
	reused.CreatedAt = 3000
	reused.UpdatedAt = 3000
	if err := store.CreateAccount(ctx, reused); !errors.Is(err, ErrAccountLoginIDConflict) {
		t.Fatalf("CreateAccount error = %v, want ErrAccountLoginIDConflict", err)
	}

	got, err := store.GetAccountByLoginID(ctx, "operator.one")
	if err != nil {
		t.Fatal(err)
	}
	if got.AccountRef != account.AccountRef || got.State != AccountStateDisabled || got.DisabledAt == nil {
		t.Fatalf("disabled account = %#v", got)
	}
}

func TestRevokeAccountSessionsInvalidatesTokenLookup(t *testing.T) {
	store := openTestStore(t)
	ctx := context.Background()
	account := AccountRecord{
		AccountRef:        "acct_00000000000000000000000000000001",
		LoginID:           "operator",
		LoginIDNormalized: "operator",
		DisplayName:       "担当者",
		PasswordPHC:       "$argon2id$test-only",
		Role:              AccountRoleAdmin,
		State:             AccountStateActive,
		CreatedAt:         1000,
		UpdatedAt:         1000,
	}
	if err := store.CreateAccount(ctx, account); err != nil {
		t.Fatal(err)
	}
	tokenHash := bytes.Repeat([]byte{0x11}, 32)
	csrfHash := bytes.Repeat([]byte{0x22}, 32)
	session := SessionRecord{
		SessionRef:        "sess_00000000000000000000000000000001",
		TokenSHA256:       tokenHash,
		CSRFSHA256:        csrfHash,
		AccountRef:        account.AccountRef,
		IssuedAt:          2000,
		LastSeenAt:        2000,
		IdleExpiresAt:     3000,
		AbsoluteExpiresAt: 4000,
	}
	if err := store.CreateSession(ctx, session); err != nil {
		t.Fatal(err)
	}
	if got, err := store.GetActiveSessionByTokenHash(ctx, tokenHash, 2500); err != nil {
		t.Fatal(err)
	} else if got.SessionRef != session.SessionRef || got.Account.AccountRef != account.AccountRef {
		t.Fatalf("active session = %#v", got)
	}

	if err := store.RevokeAccountSessions(ctx, account.AccountRef, 2600); err != nil {
		t.Fatal(err)
	}
	if _, err := store.GetActiveSessionByTokenHash(ctx, tokenHash, 2601); !errors.Is(err, ErrSessionNotFound) {
		t.Fatalf("session lookup error = %v, want ErrSessionNotFound", err)
	}
}

func TestReplaceAccountPasswordRevokesExistingSessions(t *testing.T) {
	store := openTestStore(t)
	ctx := context.Background()
	account := AccountRecord{
		AccountRef:        "acct_00000000000000000000000000000001",
		LoginID:           "operator",
		LoginIDNormalized: "operator",
		DisplayName:       "担当者",
		PasswordPHC:       "$argon2id$old",
		Role:              AccountRoleAdmin,
		State:             AccountStateActive,
		CreatedAt:         1000,
		UpdatedAt:         1000,
	}
	if err := store.CreateAccount(ctx, account); err != nil {
		t.Fatal(err)
	}
	tokenHash := bytes.Repeat([]byte{0x33}, 32)
	if err := store.CreateSession(ctx, SessionRecord{
		SessionRef:        "sess_00000000000000000000000000000001",
		TokenSHA256:       tokenHash,
		CSRFSHA256:        bytes.Repeat([]byte{0x44}, 32),
		AccountRef:        account.AccountRef,
		IssuedAt:          2000,
		LastSeenAt:        2000,
		IdleExpiresAt:     3000,
		AbsoluteExpiresAt: 4000,
	}); err != nil {
		t.Fatal(err)
	}

	if err := store.ReplaceAccountPassword(
		ctx,
		account.AccountRef,
		"$argon2id$new",
		true,
		2500,
	); err != nil {
		t.Fatal(err)
	}
	got, err := store.GetAccountByLoginID(ctx, "operator")
	if err != nil {
		t.Fatal(err)
	}
	if got.PasswordPHC != "$argon2id$new" || !got.MustChangePassword || got.UpdatedAt != 2500 {
		t.Fatalf("updated account = %#v", got)
	}
	if _, err := store.GetActiveSessionByTokenHash(ctx, tokenHash, 2501); !errors.Is(err, ErrSessionNotFound) {
		t.Fatalf("session lookup error = %v, want ErrSessionNotFound", err)
	}
}

func TestTouchAndRevokeSingleSession(t *testing.T) {
	store := openTestStore(t)
	ctx := context.Background()
	account := AccountRecord{
		AccountRef:        "acct_00000000000000000000000000000001",
		LoginID:           "viewer",
		LoginIDNormalized: "viewer",
		DisplayName:       "閲覧者",
		PasswordPHC:       "$argon2id$test-only",
		Role:              AccountRoleViewer,
		State:             AccountStateActive,
		CreatedAt:         1000,
		UpdatedAt:         1000,
	}
	if err := store.CreateAccount(ctx, account); err != nil {
		t.Fatal(err)
	}
	tokenHash := bytes.Repeat([]byte{0x55}, 32)
	session := SessionRecord{
		SessionRef:        "sess_00000000000000000000000000000001",
		TokenSHA256:       tokenHash,
		CSRFSHA256:        bytes.Repeat([]byte{0x66}, 32),
		AccountRef:        account.AccountRef,
		IssuedAt:          2000,
		LastSeenAt:        2000,
		IdleExpiresAt:     3000,
		AbsoluteExpiresAt: 5000,
	}
	if err := store.CreateSession(ctx, session); err != nil {
		t.Fatal(err)
	}
	if err := store.TouchSession(ctx, session.SessionRef, 2500, 3500); err != nil {
		t.Fatal(err)
	}
	got, err := store.GetActiveSessionByTokenHash(ctx, tokenHash, 3200)
	if err != nil {
		t.Fatal(err)
	}
	if got.LastSeenAt != 2500 || got.IdleExpiresAt != 3500 {
		t.Fatalf("touched session = %#v", got.SessionRecord)
	}
	if err := store.RevokeSession(ctx, session.SessionRef, 3300); err != nil {
		t.Fatal(err)
	}
	if _, err := store.GetActiveSessionByTokenHash(ctx, tokenHash, 3301); !errors.Is(err, ErrSessionNotFound) {
		t.Fatalf("session lookup error = %v, want ErrSessionNotFound", err)
	}
}
