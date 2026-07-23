use std::{collections::HashMap, path::PathBuf};

use iotkit_edge::{
    application::accounts::AccountService,
    auth::password::Password,
    composition::StorageWebApplication,
    storage::{Storage, StorageProfile},
    web::{ApiMutation, ApiQuery, ConsoleRequest, WebApplication},
};

#[tokio::test]
async fn production_web_adapter_owns_sessions_and_reads_operator_views() {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("edge.db"),
    })
    .await
    .unwrap();
    AccountService::new(storage.clone())
        .create_initial_system_admin(
            "owner",
            "System Owner",
            Password::new("long enough owner password").unwrap(),
            1_700_000_000_000,
        )
        .await
        .unwrap();

    let application = StorageWebApplication::new(storage);
    let login = application
        .login("owner", "long enough owner password")
        .await
        .unwrap();
    let principal = application.authenticate(&login.token).await.unwrap();
    assert_eq!(principal.login_id, "owner");
    assert!(application.validate_csrf(&login.token, &login.csrf).await);

    let storage_status = application
        .query(ApiQuery::Named {
            route: "/api/v1/system/storage".into(),
            params: HashMap::new(),
        })
        .await
        .unwrap();
    assert_eq!(storage_status["profile"], "embedded");

    let console = application
        .console(ConsoleRequest {
            path: "/system".into(),
            query: HashMap::new(),
            principal,
        })
        .await
        .unwrap();
    assert_eq!(console.storage.profile_label, "組み込みSQLite");
    assert!(!console.storage.diagnostic_messages.is_empty());

    let created = application
        .mutate(
            &login.principal,
            ApiMutation::Named {
                route: "/api/v1/accounts".into(),
                params: HashMap::new(),
            },
            serde_json::json!({
                "login_id": "viewer",
                "display_name": "Read Only",
                "role": "viewer",
                "temporary_password": "long enough viewer password",
            }),
        )
        .await
        .unwrap();
    assert_eq!(created.status, axum::http::StatusCode::CREATED);
    assert_eq!(created.body["role"], "viewer");

    application.logout(&login.token).await.unwrap();
    assert!(application.authenticate(&login.token).await.is_err());
}
