use std::{collections::HashMap, path::PathBuf};

use iotkit_edge::{
    application::accounts::AccountService,
    application::semantics::{SemanticRuleDraft, Semantics},
    auth::password::Password,
    composition::{LoginPolicy, StorageWebApplication},
    semantics::{Detector, RuleSpec, SemanticKind, TriggerMode},
    storage::{Storage, StorageProfile},
    web::{ApiMutation, ApiQuery, ConsoleRequest, WebApplication},
};
use iotkit_edge_custody_contract::DescriptorSnapshot;

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
                method: axum::http::Method::POST,
                route: "/api/v1/accounts".into(),
                params: HashMap::new(),
                expected_revision: None,
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

#[tokio::test]
async fn login_rate_limit_is_non_enumerating_and_recovers_after_its_window() {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("rate-limit.db"),
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
    let application = StorageWebApplication::with_login_policy(
        storage.clone(),
        LoginPolicy {
            max_failures: 1,
            failure_window: std::time::Duration::from_secs(10),
            max_concurrent: 1,
            max_tracked_accounts: 16,
        },
    );

    let mut limited_responses = Vec::new();
    for login_id in ["owner", "missing"] {
        let error = application
            .login(login_id, "wrong password")
            .await
            .unwrap_err();
        assert_eq!(error.status, axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(error.code, "invalid_credentials");
        let limited = application
            .login(login_id, "wrong password")
            .await
            .unwrap_err();
        assert_eq!(limited.status, axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(limited.code, "login_rate_limited");
        assert_eq!(
            limited.message, "login cannot be attempted again yet",
            "rate-limit response must not reveal account existence"
        );
        limited_responses.push((limited.code, limited.message));
    }
    assert_eq!(limited_responses[0], limited_responses[1]);
    let variant = application
        .login(" OWNER ", "wrong password")
        .await
        .expect_err("login identifier variants must share one failure bucket");
    assert_eq!(variant.status, axum::http::StatusCode::TOO_MANY_REQUESTS);

    let recovering = StorageWebApplication::with_login_policy(
        storage,
        LoginPolicy {
            max_failures: 1,
            failure_window: std::time::Duration::from_millis(20),
            max_concurrent: 1,
            max_tracked_accounts: 16,
        },
    );
    assert_eq!(
        recovering
            .login("missing", "wrong password")
            .await
            .unwrap_err()
            .status,
        axum::http::StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        recovering
            .login("missing", "wrong password")
            .await
            .unwrap_err()
            .status,
        axum::http::StatusCode::TOO_MANY_REQUESTS
    );
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let recovered = recovering
        .login("missing", "wrong password")
        .await
        .expect_err("unknown account remains invalid after recovery");
    assert_eq!(recovered.status, axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mutation_dispatch_preserves_put_and_delete_semantics() {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("methods.db"),
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
    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .unwrap();
    storage.apply_descriptor(&descriptor, 1).await.unwrap();
    let rule = Semantics::new(storage.clone())
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: descriptor.edge_node_id,
                series_key: descriptor.signals[0].series_key.clone(),
                display_name: "Temperature".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            1,
        )
        .await
        .unwrap();
    let application = StorageWebApplication::new(storage.clone());
    let principal = application
        .login("owner", "long enough owner password")
        .await
        .unwrap()
        .principal;

    let invalid_put = application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::PUT,
                route: format!("/api/v1/semantic-rules/{}", rule.rule_id),
                params: HashMap::from([("rule_id".into(), rule.rule_id.clone())]),
                expected_revision: Some(1),
            },
            serde_json::json!({}),
        )
        .await
        .expect_err("an empty PUT must not be interpreted as DELETE");
    assert_eq!(invalid_put.status, axum::http::StatusCode::BAD_REQUEST);
    assert!(
        storage
            .list_semantic_rules()
            .await
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.rule_id == rule.rule_id)
            .unwrap()
            .active
    );

    let revised = application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::PUT,
                route: format!("/api/v1/semantic-rules/{}", rule.rule_id),
                params: HashMap::from([("rule_id".into(), rule.rule_id.clone())]),
                expected_revision: Some(1),
            },
            serde_json::json!({"display_name":"Temperature revised","kind":"numeric"}),
        )
        .await
        .expect("matching revision updates the rule");
    assert_eq!(revised.body["revision"], 2);
    let stale = application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::PUT,
                route: format!("/api/v1/semantic-rules/{}", rule.rule_id),
                params: HashMap::from([("rule_id".into(), rule.rule_id.clone())]),
                expected_revision: Some(1),
            },
            serde_json::json!({"display_name":"Stale update","kind":"numeric"}),
        )
        .await
        .expect_err("stale revision must fail");
    assert_eq!(stale.status, axum::http::StatusCode::PRECONDITION_FAILED);

    application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::DELETE,
                route: format!("/api/v1/semantic-rules/{}", rule.rule_id),
                params: HashMap::from([("rule_id".into(), rule.rule_id.clone())]),
                expected_revision: Some(2),
            },
            serde_json::json!({}),
        )
        .await
        .expect("DELETE retires the rule");
    assert!(
        !storage
            .list_semantic_rules()
            .await
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.rule_id == rule.rule_id)
            .unwrap()
            .active
    );
}

