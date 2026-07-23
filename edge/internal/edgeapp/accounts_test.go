package edgeapp

import (
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeauth"
)

func TestDispatchAccountCreateRequiresSystemAdministrator(t *testing.T) {
	repository := &fakeAccountRepository{}
	service := NewAccountService(repository)
	operation := CreateAccount{
		LoginID:           "operator.one",
		DisplayName:       "第一工場 担当者",
		Role:              AccountRoleViewer,
		TemporaryPassword: "現場で使う 十分に長い仮パスワード",
	}

	if _, err := service.DispatchAccount(
		context.Background(),
		AccountActor("acct_00000000000000000000000000000001", AccountRoleAdmin),
		operation,
	); !errors.Is(err, ErrForbidden) {
		t.Fatalf("admin create error = %v, want ErrForbidden", err)
	}
	if repository.createCalls != 0 {
		t.Fatalf("repository create calls = %d, want 0", repository.createCalls)
	}

	result, err := service.DispatchAccount(
		context.Background(),
		AccountActor("acct_00000000000000000000000000000002", AccountRoleSystemAdmin),
		operation,
	)
	if err != nil {
		t.Fatal(err)
	}
	if result.Account == nil || result.Account.LoginID != "operator.one" ||
		!result.Account.MustChangePassword {
		t.Fatalf("created account = %#v", result.Account)
	}
	if repository.createCalls != 1 {
		t.Fatalf("repository create calls = %d, want 1", repository.createCalls)
	}
	if strings.Contains(repository.provision.PasswordPHC, operation.TemporaryPassword) ||
		!strings.HasPrefix(repository.provision.PasswordPHC, "$argon2id$") {
		t.Fatalf("password provision was not safely hashed: %q", repository.provision.PasswordPHC)
	}
}

func TestDispatchAccountCreateAllowsInitialLocalCLIOnly(t *testing.T) {
	repository := &fakeAccountRepository{activeAccounts: 0}
	service := NewAccountService(repository)
	operation := CreateInitialSystemAdmin{
		LoginID:     "edge.owner",
		DisplayName: "サイト管理者",
		Password:    "初期所有者の 十分に長いパスワード",
	}

	result, err := service.DispatchAccount(context.Background(), LocalCLIActor(), operation)
	if err != nil {
		t.Fatal(err)
	}
	if result.Account == nil || result.Account.Role != AccountRoleSystemAdmin ||
		result.Account.MustChangePassword {
		t.Fatalf("initial account = %#v", result.Account)
	}

	repository.activeAccounts = 1
	if _, err := service.DispatchAccount(
		context.Background(),
		LocalCLIActor(),
		operation,
	); !errors.Is(err, ErrAlreadyOwned) {
		t.Fatalf("second initial account error = %v, want ErrAlreadyOwned", err)
	}
}

func TestDispatchAccountDisableProtectsLastSystemAdministrator(t *testing.T) {
	repository := &fakeAccountRepository{
		account: Account{
			AccountRef:  "acct_00000000000000000000000000000001",
			LoginID:     "edge.owner",
			DisplayName: "サイト管理者",
			Role:        AccountRoleSystemAdmin,
			State:       AccountStateActive,
			Revision:    1,
		},
		activeSystemAdmins: 1,
	}
	service := NewAccountService(repository)
	_, err := service.DispatchAccount(
		context.Background(),
		AccountActor("acct_00000000000000000000000000000002", AccountRoleSystemAdmin),
		DisableAccount{AccountRef: repository.account.AccountRef, ExpectedRevision: 1},
	)
	if !errors.Is(err, ErrLastSystemAdmin) {
		t.Fatalf("disable error = %v, want ErrLastSystemAdmin", err)
	}
	if repository.disableCalls != 0 {
		t.Fatalf("repository disable calls = %d, want 0", repository.disableCalls)
	}
}

