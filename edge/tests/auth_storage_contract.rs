use std::path::PathBuf;

use iotkit_edge::{
    auth::{
        password::{Password, hash_password},
        principal::{AccountRole, AccountState},
        session::{SessionSecrets, SessionWindow},
    },
    storage::{AccountProvision, AuditActor, Storage, StorageError, StorageProfile, StoredSession},
};
use sqlx::sqlite::SqlitePoolOptions;
use tempfile::TempDir;

async fn sqlite_store() -> (TempDir, PathBuf, Storage) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-tmp");
    std::fs::create_dir_all(&root).expect("create workspace test temp");
    let directory = TempDir::new_in(root).expect("temp directory");
    let database = directory.path().join("edge.db");
    let store = Storage::connect(StorageProfile::Sqlite {
        path: database.clone(),
    })
    .await
    .expect("open SQLite storage");
    (directory, database, store)
}

async fn postgres_store() -> Storage {
    let dsn = std::env::var("IOTKIT_TEST_POSTGRES_DSN")
        .expect("IOTKIT_TEST_POSTGRES_DSN must be set for the PostgreSQL contract");
    Storage::connect(StorageProfile::Postgres { dsn })
        .await
        .expect("open PostgreSQL storage")
}

fn provision(login_id: &str, role: AccountRole, require_unowned: bool) -> AccountProvision {
    let password = Password::new("correct horse battery staple").expect("valid password");
    AccountProvision {
        login_id: login_id.into(),
        display_name: format!("{login_id} display"),
        role,
        password_hash: hash_password(&password).expect("hash password"),
        must_change_password: !require_unowned,
        require_unowned,
    }
}

async fn initial_admin(store: &Storage) -> iotkit_edge::storage::Account {
    store
        .create_account(
            provision("owner", AccountRole::SystemAdmin, true),
            AuditActor::local_cli(),
            1_700_000_000_000,
        )
        .await
        .expect("create initial owner")
}

async fn create_session(
    store: &Storage,
    account_ref: &str,
    now: i64,
) -> (SessionSecrets, StoredSession) {
    let account = store.get_account(account_ref).await.expect("load account");
    let secrets = SessionSecrets::generate().expect("session secrets");
    let session = store
        .create_session(
            account_ref,
            account.revision,
            secrets.session_ref().as_str(),
            secrets.token_digest(),
            secrets.csrf_digest(),
            SessionWindow::issued(now).expect("session window"),
            now,
        )
        .await
        .expect("create session");
    (secrets, session)
}

#[tokio::test]
async fn account_and_session_successes_are_audited_atomically() {
    let (_directory, _database, store) = sqlite_store().await;
    let owner = initial_admin(&store).await;
    assert_eq!(owner.revision, 1);
    assert_eq!(owner.state, AccountState::Active);

    let (secrets, session) = create_session(&store, &owner.account_ref, 1_700_000_001_000).await;
    assert_eq!(session.account.account_ref, owner.account_ref);
    let active = store
        .active_session_by_token(&secrets.token_digest(), 1_700_000_001_001)
        .await
        .expect("active session");
    assert_eq!(active.session_ref, session.session_ref);

    let audits = store.list_audit_events(10).await.expect("list audit");
    assert_eq!(audits[0].operation, "session.login");
    assert_eq!(audits[0].actor_login_id.as_deref(), Some("owner"));
    assert_eq!(audits[1].operation, "account.create");
    assert_eq!(
        audits[1].summary,
        serde_json::json!({
            "login_id": "owner",
            "display_name": "owner display",
            "role": "system_admin",
            "must_change_password": false
        })
    );
}

#[tokio::test]
async fn display_names_use_the_bounded_safe_utf8_contract() {
    let (_directory, _database, store) = sqlite_store().await;
    let owner = initial_admin(&store).await;
    let control = store
        .update_account(
            &owner.account_ref,
            owner.revision,
            "Owner\nInjected",
            AccountRole::SystemAdmin,
            AuditActor::account(&owner.account_ref),
            1_700_000_005_000,
        )
        .await
        .expect_err("control character must fail");
    assert!(matches!(control, StorageError::InvalidAccount(_)));

    let oversized = "界".repeat(43);
    assert!(oversized.len() > 128);
    let multibyte = store
        .update_account(
            &owner.account_ref,
            owner.revision,
            &oversized,
            AccountRole::SystemAdmin,
            AuditActor::account(&owner.account_ref),
            1_700_000_005_001,
        )
        .await
        .expect_err("UTF-8 byte bound must fail");
    assert!(matches!(multibyte, StorageError::InvalidAccount(_)));
}

