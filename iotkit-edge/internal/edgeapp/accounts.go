package edgeapp

import (
	"context"
	"errors"
	"strings"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeauth"
)

var ErrInvalidCurrentPassword = errors.New("current password is invalid")

type AccountRole string

const (
	AccountRoleViewer      AccountRole = "viewer"
	AccountRoleAdmin       AccountRole = "admin"
	AccountRoleSystemAdmin AccountRole = "system_admin"
)

func (role AccountRole) Valid() bool {
	return role == AccountRoleViewer || role == AccountRoleAdmin ||
		role == AccountRoleSystemAdmin
}

type AccountState string

const (
	AccountStateActive   AccountState = "active"
	AccountStateDisabled AccountState = "disabled"
)

type Account struct {
	AccountRef         string       `json:"account_ref"`
	LoginID            string       `json:"login_id"`
	DisplayName        string       `json:"display_name"`
	Role               AccountRole  `json:"role"`
	State              AccountState `json:"state"`
	MustChangePassword bool         `json:"must_change_password"`
	Revision           int64        `json:"revision"`
	CreatedAt          int64        `json:"created_at"`
	UpdatedAt          int64        `json:"updated_at"`
}

type AccountProvision struct {
	LoginID            string
	DisplayName        string
	Role               AccountRole
	PasswordPHC        string
	MustChangePassword bool
	RequireUnowned     bool
}

type AccountCredential struct {
	Account     Account
	PasswordPHC string
}

type AccountOperation interface {
	isAccountOperation()
}

type CreateInitialSystemAdmin struct {
	LoginID     string
	DisplayName string
	Password    string
}

func (CreateInitialSystemAdmin) isAccountOperation() {}

type CreateAccount struct {
	LoginID           string
	DisplayName       string
	Role              AccountRole
	TemporaryPassword string
}

func (CreateAccount) isAccountOperation() {}

type DisableAccount struct {
	AccountRef       string
	ExpectedRevision int64
}

func (DisableAccount) isAccountOperation() {}

type UpdateAccount struct {
	AccountRef       string
	DisplayName      string
	Role             AccountRole
	ExpectedRevision int64
}

func (UpdateAccount) isAccountOperation() {}

type ResetAccountPassword struct {
	AccountRef        string
	TemporaryPassword string
	ExpectedRevision  int64
}

func (ResetAccountPassword) isAccountOperation() {}

type RecoverSystemAdminPassword struct {
	LoginID  string
	Password string
}

func (RecoverSystemAdminPassword) isAccountOperation() {}

type ChangeOwnPassword struct {
	CurrentPassword string
	NewPassword     string
}

func (ChangeOwnPassword) isAccountOperation() {}

type AccountResult struct {
	Account *Account
}

type AccountRepository interface {
	CreateEdgeAccount(context.Context, Actor, AccountProvision) (Account, error)
	GetEdgeAccount(context.Context, string) (Account, error)
	GetEdgeAccountByLoginID(context.Context, string) (Account, error)
	GetEdgeAccountCredential(context.Context, string) (AccountCredential, error)
	ListEdgeAccounts(context.Context) ([]Account, error)
	UpdateEdgeAccount(
		context.Context,
		Actor,
		string,
		string,
		AccountRole,
		int64,
	) (Account, error)
	DisableEdgeAccount(context.Context, Actor, string, int64) (Account, error)
	ReplaceEdgeAccountPassword(
		context.Context,
		Actor,
		string,
		string,
		bool,
		int64,
	) (Account, error)
}

type AccountService struct {
	repository AccountRepository
}

func NewAccountService(repository AccountRepository) *AccountService {
	return &AccountService{repository: repository}
}