func TestPasswordReplacementRevokesSessionsAndSetsFirstChangeFlag(t *testing.T) {
	repository := &fakeAccountRepository{
		account: Account{
			AccountRef:  "acct_00000000000000000000000000000001",
			LoginID:     "operator",
			DisplayName: "担当者",
			Role:        AccountRoleViewer,
			State:       AccountStateActive,
			Revision:    2,
		},
		activeSystemAdmins: 2,
	}
	service := NewAccountService(repository)
	result, err := service.DispatchAccount(
		context.Background(),
		AccountActor("acct_00000000000000000000000000000002", AccountRoleSystemAdmin),
		ResetAccountPassword{
			AccountRef:        repository.account.AccountRef,
			TemporaryPassword: "再発行する 十分に長い仮パスワード",
			ExpectedRevision:  2,
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	if result.Account == nil || !result.Account.MustChangePassword {
		t.Fatalf("reset account = %#v", result.Account)
	}
	if repository.resetCalls != 1 || !repository.mustChangePassword {
		t.Fatalf("reset calls = %d, mustChange = %v",
			repository.resetCalls, repository.mustChangePassword)
	}
}

func TestAccountUpdateAndListRequireSystemAdministrator(t *testing.T) {
	repository := &fakeAccountRepository{
		account: Account{
			AccountRef:  "acct_00000000000000000000000000000001",
			LoginID:     "operator",
			DisplayName: "担当者",
			Role:        AccountRoleViewer,
			State:       AccountStateActive,
			Revision:    1,
		},
		activeSystemAdmins: 2,
	}
	service := NewAccountService(repository)
	admin := AccountActor("acct_00000000000000000000000000000002", AccountRoleAdmin)
	if _, err := service.ListAccounts(context.Background(), admin); !errors.Is(err, ErrForbidden) {
		t.Fatalf("admin list error = %v, want ErrForbidden", err)
	}
	systemAdmin := AccountActor(
		"acct_00000000000000000000000000000003",
		AccountRoleSystemAdmin,
	)
	result, err := service.DispatchAccount(
		context.Background(),
		systemAdmin,
		UpdateAccount{
			AccountRef:       repository.account.AccountRef,
			DisplayName:      "設備担当者",
			Role:             AccountRoleAdmin,
			ExpectedRevision: 1,
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	if result.Account == nil || result.Account.Role != AccountRoleAdmin ||
		result.Account.DisplayName != "設備担当者" {
		t.Fatalf("updated account = %#v", result.Account)
	}
	accounts, err := service.ListAccounts(context.Background(), systemAdmin)
	if err != nil || len(accounts) != 1 {
		t.Fatalf("accounts = %#v, err = %v", accounts, err)
	}
}

func TestChangeOwnPasswordVerifiesCurrentPasswordAndClearsTemporaryFlag(t *testing.T) {
	const currentPassword = "現在使っている 十分に長いパスワード"
	passwordPHC, err := edgeauth.HashPassword(currentPassword)
	if err != nil {
		t.Fatal(err)
	}
	repository := &fakeAccountRepository{
		account: Account{
			AccountRef:         "acct_00000000000000000000000000000001",
			LoginID:            "operator",
			DisplayName:        "担当者",
			Role:               AccountRoleViewer,
			State:              AccountStateActive,
			MustChangePassword: true,
			Revision:           3,
		},
		passwordPHC: passwordPHC,
	}
	service := NewAccountService(repository)
	actor := AccountActor(repository.account.AccountRef, AccountRoleViewer)
	if _, err := service.DispatchAccount(
		context.Background(),
		actor,
		ChangeOwnPassword{
			CurrentPassword: "間違っている 十分に長いパスワード",
			NewPassword:     "新しく使う 十分に長いパスワード",
		},
	); !errors.Is(err, ErrInvalidCurrentPassword) {
		t.Fatalf("wrong current password error = %v, want ErrInvalidCurrentPassword", err)
	}
	result, err := service.DispatchAccount(
		context.Background(),
		actor,
		ChangeOwnPassword{
			CurrentPassword: currentPassword,
			NewPassword:     "新しく使う 十分に長いパスワード",
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	if result.Account == nil || result.Account.MustChangePassword {
		t.Fatalf("changed account = %#v", result.Account)
	}
}

type fakeAccountRepository struct {
	account            Account
	activeAccounts     int
	activeSystemAdmins int
	createCalls        int
	disableCalls       int
	resetCalls         int
	updateCalls        int
	provision          AccountProvision
	mustChangePassword bool
	passwordPHC        string
}

func (repository *fakeAccountRepository) CountActiveAccounts(context.Context) (int, error) {
	return repository.activeAccounts, nil
}

func (repository *fakeAccountRepository) CountActiveSystemAdmins(context.Context) (int, error) {
	return repository.activeSystemAdmins, nil
}

func (repository *fakeAccountRepository) CreateEdgeAccount(
	_ context.Context,
	_ Actor,
	provision AccountProvision,
) (Account, error) {
	if provision.RequireUnowned && repository.activeAccounts != 0 {
		return Account{}, ErrAlreadyOwned
	}
	repository.createCalls++
	repository.provision = provision
	account := Account{
		AccountRef:         "acct_00000000000000000000000000000003",
		LoginID:            provision.LoginID,
		DisplayName:        provision.DisplayName,
		Role:               provision.Role,
		State:              AccountStateActive,
		MustChangePassword: provision.MustChangePassword,
		Revision:           1,
	}
	repository.account = account
	return account, nil
}

func (repository *fakeAccountRepository) GetEdgeAccount(
	context.Context,
	string,
) (Account, error) {
	return repository.account, nil
}

func (repository *fakeAccountRepository) GetEdgeAccountByLoginID(
	context.Context,
	string,
) (Account, error) {
	return repository.account, nil
}

func (repository *fakeAccountRepository) GetEdgeAccountCredential(
	context.Context,
	string,
) (AccountCredential, error) {
	return AccountCredential{
		Account:     repository.account,
		PasswordPHC: repository.passwordPHC,
	}, nil
}

func (repository *fakeAccountRepository) ListEdgeAccounts(context.Context) ([]Account, error) {
	return []Account{repository.account}, nil
}

func (repository *fakeAccountRepository) UpdateEdgeAccount(
	_ context.Context,
	_ Actor,
	_ string,
	displayName string,
	role AccountRole,
	_ int64,
) (Account, error) {
	repository.updateCalls++
	account := repository.account
	account.DisplayName = displayName
	account.Role = role
	account.Revision++
	repository.account = account
	return account, nil
}

func (repository *fakeAccountRepository) DisableEdgeAccount(
	_ context.Context,
	_ Actor,
	_ string,
	_ int64,
) (Account, error) {
	if repository.account.Role == AccountRoleSystemAdmin &&
		repository.activeSystemAdmins <= 1 {
		return Account{}, ErrLastSystemAdmin
	}
	repository.disableCalls++
	account := repository.account
	account.State = AccountStateDisabled
	account.Revision++
	return account, nil
}

func (repository *fakeAccountRepository) ReplaceEdgeAccountPassword(
	_ context.Context,
	_ Actor,
	_ string,
	_ string,
	mustChangePassword bool,
	_ int64,
) (Account, error) {
	repository.resetCalls++
	repository.mustChangePassword = mustChangePassword
	account := repository.account
	account.MustChangePassword = mustChangePassword
	account.Revision++
	return account, nil
}
