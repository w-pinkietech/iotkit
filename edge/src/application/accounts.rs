use crate::{
    application::authorization::{Action, AuthorizationError, authorize},
    auth::{
        password::{Password, PasswordCandidate, PasswordError, hash_password, verify_password},
        principal::{AccountRole, Principal},
    },
    storage::{Account, AccountProvision, AuditActor, Storage, StorageError},
};

#[derive(Clone)]
pub struct AccountService {
    storage: Storage,
}

impl AccountService {
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    pub async fn create_initial_system_admin(
        &self,
        login_id: &str,
        display_name: &str,
        password: Password,
        now: i64,
    ) -> Result<Account, AccountApplicationError> {
        self.storage
            .create_account(
                provision(
                    login_id,
                    display_name,
                    AccountRole::SystemAdmin,
                    password,
                    false,
                    true,
                )?,
                AuditActor::local_cli(),
                now,
            )
            .await
            .map_err(Into::into)
    }

    pub async fn recover_system_admin_password(
        &self,
        login_id: &str,
        password: Password,
        now: i64,
    ) -> Result<Account, AccountApplicationError> {
        let credential = self
            .storage
            .get_account_credential_by_login(login_id)
            .await?;
        if credential.account.role != AccountRole::SystemAdmin
            || credential.account.state != crate::auth::principal::AccountState::Active
        {
            return Err(AuthorizationError::Forbidden.into());
        }
        let replacement_hash = hash_password(&password)?;
        self.storage
            .replace_account_password(
                &credential.account.account_ref,
                credential.account.revision,
                replacement_hash,
                false,
                AuditActor::local_cli(),
                now,
            )
            .await
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_account(
        &self,
        principal: &Principal,
        login_id: &str,
        display_name: &str,
        role: AccountRole,
        temporary_password: Password,
        now: i64,
    ) -> Result<Account, AccountApplicationError> {
        authorize(principal, Action::ManageAccounts)?;
        self.storage
            .create_account(
                provision(
                    login_id,
                    display_name,
                    role,
                    temporary_password,
                    true,
                    false,
                )?,
                AuditActor::account(principal.account_ref()),
                now,
            )
            .await
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_account(
        &self,
        principal: &Principal,
        account_ref: &str,
        expected_revision: i64,
        display_name: &str,
        role: AccountRole,
        now: i64,
    ) -> Result<Account, AccountApplicationError> {
        authorize(principal, Action::ManageAccounts)?;
        self.storage
            .update_account(
                account_ref,
                expected_revision,
                display_name,
                role,
                AuditActor::account(principal.account_ref()),
                now,
            )
            .await
            .map_err(Into::into)
    }

    pub async fn disable_account(
        &self,
        principal: &Principal,
        account_ref: &str,
        expected_revision: i64,
        now: i64,
    ) -> Result<Account, AccountApplicationError> {
        authorize(principal, Action::ManageAccounts)?;
        self.storage
            .disable_account(
                account_ref,
                expected_revision,
                AuditActor::account(principal.account_ref()),
                now,
            )
            .await
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn reset_password(
        &self,
        principal: &Principal,
        account_ref: &str,
        expected_revision: i64,
        temporary_password: Password,
        now: i64,
    ) -> Result<Account, AccountApplicationError> {
        authorize(principal, Action::ManageAccounts)?;
        let password_hash = hash_password(&temporary_password)?;
        self.storage
            .replace_account_password(
                account_ref,
                expected_revision,
                password_hash,
                true,
                AuditActor::account(principal.account_ref()),
                now,
            )
            .await
            .map_err(Into::into)
    }

    pub async fn change_own_password(
        &self,
        principal: &Principal,
        current_password: PasswordCandidate,
        new_password: Password,
        now: i64,
    ) -> Result<Account, AccountApplicationError> {
        authorize(principal, Action::ChangeOwnPassword)?;
        let credential = self
            .storage
            .get_account_credential_by_login(principal.login_id())
            .await?;
        if credential.account.account_ref != principal.account_ref() {
            return Err(AccountApplicationError::InvalidCurrentPassword);
        }
        let verified = verify_password(&credential.password_hash, &current_password)
            .map_err(|_| AccountApplicationError::InvalidCurrentPassword)?;
        if !verified.matches {
            return Err(AccountApplicationError::InvalidCurrentPassword);
        }
        let replacement_hash = hash_password(&new_password)?;
        self.storage
            .replace_account_password(
                principal.account_ref(),
                credential.account.revision,
                replacement_hash,
                false,
                AuditActor::account(principal.account_ref()),
                now,
            )
            .await
            .map_err(Into::into)
    }
}

fn provision(
    login_id: &str,
    display_name: &str,
    role: AccountRole,
    password: Password,
    must_change_password: bool,
    require_unowned: bool,
) -> Result<AccountProvision, AccountApplicationError> {
    Ok(AccountProvision {
        login_id: login_id.into(),
        display_name: display_name.into(),
        role,
        password_hash: hash_password(&password)?,
        must_change_password,
        require_unowned,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum AccountApplicationError {
    #[error(transparent)]
    Authorization(#[from] AuthorizationError),
    #[error(transparent)]
    Password(#[from] PasswordError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("current password is invalid")]
    InvalidCurrentPassword,
}

#[cfg(test)]
#[path = "../../tests/unit/account_application_tests.rs"]
mod tests;