#[tokio::test]
#[ignore = "requires IOTKIT_TEST_POSTGRES_DSN"]
async fn postgres_enforces_the_same_web_revision_precondition() {
    let dsn = std::env::var("IOTKIT_TEST_POSTGRES_DSN").expect("PostgreSQL DSN");
    let storage = Storage::connect(StorageProfile::Postgres { dsn })
        .await
        .unwrap();
    storage
        .initialize_edge_identity(1_700_000_000_000)
        .await
        .unwrap();
    let unique = uuid::Uuid::new_v4().simple().to_string();
    let rule = Semantics::new(storage.clone())
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: format!("edge-node-{unique}"),
                series_key: format!("temperature-{unique}"),
                display_name: format!("Temperature {unique}"),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            1,
        )
        .await
        .unwrap();
    let principal = iotkit_edge::web::Principal {
        account_ref: "acct-postgres-test".into(),
        login_id: "postgres-test".into(),
        display_name: "PostgreSQL Test".into(),
        role: "system_admin".into(),
        state: "active".into(),
        must_change_password: false,
        revision: 1,
        created_at: 1,
        updated_at: 1,
    };
    let application = StorageWebApplication::new(storage);
    let revised = application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::PUT,
                route: format!("/api/v1/semantic-rules/{}", rule.rule_id),
                params: HashMap::from([("rule_id".into(), rule.rule_id.clone())]),
                expected_revision: Some(1),
            },
            serde_json::json!({"display_name":"PostgreSQL revised","kind":"numeric"}),
        )
        .await
        .unwrap();
    assert_eq!(revised.body["revision"], 2);
    let stale = application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: axum::http::Method::PUT,
                route: format!("/api/v1/semantic-rules/{}", rule.rule_id),
                params: HashMap::from([("rule_id".into(), rule.rule_id)]),
                expected_revision: Some(1),
            },
            serde_json::json!({"display_name":"Stale","kind":"numeric"}),
        )
        .await
        .unwrap_err();
    assert_eq!(stale.status, axum::http::StatusCode::PRECONDITION_FAILED);
}

#[tokio::test]
async fn production_web_diagnostics_use_the_runtime_threshold_and_certificate_file() {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::connect(StorageProfile::Sqlite {
        path: PathBuf::from(directory.path()).join("diagnostics.db"),
    })
    .await
    .unwrap();
    let certificate = directory.path().join("missing-broker.pem");
    let application = StorageWebApplication::with_runtime_settings(storage, 73, Some(certificate));
    let status = application
        .query(ApiQuery::Named {
            route: "/api/v1/system/storage".into(),
            params: HashMap::new(),
        })
        .await
        .unwrap();
    assert_eq!(status["warning_percent"], 73);
    let report = application
        .query(ApiQuery::Named {
            route: "/api/v1/system/diagnostics".into(),
            params: HashMap::new(),
        })
        .await
        .unwrap();
    assert_eq!(report["broker_certificate"]["available"], false);
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["code"] == "broker_certificate_unavailable")
    );
}
