package store

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

func TestOpenCreatesLocalAccountSchema(t *testing.T) {
	store := openTestStore(t)

	var version int
	if err := store.db.QueryRow("PRAGMA user_version").Scan(&version); err != nil {
		t.Fatal(err)
	}
	if version != 28 {
		t.Fatalf("schema version = %d, want 28", version)
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

func TestCreateInitialSiteAccountIsAtomicAndAudited(t *testing.T) {
	store := openTestStore(t)
	ctx := context.Background()
	provision := siteapp.AccountProvision{
		LoginID:        "site.owner",
		DisplayName:    "サイト管理者",
		Role:           siteapp.AccountRoleSystemAdmin,
		PasswordPHC:    "$argon2id$test-only",
		RequireUnowned: true,
	}

	account, err := store.CreateSiteAccount(ctx, siteapp.LocalCLIActor(), provision)
	if err != nil {
		t.Fatal(err)
	}
	if account.AccountRef == "" || account.Revision != 1 ||
		account.Role != siteapp.AccountRoleSystemAdmin ||
		account.State != siteapp.AccountStateActive {
		t.Fatalf("created account = %#v", account)
	}
	if _, err := store.CreateSiteAccount(
		ctx,
		siteapp.LocalCLIActor(),
		provision,
	); !errors.Is(err, siteapp.ErrAlreadyOwned) {
		t.Fatalf("second initial account error = %v, want ErrAlreadyOwned", err)
	}
	events, err := store.ListAuditEvents(ctx, 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) != 1 || events[0].Operation != "account.create" ||
		events[0].ResourceRef != account.AccountRef {
		t.Fatalf("audit events = %#v", events)
	}
	if bytes.Contains(events[0].Summary, []byte("argon2")) {
		t.Fatalf("audit summary contains password hash: %s", events[0].Summary)
	}
}

func TestDisableSiteAccountProtectsLastSystemAdminAndRevokesSessions(t *testing.T) {
	store := openTestStore(t)
	ctx := context.Background()
	owner := createSiteAccountForTest(
		t,
		store,
		siteapp.AccountRoleSystemAdmin,
		"site.owner",
	)
	tokenHash := bytes.Repeat([]byte{0x71}, 32)
	if err := store.CreateSession(ctx, SessionRecord{
		SessionRef:        "sess_00000000000000000000000000000001",
		TokenSHA256:       tokenHash,
		CSRFSHA256:        bytes.Repeat([]byte{0x72}, 32),
		AccountRef:        owner.AccountRef,
		IssuedAt:          1000,
		LastSeenAt:        1000,
		IdleExpiresAt:     5000,
		AbsoluteExpiresAt: 9000,
	}); err != nil {
		t.Fatal(err)
	}

	if _, err := store.DisableSiteAccount(
		ctx,
		siteapp.AccountActor(owner.AccountRef, siteapp.AccountRoleSystemAdmin),
		owner.AccountRef,
		owner.Revision,
	); !errors.Is(err, siteapp.ErrLastSystemAdmin) {
		t.Fatalf("last system admin disable error = %v", err)
	}
	second := createSiteAccountForTest(
		t,
		store,
		siteapp.AccountRoleSystemAdmin,
		"site.backup",
	)
	disabled, err := store.DisableSiteAccount(
		ctx,
		siteapp.AccountActor(second.AccountRef, siteapp.AccountRoleSystemAdmin),
		owner.AccountRef,
		owner.Revision,
	)
	if err != nil {
		t.Fatal(err)
	}
	if disabled.State != siteapp.AccountStateDisabled || disabled.Revision != owner.Revision+1 {
		t.Fatalf("disabled account = %#v", disabled)
	}
	if _, err := store.GetActiveSessionByTokenHash(ctx, tokenHash, 2000); !errors.Is(err, ErrSessionNotFound) {
		t.Fatalf("disabled account session error = %v, want ErrSessionNotFound", err)
	}
}

func TestReplaceSiteAccountPasswordIsRevisionProtectedAndRevokesSessions(t *testing.T) {
	store := openTestStore(t)
	ctx := context.Background()
	owner := createSiteAccountForTest(
		t,
		store,
		siteapp.AccountRoleSystemAdmin,
		"site.owner",
	)
	if _, err := store.ReplaceSiteAccountPassword(
		ctx,
		siteapp.AccountActor(owner.AccountRef, siteapp.AccountRoleSystemAdmin),
		owner.AccountRef,
		"$argon2id$new",
		true,
		owner.Revision+1,
	); !errors.Is(err, siteapp.ErrRevisionMismatch) {
		t.Fatalf("password revision error = %v, want ErrRevisionMismatch", err)
	}
	updated, err := store.ReplaceSiteAccountPassword(
		ctx,
		siteapp.AccountActor(owner.AccountRef, siteapp.AccountRoleSystemAdmin),
		owner.AccountRef,
		"$argon2id$new",
		true,
		owner.Revision,
	)
	if err != nil {
		t.Fatal(err)
	}
	if !updated.MustChangePassword || updated.Revision != owner.Revision+1 {
		t.Fatalf("updated account = %#v", updated)
	}
	record, err := store.GetAccountByLoginID(ctx, owner.LoginID)
	if err != nil {
		t.Fatal(err)
	}
	if record.PasswordPHC != "$argon2id$new" {
		t.Fatalf("stored password hash = %q", record.PasswordPHC)
	}
}

func TestAccountAuditSnapshotsActorIdentityWithoutCredential(t *testing.T) {
	store := openTestStore(t)
	ctx := context.Background()
	actorAccount := createSiteAccountForTest(
		t,
		store,
		siteapp.AccountRoleSystemAdmin,
		"site.owner",
	)
	target := createSiteAccountForTest(
		t,
		store,
		siteapp.AccountRoleViewer,
		"operator",
	)
	if _, err := store.UpdateSiteAccount(
		ctx,
		siteapp.AccountActor(actorAccount.AccountRef, siteapp.AccountRoleSystemAdmin),
		target.AccountRef,
		"設備担当者",
		siteapp.AccountRoleAdmin,
		target.Revision,
	); err != nil {
		t.Fatal(err)
	}
	events, err := store.ListAuditEvents(ctx, 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) == 0 || events[0].ActorLoginID == nil ||
		*events[0].ActorLoginID != actorAccount.LoginID ||
		events[0].ActorDisplayName == nil ||
		*events[0].ActorDisplayName != actorAccount.DisplayName {
		t.Fatalf("account audit actor snapshot = %#v", events)
	}
	encoded, err := json.Marshal(events[0])
	if err != nil {
		t.Fatal(err)
	}
	for _, forbidden := range []string{"password_phc", "token_sha256", "csrf_sha256"} {
		if bytes.Contains(encoded, []byte(forbidden)) {
			t.Fatalf("audit JSON contains %q: %s", forbidden, encoded)
		}
	}
}

func createSiteAccountForTest(
	t *testing.T,
	store *Store,
	role siteapp.AccountRole,
	loginID string,
) siteapp.Account {
	t.Helper()
	account, err := store.CreateSiteAccount(
		context.Background(),
		siteapp.LocalCLIActor(),
		siteapp.AccountProvision{
			LoginID:        loginID,
			DisplayName:    loginID,
			Role:           role,
			PasswordPHC:    "$argon2id$test-only",
			RequireUnowned: false,
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	return account
}