#[tokio::test]
async fn session_touch_revoke_expiry_and_disabled_account_are_enforced() {
    let (_directory, _database, store) = sqlite_store().await;
    let owner = initial_admin(&store).await;
    let (first_secrets, first) =
        create_session(&store, &owner.account_ref, 1_700_000_010_000).await;
    store
        .touch_session(&first.session_ref, 1_700_000_020_000, 1_700_028_820_000)
        .await
        .expect("touch session");
    store
        .revoke_session(
            &first.session_ref,
            AuditActor::account(&owner.account_ref),
            1_700_000_030_000,
        )
        .await
        .expect("revoke session");
    assert!(matches!(
        store
            .active_session_by_token(&first_secrets.token_digest(), 1_700_000_030_001)
            .await,
        Err(StorageError::SessionNotFound)
    ));

    let (expired_secrets, _) = create_session(&store, &owner.account_ref, 1_700_000_040_000).await;
    assert!(matches!(
        store
            .active_session_by_token(&expired_secrets.token_digest(), 1_700_028_840_000)
            .await,
        Err(StorageError::SessionNotFound)
    ));

    let second_admin = store
        .create_account(
            provision("second_owner", AccountRole::SystemAdmin, false),
            AuditActor::account(&owner.account_ref),
            1_700_000_050_000,
        )
        .await
        .expect("create second system admin");
    let (disabled_secrets, _) = create_session(&store, &owner.account_ref, 1_700_000_051_000).await;
    store
        .disable_account(
            &owner.account_ref,
            owner.revision,
            AuditActor::account(&second_admin.account_ref),
            1_700_000_052_000,
        )
        .await
        .expect("disable account");
    assert!(matches!(
        store
            .active_session_by_token(&disabled_secrets.token_digest(), 1_700_000_052_001)
            .await,
        Err(StorageError::SessionNotFound)
    ));
}

#[tokio::test]
async fn display_only_update_keeps_sessions_but_role_and_password_changes_revoke_all() {
    let (_directory, _database, store) = sqlite_store().await;
    let owner = initial_admin(&store).await;
    let member = store
        .create_account(
            provision("member", AccountRole::Admin, false),
            AuditActor::account(&owner.account_ref),
            1_700_000_100_000,
        )
        .await
        .expect("create member");
    let (display_session, _) = create_session(&store, &member.account_ref, 1_700_000_101_000).await;

    let displayed = store
        .update_account(
            &member.account_ref,
            member.revision,
            "Renamed Member",
            AccountRole::Admin,
            AuditActor::account(&owner.account_ref),
            1_700_000_102_000,
        )
        .await
        .expect("display update");
    store
        .active_session_by_token(&display_session.token_digest(), 1_700_000_102_001)
        .await
        .expect("display-only update keeps session");

    let role_session = display_session;
    let viewer = store
        .update_account(
            &member.account_ref,
            displayed.revision,
            "Renamed Member",
            AccountRole::Viewer,
            AuditActor::account(&owner.account_ref),
            1_700_000_103_000,
        )
        .await
        .expect("role update");
    assert!(matches!(
        store
            .active_session_by_token(&role_session.token_digest(), 1_700_000_103_001)
            .await,
        Err(StorageError::SessionNotFound)
    ));

    let (password_session, _) =
        create_session(&store, &member.account_ref, 1_700_000_104_000).await;
    let replacement =
        hash_password(&Password::new("a completely new password").expect("replacement password"))
            .expect("replacement hash");
    store
        .replace_account_password(
            &member.account_ref,
            viewer.revision,
            replacement,
            false,
            AuditActor::account(&owner.account_ref),
            1_700_000_105_000,
        )
        .await
        .expect("replace password");
    assert!(matches!(
        store
            .active_session_by_token(&password_session.token_digest(), 1_700_000_105_001)
            .await,
        Err(StorageError::SessionNotFound)
    ));
}

#[tokio::test]
async fn verified_old_password_revision_cannot_create_a_session_after_password_change() {
    let (_directory, _database, store) = sqlite_store().await;
    let owner = initial_admin(&store).await;
    let stale_revision = owner.revision;
    let replacement =
        hash_password(&Password::new("a completely new password").expect("replacement password"))
            .expect("replacement hash");
    store
        .replace_account_password(
            &owner.account_ref,
            stale_revision,
            replacement,
            false,
            AuditActor::account(&owner.account_ref),
            1_700_000_150_000,
        )
        .await
        .expect("replace password");

    let secrets = SessionSecrets::generate().expect("session secrets");
    let error = store
        .create_session(
            &owner.account_ref,
            stale_revision,
            secrets.session_ref().as_str(),
            secrets.token_digest(),
            secrets.csrf_digest(),
            SessionWindow::issued(1_700_000_151_000).expect("session window"),
            1_700_000_151_000,
        )
        .await
        .expect_err("stale password verification must not create a session");
    assert!(matches!(error, StorageError::RevisionMismatch));
}

