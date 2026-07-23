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
mod tests {
    use super::*;

    fn principal(role: AccountRole, must_change_password: bool) -> Principal {
        Principal::authenticated_account(
            "acct_0123456789abcdef0123456789abcdef",
            "operator",
            "Plant Operator",
            role,
            AccountState::Active,
            must_change_password,
            "sess_0123456789abcdef0123456789abcdef",
        )
        .expect("principal")
    }

    #[test]
    fn role_matrix_and_temporary_password_gate_are_explicit() {
        assert!(authorize(&principal(AccountRole::Viewer, false), Action::Read).is_ok());
        assert_eq!(
            authorize(
                &principal(AccountRole::Viewer, false),
                Action::OperationalMutation
            ),
            Err(AuthorizationError::Forbidden)
        );
        assert!(
            authorize(
                &principal(AccountRole::Admin, false),
                Action::OperationalMutation
            )
            .is_ok()
        );
        assert_eq!(
            authorize(
                &principal(AccountRole::Admin, false),
                Action::ManageAccounts
            ),
            Err(AuthorizationError::Forbidden)
        );
        assert_eq!(
            authorize(
                &principal(AccountRole::Viewer, false),
                Action::ReadSensitiveSetup
            ),
            Err(AuthorizationError::Forbidden)
        );
        assert!(
            authorize(
                &principal(AccountRole::Admin, false),
                Action::ReadSensitiveSetup
            )
            .is_ok()
        );
        assert!(
            authorize(
                &principal(AccountRole::SystemAdmin, false),
                Action::ManageAccounts
            )
            .is_ok()
        );
        let temporary = principal(AccountRole::SystemAdmin, true);
        assert!(authorize(&temporary, Action::ReadOwnSession).is_ok());
        assert!(authorize(&temporary, Action::ChangeOwnPassword).is_ok());
        assert_eq!(
            authorize(&temporary, Action::Read),
            Err(AuthorizationError::PasswordChangeRequired)
        );
    }
}
