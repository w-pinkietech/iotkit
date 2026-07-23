use crate::auth::principal::{AccountRole, AccountState, Principal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    ReadOwnSession,
    ChangeOwnPassword,
    Read,
    ReadSensitiveSetup,
    OperationalMutation,
    ManageAccounts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthorizationError {
    #[error("account is not active")]
    Unauthenticated,
    #[error("password change is required")]
    PasswordChangeRequired,
    #[error("operation is forbidden")]
    Forbidden,
}

pub fn authorize(principal: &Principal, action: Action) -> Result<(), AuthorizationError> {
    if principal.state() != AccountState::Active {
        return Err(AuthorizationError::Unauthenticated);
    }
    if principal.must_change_password() {
        return match action {
            Action::ReadOwnSession | Action::ChangeOwnPassword => Ok(()),
            _ => Err(AuthorizationError::PasswordChangeRequired),
        };
    }
    match action {
        Action::ReadOwnSession | Action::ChangeOwnPassword | Action::Read => Ok(()),
        Action::ReadSensitiveSetup
            if matches!(
                principal.role(),
                AccountRole::Admin | AccountRole::SystemAdmin
            ) =>
        {
            Ok(())
        }
        Action::OperationalMutation
            if matches!(
                principal.role(),
                AccountRole::Admin | AccountRole::SystemAdmin
            ) =>
        {
            Ok(())
        }
        Action::ManageAccounts if principal.role() == AccountRole::SystemAdmin => Ok(()),
        _ => Err(AuthorizationError::Forbidden),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/authorization_tests.rs"]
mod tests;
