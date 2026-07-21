package edgesession

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"strings"
	"sync"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeauth"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
)

const (
	idleSessionLifetime     = 8 * time.Hour
	absoluteSessionLifetime = 24 * time.Hour
	failureWindowLifetime   = time.Minute
	maxFailuresPerWindow    = 5
	maxPasswordQueue        = 18
	maxPasswordVerifiers    = 2
)

var (
	ErrInvalidCredentials = errors.New("login ID or password is invalid")
	ErrRateLimited        = errors.New("too many login attempts")
	ErrUnauthenticated    = errors.New("Edge session is not authenticated")
	ErrBusy               = errors.New("Edge authentication is busy")
)

type Principal struct {
	AccountRef         string
	LoginID            string
	DisplayName        string
	Role               edgeapp.AccountRole
	MustChangePassword bool
	SessionRef         string
}

type Session struct {
	SessionRef string
	Token      string
	CSRFToken  string
	Principal  Principal
}

type Options struct {
	Now   func() time.Time
	Delay func(context.Context, time.Duration) error
}

type failureWindow struct {
	start time.Time
	count int
}

type Manager struct {
	store      *store.Store
	now        func() time.Time
	delay      func(context.Context, time.Duration) error
	dummyPHC   string
	admission  chan struct{}
	verifiers  chan struct{}
	failuresMu sync.Mutex
	failures   map[string]failureWindow
}

func NewManager(archive *store.Store, options Options) (*Manager, error) {
	if archive == nil {
		return nil, errors.New("Edge session store is nil")
	}
	now := options.Now
	if now == nil {
		now = time.Now
	}
	delay := options.Delay
	if delay == nil {
		delay = waitContext
	}
	dummyPHC, err := edgeauth.HashPassword("dummy-authentication-password")
	if err != nil {
		return nil, err
	}
	return &Manager{
		store:     archive,
		now:       now,
		delay:     delay,
		dummyPHC:  dummyPHC,
		admission: make(chan struct{}, maxPasswordQueue),
		verifiers: make(chan struct{}, maxPasswordVerifiers),
		failures:  make(map[string]failureWindow),
	}, nil
}

func (manager *Manager) Login(
	ctx context.Context,
	source string,
	loginID string,
	password string,
) (Session, error) {
	var noSession Session
	normalized, normalizationErr := edgeauth.NormalizeLoginID(loginID)
	if normalizationErr != nil {
		normalized = strings.ToLower(strings.TrimSpace(loginID))
	}
	sourceKey := "source:" + boundedFailureIdentity(source)
	loginKey := "login:" + boundedFailureIdentity(normalized)
	now := manager.now()
	if manager.failureCount(sourceKey, now) >= maxFailuresPerWindow ||
		manager.failureCount(loginKey, now) >= maxFailuresPerWindow {
		return noSession, ErrRateLimited
	}

	select {
	case manager.admission <- struct{}{}:
		defer func() { <-manager.admission }()
	default:
		return noSession, ErrBusy
	}
	select {
	case manager.verifiers <- struct{}{}:
		defer func() { <-manager.verifiers }()
	case <-ctx.Done():
		return noSession, ctx.Err()
	}

	account, lookupErr := manager.store.GetAccountByLoginID(ctx, normalized)
	encoded := manager.dummyPHC
	if lookupErr == nil {
		encoded = account.PasswordPHC
	}
	ok, _, verifyErr := edgeauth.VerifyPassword(encoded, password)
	valid := normalizationErr == nil && lookupErr == nil && verifyErr == nil && ok &&
		account.State == store.AccountStateActive
	if !valid {
		failures := manager.recordFailure(sourceKey, now)
		if loginFailures := manager.recordFailure(loginKey, now); loginFailures > failures {
			failures = loginFailures
		}
		delay := time.Duration(failures) * 100 * time.Millisecond
		if delay > 2*time.Second {
			delay = 2 * time.Second
		}
		if err := manager.delay(ctx, delay); err != nil {
			return noSession, err
		}
		_ = manager.store.RecordAuditEvent(ctx, edgeapp.AuditEvent{
			OccurredAt:  now.UnixMilli(),
			ActorClass:  edgeapp.ActorSystem,
			ActorRef:    "authentication",
			Operation:   "session.login_failed",
			ResourceRef: loginKey,
			Outcome:     "failure",
			Summary:     []byte(`{"reason":"invalid_credentials"}`),
		})
		return noSession, ErrInvalidCredentials
	}

	manager.clearFailure(sourceKey)
	manager.clearFailure(loginKey)
	token, err := randomSecret(32)
	if err != nil {
		return noSession, err
	}
	csrf, err := randomSecret(32)
	if err != nil {
		return noSession, err
	}
	sessionRef, err := randomReference("sess_")
	if err != nil {
		return noSession, err
	}
	tokenHash := sha256.Sum256([]byte(token))
	csrfHash := sha256.Sum256([]byte(csrf))
	issuedAt := now.UnixMilli()
	record := store.SessionRecord{
		SessionRef:        sessionRef,
		TokenSHA256:       tokenHash[:],
		CSRFSHA256:        csrfHash[:],
		AccountRef:        account.AccountRef,
		IssuedAt:          issuedAt,
		LastSeenAt:        issuedAt,
		IdleExpiresAt:     now.Add(idleSessionLifetime).UnixMilli(),
		AbsoluteExpiresAt: now.Add(absoluteSessionLifetime).UnixMilli(),
	}
	if err := manager.store.CreateSessionWithAudit(ctx, record, edgeapp.AuditEvent{
		OccurredAt:  issuedAt,
		ActorClass:  edgeapp.ActorAccount,
		ActorRef:    account.AccountRef,
		Operation:   "session.login",
		ResourceRef: sessionRef,
		Outcome:     "success",
		Summary:     []byte(`{}`),
	}); err != nil {
		return noSession, err
	}
	return Session{
		SessionRef: sessionRef,
		Token:      token,
		CSRFToken:  csrf,
		Principal:  principalFromAccount(account, sessionRef),
	}, nil
}