#[tokio::test]
async fn revisions_and_last_active_system_admin_are_hard_preconditions() {
    let (_directory, _database, store) = sqlite_store().await;
    let owner = initial_admin(&store).await;

    let stale = store
        .update_account(
            &owner.account_ref,
            owner.revision + 1,
            "Owner",
            AccountRole::SystemAdmin,
            AuditActor::account(&owner.account_ref),
            1_700_000_200_000,
        )
        .await
        .expect_err("stale revision");
    assert!(matches!(stale, StorageError::RevisionMismatch));

    let demote = store
        .update_account(
            &owner.account_ref,
            owner.revision,
            "Owner",
            AccountRole::Admin,
            AuditActor::account(&owner.account_ref),
            1_700_000_200_001,
        )
        .await
        .expect_err("last system admin cannot be demoted");
    assert!(matches!(demote, StorageError::LastSystemAdmin));

    let disable = store
        .disable_account(
            &owner.account_ref,
            owner.revision,
            AuditActor::account(&owner.account_ref),
            1_700_000_200_002,
        )
        .await
        .expect_err("last system admin cannot be disabled");
    assert!(matches!(disable, StorageError::LastSystemAdmin));
}

#[tokio::test]
async fn concurrent_system_admin_demotions_cannot_remove_every_admin() {
    let (_directory, _database, store) = sqlite_store().await;
    let first = initial_admin(&store).await;
    let second = store
        .create_account(
            provision("second_owner", AccountRole::SystemAdmin, false),
            AuditActor::account(&first.account_ref),
            1_700_000_250_000,
        )
        .await
        .expect("create second system admin");

    let first_store = store.clone();
    let first_ref = first.account_ref.clone();
    let second_actor = second.account_ref.clone();
    let first_update = tokio::spawn(async move {
        first_store
            .update_account(
                &first_ref,
                first.revision,
                "First",
                AccountRole::Admin,
                AuditActor::account(second_actor),
                1_700_000_251_000,
            )
            .await
    });
    let second_store = store.clone();
    let second_ref = second.account_ref.clone();
    let first_actor = first.account_ref.clone();
    let second_update = tokio::spawn(async move {
        second_store
            .update_account(
                &second_ref,
                second.revision,
                "Second",
                AccountRole::Admin,
                AuditActor::account(first_actor),
                1_700_000_251_001,
            )
            .await
    });
    let results = [
        first_update.await.expect("first task"),
        second_update.await.expect("second task"),
    ];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(StorageError::LastSystemAdmin)))
            .count(),
        1
    );
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN"]
async fn postgres_obeys_account_session_and_admin_safety_contract() {
    let store = postgres_store().await;
    let owner = initial_admin(&store).await;
    let member = store
        .create_account(
            provision("postgres_member", AccountRole::Admin, false),
            AuditActor::account(&owner.account_ref),
            1_700_001_000_000,
        )
        .await
        .expect("create member");
    let (secrets, _) = create_session(&store, &member.account_ref, 1_700_001_001_000).await;

    let updated = store
        .update_account(
            &member.account_ref,
            member.revision,
            "PostgreSQL Member",
            AccountRole::Viewer,
            AuditActor::account(&owner.account_ref),
            1_700_001_002_000,
        )
        .await
        .expect("change role");
    assert_eq!(updated.role, AccountRole::Viewer);
    assert!(matches!(
        store
            .active_session_by_token(&secrets.token_digest(), 1_700_001_002_001)
            .await,
        Err(StorageError::SessionNotFound)
    ));

    let last_admin = store
        .disable_account(
            &owner.account_ref,
            owner.revision,
            AuditActor::account(&owner.account_ref),
            1_700_001_003_000,
        )
        .await
        .expect_err("last system admin remains protected");
    assert!(matches!(last_admin, StorageError::LastSystemAdmin));

    let audits = store.list_audit_events(20).await.expect("list audits");
    assert!(
        audits
            .iter()
            .any(|event| event.operation == "account.update")
    );
    assert!(
        audits
            .iter()
            .any(|event| event.operation == "session.login")
    );
}

#[tokio::test]
async fn audit_insert_failure_rolls_back_the_account_mutation() {
    let (_directory, database, store) = sqlite_store().await;
    let owner = initial_admin(&store).await;
    let external = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}", database.display()))
        .await
        .expect("open fault connection");
    sqlx::query(
        "CREATE TRIGGER reject_auth_audit BEFORE INSERT ON audit_events \
         WHEN NEW.operation = 'account.update' BEGIN SELECT RAISE(ABORT, 'audit fault'); END",
    )
    .execute(&external)
    .await
    .expect("install audit fault");
    external.close().await;

    assert!(
        store
            .update_account(
                &owner.account_ref,
                owner.revision,
                "Must Roll Back",
                AccountRole::SystemAdmin,
                AuditActor::account(&owner.account_ref),
                1_700_000_300_000,
            )
            .await
            .is_err()
    );
    let unchanged = store
        .get_account(&owner.account_ref)
        .await
        .expect("read account");
    assert_eq!(unchanged.display_name, owner.display_name);
    assert_eq!(unchanged.revision, owner.revision);
}
