#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountRole {
    Viewer,
    Admin,
    SystemAdmin,
}

impl AccountRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Admin => "admin",
            Self::SystemAdmin => "system_admin",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountState {
    Active,
    Disabled,
}

impl AccountState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    account_ref: String,
    login_id: String,
    display_name: String,
    role: AccountRole,
    state: AccountState,
    must_change_password: bool,
    session_ref: String,
}

impl Principal {
    #[allow(
        dead_code,
        reason = "constructed only by the authenticated session boundary added in the next slice"
    )]
    pub(crate) fn authenticated_account(
        account_ref: impl Into<String>,
        login_id: impl Into<String>,
        display_name: impl Into<String>,
        role: AccountRole,
        state: AccountState,
        must_change_password: bool,
        session_ref: impl Into<String>,
    ) -> Result<Self, PrincipalError> {
        let principal = Self {
            account_ref: account_ref.into(),
            login_id: login_id.into(),
            display_name: display_name.into(),
            role,
            state,
            must_change_password,
            session_ref: session_ref.into(),
        };
        if principal.account_ref.is_empty()
            || principal.login_id.is_empty()
            || principal.display_name.trim().is_empty()
            || principal.session_ref.is_empty()
        {
            return Err(PrincipalError::Invalid);
        }
        Ok(principal)
    }

    #[must_use]
    pub fn role(&self) -> AccountRole {
        self.role
    }

    #[must_use]
    pub fn state(&self) -> AccountState {
        self.state
    }

    #[must_use]
    pub fn must_change_password(&self) -> bool {
        self.must_change_password
    }

    #[must_use]
    pub fn account_ref(&self) -> &str {
        &self.account_ref
    }

    #[must_use]
    pub fn login_id(&self) -> &str {
        &self.login_id
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    #[must_use]
    pub fn session_ref(&self) -> &str {
        &self.session_ref
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PrincipalError {
    #[error("account principal is invalid")]
    Invalid,
}
