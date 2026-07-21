package edgesession

import (
	"context"
	"crypto/sha256"
	"errors"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeauth"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
)

func TestLoginCreatesHashedSessionAndAuthenticatesPrincipal(t *testing.T) {
	archive := openSessionStore(t)
	password := "現場担当者の 十分に長いパスワード"
	account := createSessionAccount(t, archive, "operator", password, true)
	now := time.UnixMilli(1_800_000_000_000)
	manager, err := NewManager(archive, Options{
		Now:   func() time.Time { return now },
		Delay: noDelay,
	})
	if err != nil {
		t.Fatal(err)
	}

	session, err := manager.Login(context.Background(), "192.0.2.10", "Operator", password)
	if err != nil {
		t.Fatal(err)
	}
	if session.Token == "" || session.CSRFToken == "" || session.SessionRef == "" {
		t.Fatalf("session contains empty secret/reference: %#v", session)
	}
	if session.Principal.AccountRef != account.AccountRef ||
		session.Principal.Role != edgeapp.AccountRoleViewer ||
		!session.Principal.MustChangePassword {
		t.Fatalf("principal = %#v", session.Principal)
	}
	tokenHash := sha256.Sum256([]byte(session.Token))
	stored, err := archive.GetActiveSessionByTokenHash(
		context.Background(),
		tokenHash[:],
		now.UnixMilli()+1,
	)
	if err != nil {
		t.Fatal(err)
	}
	if stored.SessionRef != session.SessionRef {
		t.Fatalf("stored session ref = %q, want %q", stored.SessionRef, session.SessionRef)
	}
	if string(stored.TokenSHA256) == session.Token || string(stored.CSRFSHA256) == session.CSRFToken {
		t.Fatal("raw session secret was persisted")
	}

	principal, err := manager.Authenticate(context.Background(), session.Token)
	if err != nil {
		t.Fatal(err)
	}
	if principal.AccountRef != account.AccountRef {
		t.Fatalf("authenticated principal = %#v", principal)
	}
	if !manager.ValidateCSRF(stored, session.CSRFToken) {
		t.Fatal("valid CSRF token was rejected")
	}
	if manager.ValidateCSRF(stored, "wrong-token") {
		t.Fatal("invalid CSRF token was accepted")
	}
}

func TestLoginUsesGenericFailureAndRateLimitsBySourceAndLogin(t *testing.T) {
	archive := openSessionStore(t)
	createSessionAccount(t, archive, "operator", "正しい 十分に長いパスワード", false)
	manager, err := NewManager(archive, Options{
		Now:   func() time.Time { return time.UnixMilli(1_800_000_000_000) },
		Delay: noDelay,
	})
	if err != nil {
		t.Fatal(err)
	}

	for attempt := 1; attempt <= 5; attempt++ {
		_, err := manager.Login(
			context.Background(),
			"192.0.2.20",
			"operator",
			"間違った 十分に長いパスワード",
		)
		if !errors.Is(err, ErrInvalidCredentials) {
			t.Fatalf("attempt %d error = %v, want ErrInvalidCredentials", attempt, err)
		}
	}
	if _, err := manager.Login(
		context.Background(),
		"192.0.2.20",
		"operator",
		"正しい 十分に長いパスワード",
	); !errors.Is(err, ErrRateLimited) {
		t.Fatalf("rate-limited login error = %v, want ErrRateLimited", err)
	}
	if _, err := manager.Login(
		context.Background(),
		"192.0.2.21",
		"unknown",
		"間違った 十分に長いパスワード",
	); !errors.Is(err, ErrInvalidCredentials) {
		t.Fatalf("unknown account error = %v, want generic invalid credentials", err)
	}
}

func TestLogoutRevokesOnlyPresentedSession(t *testing.T) {
	archive := openSessionStore(t)
	password := "現場担当者の 十分に長いパスワード"
	createSessionAccount(t, archive, "operator", password, false)
	manager, err := NewManager(archive, Options{
		Now:   func() time.Time { return time.UnixMilli(1_800_000_000_000) },
		Delay: noDelay,
	})
	if err != nil {
		t.Fatal(err)
	}
	first, err := manager.Login(context.Background(), "192.0.2.30", "operator", password)
	if err != nil {
		t.Fatal(err)
	}
	second, err := manager.Login(context.Background(), "192.0.2.31", "operator", password)
	if err != nil {
		t.Fatal(err)
	}

	if err := manager.Logout(context.Background(), first.Token); err != nil {
		t.Fatal(err)
	}
	if _, err := manager.Authenticate(context.Background(), first.Token); !errors.Is(err, ErrUnauthenticated) {
		t.Fatalf("logged-out token error = %v, want ErrUnauthenticated", err)
	}
	if _, err := manager.Authenticate(context.Background(), second.Token); err != nil {
		t.Fatalf("second session was revoked: %v", err)
	}
}

func TestAuthenticationEventsAreAuditedWithoutSecrets(t *testing.T) {
	archive := openSessionStore(t)
	password := "現場担当者の 十分に長いパスワード"
	createSessionAccount(t, archive, "operator", password, false)
	manager, err := NewManager(archive, Options{
		Now:   func() time.Time { return time.UnixMilli(1_800_000_000_000) },
		Delay: noDelay,
	})
	if err != nil {
		t.Fatal(err)
	}
	session, err := manager.Login(
		context.Background(),
		"192.0.2.40",
		"operator",
		password,
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := manager.Login(
		context.Background(),
		"192.0.2.41",
		"operator",
		"間違った 十分に長いパスワード",
	); !errors.Is(err, ErrInvalidCredentials) {
		t.Fatalf("failed login error = %v", err)
	}
	if err := manager.Logout(context.Background(), session.Token); err != nil {
		t.Fatal(err)
	}

	events, err := archive.ListAuditEvents(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	operations := map[string]bool{}
	for _, event := range events {
		operations[event.Operation] = true
		encoded := string(event.Summary)
		if strings.Contains(encoded, password) ||
			strings.Contains(encoded, session.Token) ||
			strings.Contains(encoded, session.CSRFToken) {
			t.Fatalf("audit contains authentication secret: %#v", event)
		}
	}
	for _, operation := range []string{"session.login", "session.login_failed", "session.logout"} {
		if !operations[operation] {
			t.Fatalf("missing %s audit event: %#v", operation, events)
		}
	}
}

func openSessionStore(t *testing.T) *store.Store {
	t.Helper()
	archive, err := store.Open(filepath.Join(t.TempDir(), "edge.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = archive.Close() })
	return archive
}

func createSessionAccount(
	t *testing.T,
	archive *store.Store,
	loginID string,
	password string,
	mustChange bool,
) edgeapp.Account {
	t.Helper()
	passwordPHC, err := edgeauth.HashPassword(password)
	if err != nil {
		t.Fatal(err)
	}
	account, err := archive.CreateEdgeAccount(
		context.Background(),
		edgeapp.LocalCLIActor(),
		edgeapp.AccountProvision{
			LoginID:            loginID,
			DisplayName:        "現場担当者",
			Role:               edgeapp.AccountRoleViewer,
			PasswordPHC:        passwordPHC,
			MustChangePassword: mustChange,
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	return account
}

func noDelay(context.Context, time.Duration) error {
	return nil
}