func (service *AccountService) DispatchAccount(
	ctx context.Context,
	actor Actor,
	operation AccountOperation,
) (AccountResult, error) {
	var noResult AccountResult
	if service == nil || service.repository == nil {
		return noResult, errors.New("Edge account repository is nil")
	}
	if err := actor.Validate(); err != nil {
		return noResult, err
	}

	switch operation := operation.(type) {
	case CreateInitialSystemAdmin:
		if actor.Class != ActorLocalCLI {
			return noResult, ErrForbidden
		}
		provision, err := makeProvision(
			operation.LoginID,
			operation.DisplayName,
			AccountRoleSystemAdmin,
			operation.Password,
			false,
		)
		if err != nil {
			return noResult, err
		}
		provision.RequireUnowned = true
		account, err := service.repository.CreateEdgeAccount(ctx, actor, provision)
		if err != nil {
			return noResult, err
		}
		return AccountResult{Account: &account}, nil
	case CreateAccount:
		if actor.Class != ActorAccount || actor.Role != AccountRoleSystemAdmin {
			return noResult, ErrForbidden
		}
		provision, err := makeProvision(
			operation.LoginID,
			operation.DisplayName,
			operation.Role,
			operation.TemporaryPassword,
			true,
		)
		if err != nil {
			return noResult, err
		}
		account, err := service.repository.CreateEdgeAccount(ctx, actor, provision)
		if err != nil {
			return noResult, err
		}
		return AccountResult{Account: &account}, nil
	case DisableAccount:
		if actor.Class != ActorAccount || actor.Role != AccountRoleSystemAdmin {
			return noResult, ErrForbidden
		}
		if err := validateAccountMutationRef(operation.AccountRef, operation.ExpectedRevision); err != nil {
			return noResult, err
		}
		account, err := service.repository.DisableEdgeAccount(
			ctx,
			actor,
			operation.AccountRef,
			operation.ExpectedRevision,
		)
		if err != nil {
			return noResult, err
		}
		return AccountResult{Account: &account}, nil
	case UpdateAccount:
		if actor.Class != ActorAccount || actor.Role != AccountRoleSystemAdmin {
			return noResult, ErrForbidden
		}
		if err := validateAccountMutationRef(
			operation.AccountRef,
			operation.ExpectedRevision,
		); err != nil {
			return noResult, err
		}
		if err := validateProfileText("display name", operation.DisplayName, 128); err != nil {
			return noResult, err
		}
		if !operation.Role.Valid() {
			return noResult, errors.New("unsupported Edge account role")
		}
		account, err := service.repository.UpdateEdgeAccount(
			ctx,
			actor,
			operation.AccountRef,
			strings.TrimSpace(operation.DisplayName),
			operation.Role,
			operation.ExpectedRevision,
		)
		if err != nil {
			return noResult, err
		}
		return AccountResult{Account: &account}, nil
	case ResetAccountPassword:
		if actor.Class != ActorAccount || actor.Role != AccountRoleSystemAdmin {
			return noResult, ErrForbidden
		}
		if err := validateAccountMutationRef(operation.AccountRef, operation.ExpectedRevision); err != nil {
			return noResult, err
		}
		passwordPHC, err := edgeauth.HashPassword(operation.TemporaryPassword)
		if err != nil {
			return noResult, err
		}
		account, err := service.repository.ReplaceEdgeAccountPassword(
			ctx,
			actor,
			operation.AccountRef,
			passwordPHC,
			true,
			operation.ExpectedRevision,
		)
		if err != nil {
			return noResult, err
		}
		return AccountResult{Account: &account}, nil
	case RecoverSystemAdminPassword:
		if actor.Class != ActorLocalCLI {
			return noResult, ErrForbidden
		}
		normalizedLoginID, err := edgeauth.NormalizeLoginID(operation.LoginID)
		if err != nil {
			return noResult, err
		}
		account, err := service.repository.GetEdgeAccountByLoginID(ctx, normalizedLoginID)
		if err != nil {
			return noResult, err
		}
		if account.Role != AccountRoleSystemAdmin ||
			account.State != AccountStateActive {
			return noResult, ErrForbidden
		}
		passwordPHC, err := edgeauth.HashPassword(operation.Password)
		if err != nil {
			return noResult, err
		}
		account, err = service.repository.ReplaceEdgeAccountPassword(
			ctx,
			actor,
			account.AccountRef,
			passwordPHC,
			false,
			account.Revision,
		)
		if err != nil {
			return noResult, err
		}
		return AccountResult{Account: &account}, nil
	case ChangeOwnPassword:
		if actor.Class != ActorAccount {
			return noResult, ErrForbidden
		}
		credential, err := service.repository.GetEdgeAccountCredential(ctx, actor.Ref)
		if err != nil {
			return noResult, err
		}
		ok, _, err := edgeauth.VerifyPassword(
			credential.PasswordPHC,
			operation.CurrentPassword,
		)
		if err != nil || !ok {
			return noResult, ErrInvalidCurrentPassword
		}
		passwordPHC, err := edgeauth.HashPassword(operation.NewPassword)
		if err != nil {
			return noResult, err
		}
		account, err := service.repository.ReplaceEdgeAccountPassword(
			ctx,
			actor,
			actor.Ref,
			passwordPHC,
			false,
			credential.Account.Revision,
		)
		if err != nil {
			return noResult, err
		}
		return AccountResult{Account: &account}, nil
	default:
		return noResult, errors.New("unsupported Edge account operation")
	}
}

func (service *AccountService) ListAccounts(
	ctx context.Context,
	actor Actor,
) ([]Account, error) {
	if service == nil || service.repository == nil {
		return nil, errors.New("Edge account repository is nil")
	}
	if err := actor.Validate(); err != nil {
		return nil, err
	}
	if actor.Class != ActorAccount || actor.Role != AccountRoleSystemAdmin {
		return nil, ErrForbidden
	}
	return service.repository.ListEdgeAccounts(ctx)
}

func makeProvision(
	loginID string,
	displayName string,
	role AccountRole,
	password string,
	mustChangePassword bool,
) (AccountProvision, error) {
	normalizedLoginID, err := edgeauth.NormalizeLoginID(loginID)
	if err != nil {
		return AccountProvision{}, err
	}
	if err := validateProfileText("display name", displayName, 128); err != nil {
		return AccountProvision{}, err
	}
	if !role.Valid() {
		return AccountProvision{}, errors.New("unsupported Edge account role")
	}
	passwordPHC, err := edgeauth.HashPassword(password)
	if err != nil {
		return AccountProvision{}, err
	}
	return AccountProvision{
		LoginID:            normalizedLoginID,
		DisplayName:        strings.TrimSpace(displayName),
		Role:               role,
		PasswordPHC:        passwordPHC,
		MustChangePassword: mustChangePassword,
	}, nil
}

func validateAccountMutationRef(accountRef string, expectedRevision int64) error {
	if err := validateResourceRef(accountRef, "acct_"); err != nil {
		return err
	}
	if expectedRevision < 1 {
		return errors.New("expected account revision must be positive")
	}
	return nil
}