func (manager *Manager) Authenticate(ctx context.Context, token string) (Principal, error) {
	if token == "" {
		return Principal{}, ErrUnauthenticated
	}
	tokenHash := sha256.Sum256([]byte(token))
	now := manager.now()
	active, err := manager.store.GetActiveSessionByTokenHash(ctx, tokenHash[:], now.UnixMilli())
	if err != nil {
		if errors.Is(err, store.ErrSessionNotFound) {
			return Principal{}, ErrUnauthenticated
		}
		return Principal{}, err
	}
	idleExpiresAt := now.Add(idleSessionLifetime).UnixMilli()
	if idleExpiresAt > active.AbsoluteExpiresAt {
		idleExpiresAt = active.AbsoluteExpiresAt
	}
	if err := manager.store.TouchSession(
		ctx,
		active.SessionRef,
		now.UnixMilli(),
		idleExpiresAt,
	); err != nil {
		if errors.Is(err, store.ErrSessionNotFound) {
			return Principal{}, ErrUnauthenticated
		}
		return Principal{}, err
	}
	return principalFromAccount(active.Account, active.SessionRef), nil
}

func (manager *Manager) Logout(ctx context.Context, token string) error {
	if token == "" {
		return ErrUnauthenticated
	}
	tokenHash := sha256.Sum256([]byte(token))
	now := manager.now().UnixMilli()
	active, err := manager.store.GetActiveSessionByTokenHash(ctx, tokenHash[:], now)
	if err != nil {
		if errors.Is(err, store.ErrSessionNotFound) {
			return ErrUnauthenticated
		}
		return err
	}
	if err := manager.store.RevokeSessionWithAudit(
		ctx,
		active.SessionRef,
		now,
		edgeapp.AuditEvent{
			OccurredAt:  now,
			ActorClass:  edgeapp.ActorAccount,
			ActorRef:    active.Account.AccountRef,
			Operation:   "session.logout",
			ResourceRef: active.SessionRef,
			Outcome:     "success",
			Summary:     []byte(`{}`),
		},
	); err != nil {
		if errors.Is(err, store.ErrSessionNotFound) {
			return ErrUnauthenticated
		}
		return err
	}
	return nil
}

func (manager *Manager) ValidateCSRF(active store.ActiveSession, token string) bool {
	actual := sha256.Sum256([]byte(token))
	return len(active.CSRFSHA256) == sha256.Size &&
		subtle.ConstantTimeCompare(actual[:], active.CSRFSHA256) == 1
}

func (manager *Manager) ValidateSessionCSRF(
	ctx context.Context,
	sessionToken string,
	csrfToken string,
) bool {
	if sessionToken == "" || csrfToken == "" {
		return false
	}
	tokenHash := sha256.Sum256([]byte(sessionToken))
	active, err := manager.store.GetActiveSessionByTokenHash(
		ctx,
		tokenHash[:],
		manager.now().UnixMilli(),
	)
	if err != nil {
		return false
	}
	return manager.ValidateCSRF(active, csrfToken)
}

func principalFromAccount(account store.AccountRecord, sessionRef string) Principal {
	return Principal{
		AccountRef:         account.AccountRef,
		LoginID:            account.LoginID,
		DisplayName:        account.DisplayName,
		Role:               edgeapp.AccountRole(account.Role),
		MustChangePassword: account.MustChangePassword,
		SessionRef:         sessionRef,
	}
}

func randomSecret(size int) (string, error) {
	value := make([]byte, size)
	if _, err := rand.Read(value); err != nil {
		return "", errors.New("generate Edge session secret")
	}
	return base64.RawURLEncoding.EncodeToString(value), nil
}

func randomReference(prefix string) (string, error) {
	value := make([]byte, 16)
	if _, err := rand.Read(value); err != nil {
		return "", errors.New("generate Edge session reference")
	}
	return prefix + hex.EncodeToString(value), nil
}

func boundedFailureIdentity(value string) string {
	sum := sha256.Sum256([]byte(value))
	return hex.EncodeToString(sum[:])
}

func (manager *Manager) failureCount(key string, now time.Time) int {
	manager.failuresMu.Lock()
	defer manager.failuresMu.Unlock()
	window, found := manager.failures[key]
	if !found || now.Sub(window.start) >= failureWindowLifetime {
		delete(manager.failures, key)
		return 0
	}
	return window.count
}

func (manager *Manager) recordFailure(key string, now time.Time) int {
	manager.failuresMu.Lock()
	defer manager.failuresMu.Unlock()
	window, found := manager.failures[key]
	if !found || now.Sub(window.start) >= failureWindowLifetime {
		window = failureWindow{start: now}
	}
	window.count++
	manager.failures[key] = window
	return window.count
}

func (manager *Manager) clearFailure(key string) {
	manager.failuresMu.Lock()
	defer manager.failuresMu.Unlock()
	delete(manager.failures, key)
}

func waitContext(ctx context.Context, duration time.Duration) error {
	timer := time.NewTimer(duration)
	defer timer.Stop()
	select {
	case <-timer.C:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}
