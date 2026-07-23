use std::path::PathBuf;

use tempfile::TempDir;

use super::*;
use crate::{
    auth::principal::{AccountState, Principal},
    storage::StorageProfile,
};

async fn service() -> (TempDir, AccountService) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).expect("create test temp root");
    let directory = TempDir::new_in(root).expect("temp directory");
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: directory.path().join("edge.db"),
    })
    .await
    .expect("open storage");
    (directory, AccountService::new(storage))
}

fn principal(
    account_ref: &str,
    login_id: &str,
    role: AccountRole,
    must_change_password: bool,
) -> Principal {
    Principal::authenticated_account(
        account_ref,
        login_id,
        login_id,
        role,
        AccountState::Active,
        must_change_password,
        "sess_0123456789abcdef0123456789abcdef",
    )
    .expect("valid principal")
}

#[tokio::test]
async fn roles_and_temporary_passwords_gate_account_management() {
    let (_directory, service) = service().await;
    let owner = service
        .create_initial_system_admin(
            "owner",
            "System Owner",
            Password::new("correct horse battery staple").expect("valid password"),
            1_700_001_000_000,
        )
        .await
        .expect("initial owner");
    let owner_principal = principal(
        &owner.account_ref,
        &owner.login_id,
        AccountRole::SystemAdmin,
        false,
    );
    let member = service
        .create_account(
            &owner_principal,
            "member",
            "Member",
            AccountRole::Admin,
            Password::new("temporary password value").expect("valid password"),
            1_700_001_001_000,
        )
        .await
        .expect("create member");
    assert!(member.must_change_password);

    let admin = principal(
        &member.account_ref,
        &member.login_id,
        AccountRole::Admin,
        false,
    );
    assert!(matches!(
        service
            .create_account(
                &admin,
                "blocked",
                "Blocked",
                AccountRole::Viewer,
                Password::new("temporary password value").expect("valid password"),
                1_700_001_002_000,
            )
            .await,
        Err(AccountApplicationError::Authorization(_))
    ));

    let temporary_owner = principal(
        &owner.account_ref,
        &owner.login_id,
        AccountRole::SystemAdmin,
        true,
    );
    assert!(matches!(
        service
            .disable_account(
                &temporary_owner,
                &member.account_ref,
                member.revision,
                1_700_001_003_000,
            )
            .await,
        Err(AccountApplicationError::Authorization(_))
    ));
}

#[tokio::test]
async fn own_password_change_checks_current_password_and_clears_gate() {
    let (_directory, service) = service().await;
    let owner = service
        .create_initial_system_admin(
            "owner",
            "System Owner",
            Password::new("correct horse battery staple").expect("valid password"),
            1_700_002_000_000,
        )
        .await
        .expect("initial owner");
    let owner_principal = principal(
        &owner.account_ref,
        &owner.login_id,
        AccountRole::SystemAdmin,
        false,
    );
    let member = service
        .create_account(
            &owner_principal,
            "member",
            "Member",
            AccountRole::Viewer,
            Password::new("temporary password value").expect("valid password"),
            1_700_002_001_000,
        )
        .await
        .expect("create member");
    let temporary = principal(
        &member.account_ref,
        &member.login_id,
        AccountRole::Viewer,
        true,
    );

    assert!(matches!(
        service
            .change_own_password(
                &temporary,
                PasswordCandidate::new("incorrect password value"),
                Password::new("new permanent password value").expect("valid password"),
                1_700_002_002_000,
            )
            .await,
        Err(AccountApplicationError::InvalidCurrentPassword)
    ));
    let changed = service
        .change_own_password(
            &temporary,
            PasswordCandidate::new("temporary password value"),
            Password::new("new permanent password value").expect("valid password"),
            1_700_002_003_000,
        )
        .await
        .expect("change own password");
    assert!(!changed.must_change_password);
    assert_eq!(changed.revision, member.revision + 1);
}
